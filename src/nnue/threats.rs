use std::ops::Sub;

use arrayvec::ArrayVec;
use cozy_chess::{BitBoard, Board, Color, Move, Piece, Square};

use crate::{
    ext::{BitBoardExt, ColoredPiece, ExtBoard, MoveType},
    nnue::{
        network::{Aligned, HL, Network},
        ti::{self, MAX_TI_INPUTS, pawn_mask3},
        update::{ThreatRows, ThreatUpdate},
    },
    param::MAX_DEPTH_USIZE,
};

#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [Aligned<i16, HL>; 2],
    is_clean: [bool; 2],
    king_sq: [Square; 2],
    update: ThreatUpdate,
    // for lazy [update], (prev_board, board, move)
    pending: Option<(Board, Board, Move)>,
}

impl Accumulator {
    fn new() -> Self {
        Self {
            vals: [Aligned::<i16, HL>::zeroed(), Aligned::<i16, HL>::zeroed()],
            is_clean: [false; 2],
            update: ThreatUpdate::default(),
            pending: None,
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
            self.refresh(side, self.head, board, network);
        }
    }

    pub fn make_move(&mut self, board: &Board, new_board: &Board, m: Move) {
        self.head += 1;
        let acc = &mut self.side[self.head];
        acc.is_clean = [false; 2];
        acc.king_sq[0] = new_board.king(Color::White);
        acc.king_sq[1] = new_board.king(Color::Black);
        // lazy threat update compute
        acc.pending = Some((board.clone(), new_board.clone(), m));
    }

    fn record_sq_inout(board: &Board, sq: Square, mask: BitBoard, out: &mut ThreatRows) {
        let piece = board.color_piece_on(sq).unwrap();
        if piece.piece == Piece::King {
            return;
        }

        let occ = board.occupied();

        let knight = cozy_chess::get_knight_moves(sq);
        let bishop = cozy_chess::get_bishop_moves(sq, occ);
        let rook = cozy_chess::get_rook_moves(sq, occ);
        let queen = bishop | rook;

        // remove outgoing
        let attacks = match piece.piece {
            Piece::Pawn => cozy_chess::get_pawn_attacks(sq, piece.color),
            Piece::Knight => knight,
            Piece::Bishop => bishop,
            Piece::Rook => rook,
            Piece::Queen => queen,
            Piece::King => unreachable!(),
        };

        let targets = board.piece_select(|t| ti::is_feature(piece.piece, t));
        for target_sq in attacks & targets & mask {
            let piece2 = board.color_piece_on(target_sq).unwrap();
            out.push_threat(piece, sq, piece2, target_sq);
        }

        // remove incoming, prefiltered to attacker types that form a feature with [piece]
        let ok = |a: Piece| -> BitBoard {
            if ti::is_feature(a, piece.piece) {
                board.pieces(a)
            } else {
                BitBoard::EMPTY
            }
        };
        let pawn_ok = ok(Piece::Pawn);
        let incoming_pawns =
            (cozy_chess::get_pawn_attacks(sq, Color::Black) & board.colors(Color::White) & pawn_ok)
                | (cozy_chess::get_pawn_attacks(sq, Color::White)
                    & board.colors(Color::Black)
                    & pawn_ok);
        let incoming = incoming_pawns
            | (knight & ok(Piece::Knight))
            | (bishop & ok(Piece::Bishop))
            | (rook & ok(Piece::Rook))
            | (queen & ok(Piece::Queen));

        for in_sq in incoming & mask {
            let in_piece = board.color_piece_on(in_sq).unwrap();
            out.push_threat(in_piece, in_sq, piece, sq);
        }
    }

    /// Records new attacks created because `vacated_sq` was emptied.
    fn record_discovered(
        new_board: &Board,
        vacated_sq: Square,
        ignore_sq: BitBoard,
        out: &mut ThreatRows,
        ignored_dir: usize,
    ) {
        let occ = new_board.occupied();
        let queens = new_board.pieces(Piece::Queen);
        let rq = new_board.pieces(Piece::Rook) | queens;
        let bq = new_board.pieces(Piece::Bishop) | queens;
        let skip = new_board.pieces(Piece::King) | ignore_sq;

        for dir in 0..4 {
            if dir == ignored_dir {
                continue;
            }

            let ty = if dir & 1 == 0 { rq } else { bq };
            let line = get_line(vacated_sq, dir) & occ;

            // no slider of the right kind anywhere on the line: nothing to discover
            if (line & ty).is_empty() {
                continue;
            }

            if let Some((a, b)) = line.pop2(vacated_sq as usize) {
                let ends = a.bitboard() | b.bitboard();
                if !(ends & skip).is_empty() || (ends & ty).is_empty() {
                    continue;
                }

                let pa = new_board.color_piece_on(a).unwrap();
                let pb = new_board.color_piece_on(b).unwrap();

                if ty.has(a) && ti::is_feature(pa.piece, pb.piece) {
                    out.push_threat(pa, a, pb, b);
                }

                if ty.has(b) && ti::is_feature(pb.piece, pa.piece) {
                    out.push_threat(pb, b, pa, a);
                }
            }
        }
    }

    fn record_pp3wide(
        board: &Board,
        c: Color,
        sq: Square,
        exclude: BitBoard,
        out: &mut ThreatRows,
    ) {
        let pawns = [
            board
                .colored_pieces(Color::White, Piece::Pawn)
                .sub(exclude)
                .sub(sq.bitboard()),
            board
                .colored_pieces(Color::Black, Piece::Pawn)
                .sub(exclude)
                .sub(sq.bitboard()),
        ];

        let mask = pawn_mask3(sq);
        for (color, pawns) in Color::ALL.into_iter().zip(pawns.into_iter()) {
            for pawn in pawns & mask {
                out.push_pp((c, sq), (color, pawn));
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

                Self::record_sq_inout(board, from, BitBoard::FULL, &mut threats.subs);
                if is_cap {
                    Self::record_sq_inout(board, to, !from.bitboard(), &mut threats.subs);
                }

                Self::record_sq_inout(&new_board, to, BitBoard::FULL, &mut threats.adds);
                Self::record_discovered(&new_board, from, to.bitboard(), &mut threats.adds, 4);

                if !is_cap {
                    Self::record_discovered(board, to, from.bitboard(), &mut threats.subs, 4);
                }
            }
            MoveType::CASTLE => {
                // king takes rook
                let rook_from = m.to;
                let (_king_to, rook_to) = board.castle_to(m);

                Self::record_sq_inout(board, rook_from, BitBoard::FULL, &mut threats.subs);
                Self::record_sq_inout(&new_board, rook_to, BitBoard::FULL, &mut threats.adds);
            }
            MoveType::ENPASSENT => {
                Self::record_sq_inout(board, m.from, BitBoard::FULL, &mut threats.subs);
                Self::record_sq_inout(
                    board,
                    board.ep_capture_square().unwrap(),
                    !m.from.bitboard(),
                    &mut threats.subs,
                );

                Self::record_sq_inout(&new_board, m.to, BitBoard::FULL, &mut threats.adds);
                Self::record_discovered(
                    board,
                    m.to,
                    board.ep_capture_square().unwrap().bitboard() | m.from.bitboard(),
                    &mut threats.subs,
                    4,
                );

                // we unblock two on the same rank
                Self::record_discovered(new_board, m.from, m.to.bitboard(), &mut threats.adds, 4);

                Self::record_discovered(
                    new_board,
                    board.ep_capture_square().unwrap(),
                    m.to.bitboard(),
                    &mut threats.adds,
                    0,
                    // horizontal ignore
                );
            }
            MoveType::NONE => unreachable!(),
        }

        let mut dirty = BitBoard::EMPTY;
        if board.piece_on(m.to).is_some() {
            dirty |= m.to.bitboard();
        }

        // pinned updates
        // diff bitboard and mark dirty

        // stm of [board]
        {
            let old_stm = board.pinned();
            let new_stm = new_board.opp_pinned();

            let mut removed = old_stm & !new_stm;
            let mut added = new_stm & !old_stm;

            if !(dirty & (new_stm & old_stm)).is_empty() {
                removed |= dirty;
                added |= dirty;
            }

            for square in removed {
                threats.subs.push_pinned(
                    board.color_piece_on(square).unwrap(),
                    square,
                    board.side_to_move(),
                );
            }

            for square in added {
                threats.adds.push_pinned(
                    new_board.color_piece_on(square).unwrap(),
                    square,
                    board.side_to_move(),
                );
            }
        }

        // ntm of [board]
        {
            let old_stm = board.opp_pinned();
            let new_stm = new_board.pinned();

            let mut removed = old_stm & !new_stm;
            let mut added = new_stm & !old_stm;

            if !(dirty & (new_stm & old_stm)).is_empty() {
                removed |= dirty;
                added |= dirty;
            }

            for square in removed {
                threats.subs.push_pinned(
                    board.color_piece_on(square).unwrap(),
                    square,
                    !board.side_to_move(),
                );
            }

            for square in added {
                threats.adds.push_pinned(
                    new_board.color_piece_on(square).unwrap(),
                    square,
                    !board.side_to_move(),
                );
            }
        }

        // pp 3wide updates
        match board.move_type(m) {
            MoveType::CASTLE => {}
            MoveType::NORMAL => {
                // if pawn moved
                if matches!(board.piece_on(m.from), Some(Piece::Pawn)) {
                    Self::record_pp3wide(
                        board,
                        board.side_to_move(),
                        m.from,
                        BitBoard::EMPTY,
                        &mut threats.subs,
                    );
                    Self::record_pp3wide(
                        new_board,
                        board.side_to_move(),
                        m.to,
                        BitBoard::EMPTY,
                        &mut threats.adds,
                    );
                }

                // if captured pawn. I think the from exclude works even if captured by a pawn
                if matches!(board.piece_on(m.to), Some(Piece::Pawn)) {
                    Self::record_pp3wide(
                        board,
                        !board.side_to_move(),
                        m.to,
                        m.from.bitboard(),
                        &mut threats.subs,
                    );
                }
            }
            MoveType::PROMOTION => {
                Self::record_pp3wide(
                    board,
                    board.side_to_move(),
                    m.from,
                    BitBoard::EMPTY,
                    &mut threats.subs,
                );
            }
            MoveType::ENPASSENT => {
                Self::record_pp3wide(
                    board,
                    board.side_to_move(),
                    m.from,
                    BitBoard::EMPTY,
                    &mut threats.subs,
                );
                Self::record_pp3wide(
                    new_board,
                    board.side_to_move(),
                    m.to,
                    BitBoard::EMPTY,
                    &mut threats.adds,
                );

                let ep_square = board.ep_capture_square().unwrap();
                Self::record_pp3wide(
                    board,
                    !board.side_to_move(),
                    ep_square,
                    m.from.bitboard(),
                    &mut threats.subs,
                );
            }
            MoveType::NONE => panic!(),
        }
    }

    pub fn unmake_move(&mut self) {
        self.head -= 1;
    }

    fn refresh(&mut self, side: Color, head: usize, board: &Board, network: &Box<Network>) {
        let mut adds: ArrayVec<u32, MAX_TI_INPUTS> = ArrayVec::new();
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
                let i = network.threat_index(
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
                    adds.push(i as u32);
                }
            }
        }

        // pinned features
        // board stm
        for square in board.pinned() {
            adds.push(network.threat_pinned_index(
                board.king(side),
                side,
                side == board.side_to_move(),
                board.color_piece_on(square).unwrap(),
                square,
            ) as u32);
        }

        // board ntm
        for square in board.opp_pinned() {
            adds.push(network.threat_pinned_index(
                board.king(side),
                side,
                side != board.side_to_move(),
                board.color_piece_on(square).unwrap(),
                square,
            ) as u32);
        }

        // pp3wide features
        let mut pawns = board.pieces(Piece::Pawn);
        while let Some(square1) = pawns.next_square() {
            pawns ^= square1.bitboard();

            let mask = pawn_mask3(square1);
            for square2 in mask & pawns & !square1.bitboard() {
                adds.push(network.threat_pp_index(
                    board.king(side),
                    side,
                    (board.color_on(square1).unwrap(), square1),
                    (board.color_on(square2).unwrap(), square2),
                ) as u32);
            }
        }

        unsafe {
            network.threat_accumulate(
                self.side[head].vals[side as usize].as_mut_ptr(),
                std::ptr::null(),
                &adds,
                &[],
            );
        }

        self.side[head].is_clean[side as usize] = true;
    }

    pub fn catchup(&mut self, head: usize, board: &Board, network: &Box<Network>) {
        for side in Color::ALL {
            if self.side[head].is_clean[side as usize] {
                continue;
            }

            let mut base = head;
            loop {
                if Network::needs_refresh_threat(
                    self.side[base].king_sq[side as usize],
                    self.side[head].king_sq[side as usize],
                ) {
                    self.refresh(side, head, board, network);
                    break;
                }

                if self.side[base].is_clean[side as usize] {
                    // lazy update first to prefetch
                    for acc in &mut self.side[base + 1..=head] {
                        // note that acc.pending is shared for both sides
                        if let Some((old_board, new_board, m)) = acc.pending.take() {
                            acc.update.reset(acc.king_sq, network);
                            Self::get_threat_update(&old_board, &new_board, m, &mut acc.update);
                        }
                    }

                    for i in base + 1..=head {
                        let (base, next) = self.side.split_at_mut(i);
                        let next = &mut next[0];
                        unsafe {
                            network.threat_accumulate(
                                next.vals[side as usize].as_mut_ptr(),
                                base[i - 1].vals[side as usize].as_ptr(),
                                &next.update.adds.rows[side as usize],
                                &next.update.subs.rows[side as usize],
                            );
                        }
                        next.is_clean[side as usize] = true;
                    }
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
