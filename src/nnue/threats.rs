use std::{
    collections::HashSet,
    mem::{self},
};

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
        acc.update = Self::get_threat_update(board, new_board, m);
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

        let ntm = board.colors(!piece.color);

        // remove outgoing
        let attacks = match piece.piece {
            Piece::Pawn => pawn,
            Piece::Knight => knight,
            Piece::Bishop => bishop,
            Piece::Rook => rook,
            Piece::Queen => queen,
            Piece::King => unreachable!(),
        };
        for target_sq in attacks & ntm & mask & !(board.pieces(Piece::King)) {
            let piece2: ColoredPiece = board.color_piece_on(target_sq).unwrap();
            out.push(ThreatDelta::new(
                piece.piece,
                sq,
                piece2.piece,
                target_sq,
                piece.color,
            ));
        }

        if also_incoming {
            // remove incoming
            let incoming = (pawn & board.pieces(Piece::Pawn))
                | (knight & board.pieces(Piece::Knight))
                | (bishop & board.pieces(Piece::Bishop))
                | (rook & board.pieces(Piece::Rook))
                | (queen & board.pieces(Piece::Queen));

            for in_sq in incoming & ntm & mask {
                let in_piece = board.color_piece_on(in_sq).unwrap();
                out.push(ThreatDelta::new(
                    in_piece.piece,
                    in_sq,
                    piece.piece,
                    sq,
                    in_piece.color,
                ));
            }
        }
    }

    /// Records new attacks created because `vacated_sq` was emptied.
    fn record_unblocked_discovered(
        new_board: &Board,
        vacated_sq: Square,
        ignore_sq: Square,
        out: &mut ThreatDeltaUpdates,
    ) {
        let new_occ = new_board.occupied();
        let bishops_queens = new_board.pieces(Piece::Bishop) | new_board.pieces(Piece::Queen);
        let rooks_queens = new_board.pieces(Piece::Rook) | new_board.pieces(Piece::Queen);

        if !(bishops_queens & !ignore_sq.bitboard()).is_empty() {
            // Diagonal discovery
            let diag_moves = cozy_chess::get_bishop_moves(vacated_sq, new_occ);
            let diag_sliders = diag_moves & bishops_queens & !ignore_sq.bitboard();

            if !diag_sliders.is_empty() {
                let valid_targets = diag_moves
                    & !new_board.pieces(Piece::King)
                    & !ignore_sq.bitboard()
                    & !vacated_sq.bitboard();

                for slider_sq in diag_sliders {
                    let piece = new_board.color_piece_on(slider_sq).unwrap();
                    let enemy_targets = valid_targets & new_board.colors(!piece.color);

                    for att_sq in enemy_targets {
                        if cozy_chess::get_between_rays(slider_sq, att_sq).has(vacated_sq) {
                            let att_piece = new_board.color_piece_on(att_sq).unwrap();
                            out.push(ThreatDelta::new(
                                piece.piece,
                                slider_sq,
                                att_piece.piece,
                                att_sq,
                                piece.color,
                            ));
                        }
                    }
                }
            }
        }

        if !(rooks_queens & !ignore_sq.bitboard()).is_empty() {
            // Orthogonal discovery
            let ortho_moves = cozy_chess::get_rook_moves(vacated_sq, new_occ);
            let ortho_sliders = ortho_moves & rooks_queens & !ignore_sq.bitboard();

            if !ortho_sliders.is_empty() {
                let valid_targets = ortho_moves
                    & !new_board.pieces(Piece::King)
                    & !ignore_sq.bitboard()
                    & !vacated_sq.bitboard();

                for slider_sq in ortho_sliders {
                    let piece = new_board.color_piece_on(slider_sq).unwrap();
                    let enemy_targets = valid_targets & new_board.colors(!piece.color);

                    for att_sq in enemy_targets {
                        if cozy_chess::get_between_rays(slider_sq, att_sq).has(vacated_sq) {
                            let att_piece = new_board.color_piece_on(att_sq).unwrap();
                            out.push(ThreatDelta::new(
                                piece.piece,
                                slider_sq,
                                att_piece.piece,
                                att_sq,
                                piece.color,
                            ));
                        }
                    }
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
                        let enemy_targets = valid_targets & new_board.colors(!piece.color);

                        for att_sq in enemy_targets {
                            if cozy_chess::get_between_rays(slider_sq, att_sq).has(vacated_sq) {
                                let att_piece = new_board.color_piece_on(att_sq).unwrap();
                                out.push(ThreatDelta::new(
                                    piece.piece,
                                    slider_sq,
                                    att_piece.piece,
                                    att_sq,
                                    piece.color,
                                ));
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
                    let enemy_targets = valid_targets & new_board.colors(!piece.color);

                    for att_sq in enemy_targets {
                        if !(cozy_chess::get_between_rays(slider_sq, att_sq)
                            & (vacated_sq1.bitboard() | vacated_sq2.bitboard()))
                        .is_empty()
                        {
                            let att_piece = new_board.color_piece_on(att_sq).unwrap();
                            out.push(ThreatDelta::new(
                                piece.piece,
                                slider_sq,
                                att_piece.piece,
                                att_sq,
                                piece.color,
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Records old attacks destroyed because `blocked_sq` became occupied.
    fn record_blocked_discovered(
        old_board: &Board,
        blocked_sq: Square,
        ignore: BitBoard,
        out: &mut ThreatDeltaUpdates,
    ) {
        let old_occ = old_board.occupied();
        let bishops_queens = old_board.pieces(Piece::Bishop) | old_board.pieces(Piece::Queen);
        let rooks_queens = old_board.pieces(Piece::Rook) | old_board.pieces(Piece::Queen);

        if !(bishops_queens & !ignore).is_empty() {
            // Diagonal blocked attacks
            let diag_moves = cozy_chess::get_bishop_moves(blocked_sq, old_occ);
            let diag_sliders = diag_moves & bishops_queens & !ignore;

            if !diag_sliders.is_empty() {
                let valid_targets =
                    diag_moves & !old_board.pieces(Piece::King) & !ignore & !blocked_sq.bitboard();

                for slider_sq in diag_sliders {
                    let piece = old_board.color_piece_on(slider_sq).unwrap();
                    let enemy_targets = valid_targets & old_board.colors(!piece.color);

                    for att_sq in enemy_targets {
                        if cozy_chess::get_between_rays(slider_sq, att_sq).has(blocked_sq) {
                            let att_piece = old_board.color_piece_on(att_sq).unwrap();
                            out.push(ThreatDelta::new(
                                piece.piece,
                                slider_sq,
                                att_piece.piece,
                                att_sq,
                                piece.color,
                            ));
                        }
                    }
                }
            }
        }

        if !(rooks_queens & !ignore).is_empty() {
            // Orthogonal blocked attacks
            let ortho_moves = cozy_chess::get_rook_moves(blocked_sq, old_occ);
            let ortho_sliders = ortho_moves & rooks_queens & !ignore;

            if !ortho_sliders.is_empty() {
                let valid_targets =
                    ortho_moves & !old_board.pieces(Piece::King) & !ignore & !blocked_sq.bitboard();

                for slider_sq in ortho_sliders {
                    let piece = old_board.color_piece_on(slider_sq).unwrap();
                    let enemy_targets = valid_targets & old_board.colors(!piece.color);

                    for att_sq in enemy_targets {
                        if cozy_chess::get_between_rays(slider_sq, att_sq).has(blocked_sq) {
                            let att_piece = old_board.color_piece_on(att_sq).unwrap();
                            out.push(ThreatDelta::new(
                                piece.piece,
                                slider_sq,
                                att_piece.piece,
                                att_sq,
                                piece.color,
                            ));
                        }
                    }
                }
            }
        }
    }

    fn get_threat_update(board: &Board, new_board: &Board, m: Move) -> ThreatUpdate {
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
                let mut threats = ThreatUpdate::default();

                let from = m.from;
                let to = m.to;
                let is_cap = board.piece_on(to).is_some();

                Self::record_sq_inout(board, from, BitBoard::FULL, true, &mut threats.subs);
                if is_cap {
                    Self::record_sq_inout(
                        board,
                        to,
                        BitBoard::FULL & !from.bitboard(),
                        true,
                        &mut threats.subs,
                    );
                }

                Self::record_sq_inout(&new_board, to, BitBoard::FULL, true, &mut threats.adds);
                Self::record_unblocked_discovered(&new_board, from, to, &mut threats.adds);

                if !is_cap {
                    Self::record_blocked_discovered(board, to, from.bitboard(), &mut threats.subs);
                }

                threats
            }
            MoveType::CASTLE => {
                let mut threats = ThreatUpdate::default();
                // king takes rook
                let king_from = m.from;
                let rook_from = m.to;

                let (king_to, rook_to) = board.castle_to(m);

                Self::record_sq_inout(board, rook_from, BitBoard::FULL, true, &mut threats.subs);

                Self::record_sq_inout(
                    &new_board,
                    rook_to,
                    BitBoard::FULL,
                    false,
                    &mut threats.adds,
                );

                // if threats.subs.len() > 0 {
                //     println!("{:?} {}", threats, board);
                // }

                // guaranteed nothing attacking and starts attacking rook/king
                // guaranteed no discovered attacks or blocked attacks
                threats
            }
            MoveType::ENPASSENT => {
                let mut threats = ThreatUpdate::default();

                Self::record_sq_inout(board, m.from, BitBoard::FULL, true, &mut threats.subs);
                Self::record_sq_inout(
                    board,
                    board.ep_capture_square().unwrap(),
                    BitBoard::FULL & !m.from.bitboard(),
                    true,
                    &mut threats.subs,
                );

                Self::record_sq_inout(&new_board, m.to, BitBoard::FULL, true, &mut threats.adds);
                Self::record_blocked_discovered(
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

                threats
            }
            _ => unreachable!(),
        }
    }

    pub fn unmake_move(&mut self) {
        self.head -= 1;
    }

    fn refresh(&mut self, side: Color, board: &Board, network: &Box<Network>) {
        SimdOps::zero(&mut self.side[self.head].vals[side as usize]);

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

            for sq2 in board.colors(!piece1.color) & !board.pieces(Piece::King) & attacks {
                let piece2 = board.color_piece_on(sq2).unwrap();
                SimdOps::fused_add2(
                    &mut self.side[self.head].vals[side as usize],
                    network.threat_feature_lookup(
                        board.king(side),
                        side,
                        piece1.color,
                        piece1.piece,
                        sq1,
                        piece2.piece,
                        sq2,
                    ),
                );
            }
        }

        self.side[self.head].is_clean[side as usize] = true;
    }

    pub fn catchup(&mut self, board: &Board, network: &Box<Network>) {
        for side in Color::ALL {
            if self.side[self.head].is_clean[side as usize] {
                continue;
            }

            // if board.hash() % 4 == 0 {
            // self.refresh(side,board, network);
            // return;
            // }

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
