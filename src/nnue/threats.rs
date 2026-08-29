use std::arch::x86_64::*;

use arrayvec::ArrayVec;
use cozy_chess::{BitBoard, Board, Color, Move, Piece, Square};

use crate::{
    ext::{BitBoardExt, ColoredPiece, ExtBoard, ExtMove, MoveType},
    nnue::{
        network::{Aligned, HL, KINGS, Network, SimdOps},
        update::{ThreatDelta, ThreatDeltaUpdates, ThreatUpdate},
    },
    param::MAX_DEPTH_USIZE,
};

#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [Aligned<i16, HL>; 2],
    is_clean: [bool; 2],
    king_sq: [Square; 2],
    update: ThreatUpdate,
}

impl Accumulator {
    fn new() -> Self {
        Self {
            vals: [Aligned::<i16, HL>::zeroed(), Aligned::<i16, HL>::zeroed()],
            is_clean: [false; 2],
            update: ThreatUpdate::default(),
            king_sq: [Square::A1; 2],
        }
    }
}

fn get_line(sq: Square, dir: usize) -> BitBoard {
    const LINE: [[BitBoard; 4]; 64] = {
        let mut line = [[BitBoard::EMPTY; 4]; 64];
        // Offsets for: 0: Horizontal, 1: Anti-Diagonal, 2: Vertical, 3: Main Diagonal
        let offsets: [(i32, i32); 4] = [(1, 0), (-1, 1), (0, 1), (1, 1)];

        let mut sq = 0;
        while sq < 64 {
            let file = (sq % 8) as i32;
            let rank = (sq / 8) as i32;

            let mut i = 0;
            while i < 4 {
                let (df, dr) = offsets[i];
                let mut bitboard = 0u64;

                // Ray forward
                let mut f = file + df;
                let mut r = rank + dr;
                while f >= 0 && f < 8 && r >= 0 && r < 8 {
                    bitboard |= 1u64 << (r * 8 + f);
                    f += df;
                    r += dr;
                }

                // Ray backward
                let mut f = file - df;
                let mut r = rank - dr;
                while f >= 0 && f < 8 && r >= 0 && r < 8 {
                    bitboard |= 1u64 << (r * 8 + f);
                    f -= df;
                    r -= dr;
                }

                line[sq][i] = BitBoard(bitboard);
                i += 1;
            }

            sq += 1;
        }

        line
    };

    LINE[sq as usize][dir]
}

pub struct Threats {
    pub side: Box<[Accumulator]>,
    pub head: usize,
}

impl Threats {
    pub fn new() -> Self {
        // we don't have clone on [accumulator]
        let mut sides = vec![];
        for _ in 0..MAX_DEPTH_USIZE {
            sides.push(Accumulator::new());
        }

        Self {
            side: sides.into_boxed_slice(),
            head: 0,
        }
    }

    pub fn init(&mut self, board: &Board, network: &Box<Network>) {
        self.head = 0;
        for side in Color::ALL {
            self.side[self.head].king_sq[side as usize] = board.king(side);
            self.refresh(side, board, network);
        }
    }

    pub fn make_move(&mut self, board: &Board, new_board: &Board, m: Move) {
        self.head += 1;
        let acc = &mut self.side[self.head];
        acc.is_clean = [false; 2];
        acc.king_sq[0] = new_board.king(Color::White);
        acc.king_sq[1] = new_board.king(Color::Black);
        acc.update.clear();
        Self::get_threat_update(board, new_board, m, &mut acc.update);
    }

    fn record_sq_inout(
        board: &Board,
        sq: Square,
        mask: BitBoard,
        also_incoming: bool,
        out: &mut ThreatDeltaUpdates,
    ) {
        let piece = board.color_piece_on(sq).unwrap();
        if piece.piece == Piece::King {
            return;
        }

        let occ = board.occupied();

        let pawn = cozy_chess::get_pawn_attacks(sq, piece.color);
        let knight = cozy_chess::get_knight_moves(sq);
        let bishop = cozy_chess::get_bishop_moves(sq, occ);
        let rook = cozy_chess::get_rook_moves(sq, occ);
        let queen = bishop | rook;

        // remove outgoing
        let attacks = match piece.piece {
            Piece::Pawn => pawn,
            Piece::Knight => knight,
            Piece::Bishop => bishop,
            Piece::Rook => rook,
            Piece::Queen => queen,
            Piece::King => unreachable!(),
        };
        for target_sq in attacks & occ & mask & !(board.pieces(Piece::King)) {
            let piece2: ColoredPiece = board.color_piece_on(target_sq).unwrap();
            out.push(ThreatDelta::new(piece, sq, piece2, target_sq));
        }

        // remove incoming
        let incoming_pawns = (cozy_chess::get_pawn_attacks(sq, Color::Black)
            & board.colored_pieces(Color::White, Piece::Pawn))
            | (cozy_chess::get_pawn_attacks(sq, Color::White)
                & board.colored_pieces(Color::Black, Piece::Pawn));
        let incoming = incoming_pawns
            | (knight & board.pieces(Piece::Knight))
            | (bishop & board.pieces(Piece::Bishop))
            | (rook & board.pieces(Piece::Rook))
            | (queen & board.pieces(Piece::Queen));

        for in_sq in incoming & occ & mask {
            let in_piece = board.color_piece_on(in_sq).unwrap();
            out.push(ThreatDelta::new(in_piece, in_sq, piece, sq));
        }
    }

    /// Records new attacks created because `vacated_sq` was emptied.
    fn record_discovered(
        new_board: &Board,
        vacated_sq: Square,
        ignore_sq: BitBoard,
        out: &mut ThreatDeltaUpdates,
    ) {
        let occ = new_board.occupied();

        let types = [
            new_board.pieces(Piece::Rook) | new_board.pieces(Piece::Queen),
            new_board.pieces(Piece::Bishop) | new_board.pieces(Piece::Queen),
            new_board.pieces(Piece::Rook) | new_board.pieces(Piece::Queen),
            new_board.pieces(Piece::Bishop) | new_board.pieces(Piece::Queen),
        ];

        for dir in 0..4 {
            let line = get_line(vacated_sq, dir) & occ;
            let ty = types[dir];
            if let Some((a, b)) = line.pop2(vacated_sq as usize) {
                let pa = new_board.color_piece_on(a).unwrap();
                let pb = new_board.color_piece_on(b).unwrap();

                if pa.piece == Piece::King
                    || pb.piece == Piece::King
                    || ignore_sq.has(a)
                    || ignore_sq.has(b)
                {
                    continue;
                }

                if ty.has(a) {
                    out.push(ThreatDelta::new(pa, a, pb, b));
                }

                if ty.has(b) {
                    out.push(ThreatDelta::new(pb, b, pa, a));
                }
            }
        }
    }

    fn record_unblocked_discovered_ep(
        new_board: &Board,
        vacated_sq1: Square,
        vacated_sq2: Square,
        ignore: BitBoard,
        out: &mut ThreatDeltaUpdates,
    ) {
        let new_occ = new_board.occupied();
        let bishops_queens = new_board.pieces(Piece::Bishop) | new_board.pieces(Piece::Queen);
        let rooks_queens = new_board.pieces(Piece::Rook) | new_board.pieces(Piece::Queen);

        if !(bishops_queens).is_empty() {
            for vacated_sq in [vacated_sq1, vacated_sq2] {
                // Diagonal discovery
                let diag_moves = cozy_chess::get_bishop_moves(vacated_sq, new_occ);
                let diag_sliders = diag_moves & bishops_queens;

                if !diag_sliders.is_empty() {
                    let valid_targets = diag_moves
                        & !new_board.pieces(Piece::King)
                        & !vacated_sq.bitboard()
                        & !ignore;

                    for slider_sq in diag_sliders {
                        let piece = new_board.color_piece_on(slider_sq).unwrap();
                        let enemy_targets = valid_targets & new_occ;

                        for att_sq in enemy_targets {
                            if cozy_chess::get_between_rays(slider_sq, att_sq).has(vacated_sq) {
                                let att_piece = new_board.color_piece_on(att_sq).unwrap();
                                out.push(ThreatDelta::new(piece, slider_sq, att_piece, att_sq));
                            }
                        }
                    }
                }
            }
        }

        if !(rooks_queens).is_empty() {
            // Orthogonal discovery
            let ortho_moves = cozy_chess::get_rook_moves(vacated_sq1, new_occ)
                | cozy_chess::get_rook_moves(vacated_sq2, new_occ);
            let ortho_sliders = ortho_moves & rooks_queens;

            if !ortho_sliders.is_empty() {
                let valid_targets = ortho_moves & !new_board.pieces(Piece::King) & !ignore;

                for slider_sq in ortho_sliders {
                    let piece = new_board.color_piece_on(slider_sq).unwrap();
                    let enemy_targets = valid_targets & new_occ;

                    for att_sq in enemy_targets {
                        if !(cozy_chess::get_between_rays(slider_sq, att_sq)
                            & (vacated_sq1.bitboard() | vacated_sq2.bitboard()))
                        .is_empty()
                        {
                            let att_piece = new_board.color_piece_on(att_sq).unwrap();
                            out.push(ThreatDelta::new(piece, slider_sq, att_piece, att_sq));
                        }
                    }
                }
            }
        }
    }

    fn get_threat_update(board: &Board, new_board: &Board, m: Move, threats: &mut ThreatUpdate) {
        match board.move_type(m) {
            MoveType::NORMAL | MoveType::PROMOTION => {
                /*
                Removes:
                from outgoing/incoming

                if cap
                    to outgoing/incoming
                else
                    blocked discovered attacks bypassing to

                Adds:
                to outgoing
                to incoming

                discovered attacks bypassing from
                 */
                let from = m.from;
                let to = m.to;
                let is_cap = board.piece_on(to).is_some();

                Self::record_sq_inout(board, from, BitBoard::FULL, true, &mut threats.subs);
                if is_cap {
                    Self::record_sq_inout(board, to, !from.bitboard(), true, &mut threats.subs);
                }

                Self::record_sq_inout(&new_board, to, BitBoard::FULL, true, &mut threats.adds);
                Self::record_discovered(&new_board, from, to.bitboard(), &mut threats.adds);

                if !is_cap {
                    Self::record_discovered(board, to, from.bitboard(), &mut threats.subs);
                }
            }
            MoveType::CASTLE => {
                // king takes rook
                let king_from = m.from;
                let rook_from = m.to;

                let (king_to, rook_to) = board.castle_to(m);

                Self::record_sq_inout(board, rook_from, BitBoard::FULL, true, &mut threats.subs);
                Self::record_sq_inout(&new_board, rook_to, BitBoard::FULL, true, &mut threats.adds);
            }
            MoveType::ENPASSENT => {
                Self::record_sq_inout(board, m.from, BitBoard::FULL, true, &mut threats.subs);
                Self::record_sq_inout(
                    board,
                    board.ep_capture_square().unwrap(),
                    BitBoard::FULL & !m.from.bitboard(),
                    true,
                    &mut threats.subs,
                );

                Self::record_sq_inout(&new_board, m.to, BitBoard::FULL, true, &mut threats.adds);
                Self::record_discovered(
                    board,
                    m.to,
                    board.ep_capture_square().unwrap().bitboard() | m.from.bitboard(),
                    &mut threats.subs,
                );

                Self::record_unblocked_discovered_ep(
                    &new_board,
                    m.from,
                    board.ep_capture_square().unwrap(),
                    m.to.bitboard(),
                    &mut threats.adds,
                );
            }
            _ => unreachable!(),
        }
    }

    pub fn unmake_move(&mut self) {
        self.head -= 1;
    }

    fn refresh(&mut self, side: Color, board: &Board, network: &Box<Network>) {
        let mut adds: ArrayVec<usize, 96> = ArrayVec::new();
        let occ = board.occupied();
        for sq1 in occ & !board.pieces(Piece::King) {
            let piece1 = board.color_piece_on(sq1).unwrap();

            let attacks = match piece1.piece {
                Piece::Pawn => cozy_chess::get_pawn_attacks(sq1, piece1.color),
                Piece::Knight => cozy_chess::get_knight_moves(sq1),
                Piece::Bishop => cozy_chess::get_bishop_moves(sq1, occ),
                Piece::Rook => cozy_chess::get_rook_moves(sq1, occ),
                Piece::Queen => {
                    cozy_chess::get_bishop_moves(sq1, occ) | cozy_chess::get_rook_moves(sq1, occ)
                }
                Piece::King => BitBoard::EMPTY,
            };

            for sq2 in occ & !board.pieces(Piece::King) & attacks {
                let piece2 = board.color_piece_on(sq2).unwrap();
                let i = network.threat_feature_lookup_index(
                    board.king(side),
                    side,
                    piece1.color,
                    piece2.color,
                    piece1.piece,
                    sq1,
                    piece2.piece,
                    sq2,
                );
                if i >= 0 {
                    adds.push(i as usize);
                }
            }
        }

        unsafe {
            let out = self.side[self.head].vals[side as usize].as_mut_ptr();
            for i in (0..HL).step_by(32 * 8) {
                let mut acc = [_mm512_setzero_si512(); 8];
                let mut add_idx = 0;

                while add_idx + 1 < adds.len() {
                    let add1 = network.threat_weights[adds[add_idx]].as_ptr().add(i);
                    let add2 = network.threat_weights[adds[add_idx + 1]].as_ptr().add(i);
                    for k in 0..8 {
                        acc[k] = _mm512_add_epi16(
                            acc[k],
                            _mm512_cvtepi8_epi16(*(add1.add(k * 32) as *const __m256i)),
                        );
                        acc[k] = _mm512_add_epi16(
                            acc[k],
                            _mm512_cvtepi8_epi16(*(add2.add(k * 32) as *const __m256i)),
                        );
                    }
                    add_idx += 2;
                }

                while add_idx < adds.len() {
                    let add = network.threat_weights[adds[add_idx]].as_ptr().add(i);
                    for k in 0..8 {
                        acc[k] = _mm512_add_epi16(
                            acc[k],
                            _mm512_cvtepi8_epi16(*(add.add(k * 32) as *const __m256i)),
                        );
                    }
                    add_idx += 1;
                }
                for k in 0..8 {
                    *(out.add(i + k * 32) as *mut __m512i) = acc[k];
                }
            }
        }

        self.side[self.head].is_clean[side as usize] = true;
    }

    pub fn catchup(&mut self, board: &Board, network: &Box<Network>) {
        for side in Color::ALL {
            if self.side[self.head].is_clean[side as usize] {
                continue;
            }

            let mut base = self.head;
            loop {
                if Network::needs_refresh_threat(
                    self.side[base].king_sq[side as usize],
                    self.side[self.head].king_sq[side as usize],
                ) {
                    self.refresh(side, board, network);
                    break;
                }

                if self.side[base].is_clean[side as usize] {
                    for i in base + 1..=self.head {
                        let (base, next) = self.side.split_at_mut(i);
                        network.threat_apply_update(
                            &mut next[0].vals[side as usize],
                            &base[i - 1].vals[side as usize],
                            &next[0].update,
                            side,
                            board.king(side),
                        );
                        next[0].is_clean[side as usize] = true;
                    }

                    self.side[self.head].is_clean[side as usize] = true;
                    break;
                }

                if base == 0 {
                    panic!("no clean base");
                }
                base -= 1;
            }
        }
    }
}
