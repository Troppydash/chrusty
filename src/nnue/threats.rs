use std::{
    collections::HashSet,
    mem::{self},
};

use arrayvec::ArrayVec;
use cozy_chess::{BitBoard, Board, Color, Move, Piece, Square};

use crate::{
    ext::{BitBoardExt, ColoredPiece, ExtBoard, MoveType},
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
    update: ThreatUpdate,
}

impl Accumulator {
    fn new() -> Self {
        Self {
            vals: [Aligned::<i16, HL>::zeroed(), Aligned::<i16, HL>::zeroed()],
            is_clean: [false; 2],
            update: ThreatUpdate::default(),
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
        self.side[self.head].update.clear();
        self.refresh(board, Color::White, network);
        self.refresh(board, Color::Black, network);
    }

    pub fn make_move(&mut self, board: &Board, m: Move) {
        self.head += 1;
        let acc = &mut self.side[self.head];
        acc.is_clean = [false; 2];
        acc.update.clear();
        acc.update.move_type = board.move_type(m);

        // Fallback to full refresh for complex move types
        if matches!(
            acc.update.move_type,
            MoveType::CASTLE | MoveType::PROMOTION | MoveType::ENPASSENT
        ) {
            return;
        }

        match acc.update.move_type {
            MoveType::NORMAL => {
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

                Self::record_sq_inout(board, from, BitBoard::FULL, &mut acc.update.subs);
                if is_cap {
                    Self::record_sq_inout(
                        board,
                        to,
                        BitBoard::FULL & !from.bitboard(),
                        &mut acc.update.subs,
                    );
                }

                let mut new_board = board.clone();
                new_board.play(m);

                Self::record_sq_inout(&new_board, to, BitBoard::FULL, &mut acc.update.adds);
                Self::record_unblocked_discovered(
                    board,
                    &new_board,
                    from,
                    to,
                    &mut acc.update.adds,
                );

                if !is_cap {
                    Self::record_blocked_discovered(
                        board,
                        &new_board,
                        to,
                        from,
                        &mut acc.update.subs,
                    );
                }
            }
            _ => panic!(""),
        }
    }

    fn record_sq_inout(board: &Board, sq: Square, mask: BitBoard, out: &mut ThreatDeltaUpdates) {
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
            out.push(ThreatDelta {
                p1: piece,
                sq1: sq,
                p2: piece2,
                sq2: target_sq,
            });
        }

        // remove incoming
        let incoming = (pawn & board.pieces(Piece::Pawn))
            | (knight & board.pieces(Piece::Knight))
            | (bishop & board.pieces(Piece::Bishop))
            | (rook & board.pieces(Piece::Rook))
            | (queen & board.pieces(Piece::Queen));

        for in_sq in incoming & ntm & mask {
            let in_piece = board.color_piece_on(in_sq).unwrap();
            out.push(ThreatDelta {
                p1: in_piece,
                sq1: in_sq,
                p2: piece,
                sq2: sq,
            });
        }
    }

    /// Records new attacks created because `vacated_sq` was emptied.
    fn record_unblocked_discovered(
        old_board: &Board,
        new_board: &Board,
        vacated_sq: Square,
        ignore_sq: Square,
        out: &mut ThreatDeltaUpdates,
    ) {
        let old_occ = old_board.occupied();
        let new_occ = new_board.occupied();

        let bishops_queens = new_board.pieces(Piece::Bishop) | new_board.pieces(Piece::Queen);
        let rooks_queens = new_board.pieces(Piece::Rook) | new_board.pieces(Piece::Queen);

        let diag_sliders = cozy_chess::get_bishop_moves(vacated_sq, new_occ)
            & bishops_queens
            & !ignore_sq.bitboard();

        let ortho_sliders =
            cozy_chess::get_rook_moves(vacated_sq, new_occ) & rooks_queens & !ignore_sq.bitboard();

        let valid_targets =
            !new_board.pieces(Piece::King) & !ignore_sq.bitboard() & !vacated_sq.bitboard();

        for slider_sq in diag_sliders {
            let piece = new_board.color_piece_on(slider_sq).unwrap();
            let new_attacks = cozy_chess::get_bishop_moves(slider_sq, new_occ)
                & new_board.colors(!piece.color)
                & valid_targets;
            let old_attacks =
                cozy_chess::get_bishop_moves(slider_sq, old_occ) & old_board.colors(!piece.color);

            for att_sq in new_attacks & !old_attacks {
                let att_piece = new_board.color_piece_on(att_sq).unwrap();
                out.push(ThreatDelta {
                    p1: piece,
                    sq1: slider_sq,
                    p2: att_piece,
                    sq2: att_sq,
                });
            }
        }

        for slider_sq in ortho_sliders {
            let piece = new_board.color_piece_on(slider_sq).unwrap();
            let new_attacks = cozy_chess::get_rook_moves(slider_sq, new_occ)
                & new_board.colors(!piece.color)
                & valid_targets;
            let old_attacks =
                cozy_chess::get_rook_moves(slider_sq, old_occ) & old_board.colors(!piece.color);

            for att_sq in new_attacks & !old_attacks {
                let att_piece = new_board.color_piece_on(att_sq).unwrap();
                out.push(ThreatDelta {
                    p1: piece,
                    sq1: slider_sq,
                    p2: att_piece,
                    sq2: att_sq,
                });
            }
        }
    }

    /// Records old attacks destroyed because `blocked_sq` became occupied.
    fn record_blocked_discovered(
        old_board: &Board,
        new_board: &Board,
        blocked_sq: Square,
        ignore_sq: Square,
        out: &mut ThreatDeltaUpdates,
    ) {
        let old_occ = old_board.occupied();
        let new_occ = new_board.occupied();

        let bishops_queens = old_board.pieces(Piece::Bishop) | old_board.pieces(Piece::Queen);
        let rooks_queens = old_board.pieces(Piece::Rook) | old_board.pieces(Piece::Queen);

        // Look for sliders on the OLD board that passed through blocked_sq
        let diag_sliders = cozy_chess::get_bishop_moves(blocked_sq, old_occ)
            & bishops_queens
            & !ignore_sq.bitboard();

        let ortho_sliders =
            cozy_chess::get_rook_moves(blocked_sq, old_occ) & rooks_queens & !ignore_sq.bitboard();

        let valid_targets =
            !old_board.pieces(Piece::King) & !ignore_sq.bitboard() & !blocked_sq.bitboard();

        for slider_sq in diag_sliders {
            let piece = old_board.color_piece_on(slider_sq).unwrap();
            let old_attacks = cozy_chess::get_bishop_moves(slider_sq, old_occ)
                & old_board.colors(!piece.color)
                & valid_targets;
            let new_attacks =
                cozy_chess::get_bishop_moves(slider_sq, new_occ) & new_board.colors(!piece.color);

            for att_sq in old_attacks & !new_attacks {
                let att_piece = old_board.color_piece_on(att_sq).unwrap();
                out.push(ThreatDelta {
                    p1: piece,
                    sq1: slider_sq,
                    p2: att_piece,
                    sq2: att_sq,
                });
            }
        }

        for slider_sq in ortho_sliders {
            let piece = old_board.color_piece_on(slider_sq).unwrap();
            let old_attacks = cozy_chess::get_rook_moves(slider_sq, old_occ)
                & old_board.colors(!piece.color)
                & valid_targets;
            let new_attacks =
                cozy_chess::get_rook_moves(slider_sq, new_occ) & new_board.colors(!piece.color);

            for att_sq in old_attacks & !new_attacks {
                let att_piece = old_board.color_piece_on(att_sq).unwrap();
                out.push(ThreatDelta {
                    p1: piece,
                    sq1: slider_sq,
                    p2: att_piece,
                    sq2: att_sq,
                });
            }
        }
    }

    // fn get_attacks(occ: BitBoard, sq: Square, piece: ColoredPiece) -> BitBoard {
    //     match piece.piece {
    //         Piece::Pawn => cozy_chess::get_pawn_attacks(sq, piece.color),
    //         Piece::Knight => cozy_chess::get_knight_moves(sq),
    //         Piece::Bishop => cozy_chess::get_bishop_moves(sq, occ),
    //         Piece::Rook => cozy_chess::get_rook_moves(sq, occ),
    //         Piece::Queen => {
    //             cozy_chess::get_bishop_moves(sq, occ) | cozy_chess::get_rook_moves(sq, occ)
    //         }
    //         Piece::King => BitBoard::EMPTY,
    //     }
    // }

    // fn record_sq_outgoing(board: &Board, sq: Square, out: &mut ThreatDeltaUpdates) {
    //     let piece1 = board.color_piece_on(sq).unwrap();
    //     if piece1.piece == Piece::King {
    //         return;
    //     }

    //     let targets = Self::get_attacks(board.occupied(), sq, piece1)
    //         & board.colors(!piece1.color)
    //         & !board.pieces(Piece::King);
    //     for target_sq in targets {
    //         let piece2 = board.color_piece_on(target_sq).unwrap();
    //         out.push(ThreatDelta {
    //             p1: piece1,
    //             sq1: sq,
    //             p2: piece2,
    //             sq2: target_sq,
    //         });
    //     }
    // }

    // fn record_sq_incoming_non_sliders(
    //     board: &Board,
    //     target_sq: Square,
    //     occ: BitBoard,
    //     out: &mut ThreatDeltaUpdates,
    // ) {
    //     let piece2 = board.color_piece_on(target_sq).unwrap();
    //     if piece2.piece == Piece::King {
    //         return;
    //     }

    //     let pawn_attackers =
    //         cozy_chess::get_pawn_attacks(target_sq, !piece2.color) & board.pieces(Piece::Pawn);
    //     let knight_attackers =
    //         cozy_chess::get_knight_moves(target_sq) & board.pieces(Piece::Knight);

    //     let attackers = (pawn_attackers | knight_attackers) & board.colors(!piece2.color) & occ;
    //     for attacker_sq in attackers {
    //         let piece1 = board.color_piece_on(attacker_sq).unwrap();
    //         out.push(ThreatDelta {
    //             p1: piece1,
    //             sq1: attacker_sq,
    //             p2: piece2,
    //             sq2: target_sq,
    //         });
    //     }
    // }

    // fn get_ray_aligned_sliders(board: &Board, occ: BitBoard, sqs: &[Square]) -> BitBoard {
    //     let mut sliders = BitBoard::EMPTY;
    //     let bishops_queens = board.pieces(Piece::Bishop) | board.pieces(Piece::Queen);
    //     let rooks_queens = board.pieces(Piece::Rook) | board.pieces(Piece::Queen);

    //     for &sq in sqs {
    //         let diag = cozy_chess::get_bishop_moves(sq, occ) & bishops_queens;
    //         let ortho = cozy_chess::get_rook_moves(sq, occ) & rooks_queens;
    //         sliders |= diag | ortho;
    //     }
    //     sliders
    // }

    pub fn unmake_move(&mut self) {
        self.head -= 1;
    }

    fn refresh(&mut self, board: &Board, side: Color, network: &Box<Network>) {
        SimdOps::zero(&mut self.side[self.head].vals[side as usize]);

        let occ = board.occupied();
        for sq1 in occ {
            let piece1 = board.color_piece_on(sq1).unwrap();
            if piece1.piece == Piece::King {
                continue;
            }

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

            for sq2 in occ {
                let piece2 = board.color_piece_on(sq2).unwrap();
                if piece2.color == piece1.color {
                    continue;
                }

                if piece2.piece == Piece::King {
                    continue;
                }

                if attacks.has(sq2) {
                    SimdOps::fused_add(
                        &mut self.side[self.head].vals[side as usize],
                        network.threat_feature_lookup(side, piece1, sq1, piece2, sq2),
                    );
                }
            }
        }

        self.side[self.head].is_clean[side as usize] = true;
    }

    pub fn catchup(&mut self, board: &Board, network: &Box<Network>) {
        for side in 0..=1 {
            if self.side[self.head].is_clean[side] {
                continue;
            }

            let mut base = self.head;
            loop {
                if matches!(
                    self.side[base].update.move_type,
                    MoveType::CASTLE | MoveType::PROMOTION | MoveType::ENPASSENT
                ) {
                    self.refresh(board, Color::ALL[side], network);
                    break;
                }

                if self.side[base].is_clean[side] {
                    for i in base + 1..=self.head {
                        let (base, next) = self.side.split_at_mut(i);

                        network.threat_apply_update(
                            &mut next[0].vals[side],
                            &base[i - 1].vals[side],
                            &next[0].update,
                            Color::ALL[side],
                        );
                        self.side[i].is_clean[side] = true;
                    }

                    self.side[self.head].is_clean[side] = true;
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
