use arrayvec::ArrayVec;
use cozy_chess::{
    Board,
    Color::{Black, White},
    Move, Piece, Rank,
};

use crate::{
    ext::{ExtBoard, ExtMove, ScoredMove, ScoredMoveList},
    param::{BAD_QUIET_SCORE, MVV_MULTIPLIER, NONE_PIECE_INDEX, PIECE_VALUE},
};

enum Stage {
    Pv = 0,
    CaptureInit,
    GoodCapture,
    QuietInit,
    GoodQuiet,
    BadCapture,
    BadQuiet,
}

pub struct Movepick<'a> {
    pos: &'a Board,
    moves: ScoredMoveList,
    ptr: usize,
    pv: Move,
    stage: Stage,
    captures_end: usize,
    bad_capture_len: usize,
    bad_quiet_len: usize,
}

impl<'a> Movepick<'a> {
    pub fn new_negamax(pos: &'a Board, pv: Move) -> Self {
        Self {
            pos,
            moves: ScoredMoveList::new(),
            ptr: 0,
            pv,
            stage: Stage::Pv,
            captures_end: 0,
            bad_capture_len: 0,
            bad_quiet_len: 0,
        }
    }

    pub fn is_quiet(pos: &Board, m: &Move) -> bool {
        // a quiet move is not a capture and not a queen promotion
        let is_queen_promotion = m
            .promotion
            .map(|piece| piece == Piece::Queen)
            .unwrap_or(false);

        !pos.is_capture(m) && !is_queen_promotion
    }

    /// Sort by descending
    pub fn sort_moves(&mut self, start: usize, end: usize) {
        // TODO: try selection sort
        for i in (start + 1)..end {
            let temp = self.moves[i];
            let mut j = i - 1;
            while j >= start && self.moves[j].score < temp.score {
                self.moves[j + 1] = self.moves[j];

                if j == 0 {
                    break;
                }
                j -= 1;
            }
            self.moves[j + 1] = temp;
        }
    }

    pub fn pick<F>(&mut self, end: usize, mut filter: F) -> ScoredMove
    where
        F: FnMut(&mut ScoredMoveList, usize) -> bool,
    {
        while self.ptr < end {
            let ok = filter(&mut self.moves, self.ptr);
            self.ptr += 1;
            if ok {
                return self.moves[self.ptr - 1];
            }
        }

        ScoredMove::NULL_MOVE
    }

    pub fn next_move(&mut self) -> ScoredMove {
        loop {
            match self.stage {
                Stage::Pv => {
                    self.stage = Stage::CaptureInit;
                    if !self.pv.is_null() {
                        return ScoredMove::from_move(self.pv);
                    }
                }
                Stage::CaptureInit => {
                    // generate captures into [moves]
                    let opp = self.pos.colors(!self.pos.side_to_move());
                    let promotion_pawns = self.pos.pieces(Piece::Pawn)
                        & match self.pos.side_to_move() {
                            White => Rank::Seventh.bitboard(),
                            Black => Rank::Second.bitboard(),
                        };
                    let non_promotions = self.pos.occupied() - promotion_pawns;

                    // captures
                    self.pos.generate_moves_for(non_promotions, |moves| {
                        for t in moves.to & opp {
                            self.moves.push(ScoredMove {
                                inner: Move {
                                    from: moves.from,
                                    to: t,
                                    promotion: None,
                                },
                                score: 0,
                            });
                        }
                        false
                    });

                    // capture promotions or queen promotions
                    self.pos.generate_moves_for(promotion_pawns, |moves| {
                        // captures
                        for t in moves.to & opp {
                            for p in [Piece::Queen, Piece::Rook, Piece::Knight, Piece::Bishop] {
                                self.moves.push(ScoredMove {
                                    inner: Move {
                                        from: moves.from,
                                        to: t,
                                        promotion: Some(p),
                                    },
                                    score: 0,
                                });
                            }
                        }

                        // non-captures
                        for t in moves.to & !opp {
                            self.moves.push(ScoredMove {
                                inner: Move {
                                    from: moves.from,
                                    to: t,
                                    promotion: Some(Piece::Queen),
                                },
                                score: 0,
                            });
                        }

                        false
                    });

                    let mut i = 0;
                    while i < self.moves.len() {
                        let scored_move = &mut self.moves[i];
                        if self.pv == scored_move.inner {
                            self.moves.swap_remove(i);
                            continue;
                        }

                        // mvv-lva
                        scored_move.score = MVV_MULTIPLIER
                            * PIECE_VALUE[self.pos.get_captured_index(&scored_move.inner)]
                            - PIECE_VALUE[self.pos.get_from_index(&scored_move.inner)];

                        i += 1;
                    }

                    self.sort_moves(0, self.moves.len());
                    self.ptr = 0;
                    self.captures_end = self.moves.len();
                    self.stage = Stage::GoodCapture;
                }
                Stage::GoodCapture => {
                    let next_move = self.pick(self.moves.len(), |_list, _i| {
                        // TODO: see
                        true
                    });
                    self.bad_capture_len = 0;

                    if !next_move.is_null() {
                        return next_move;
                    }

                    self.stage = Stage::QuietInit;
                }
                Stage::QuietInit => {
                    // generate quiets into [moves]
                    let opp = self.pos.colors(!self.pos.side_to_move());
                    let promotion_pawns = self.pos.pieces(Piece::Pawn)
                        & match self.pos.side_to_move() {
                            White => Rank::Seventh.bitboard(),
                            Black => Rank::Second.bitboard(),
                        };
                    let non_promotions = self.pos.occupied() - promotion_pawns;

                    // non-captures
                    self.pos.generate_moves_for(non_promotions, |moves| {
                        for t in moves.to & !opp {
                            self.moves.push(ScoredMove {
                                inner: Move {
                                    from: moves.from,
                                    to: t,
                                    promotion: None,
                                },
                                score: 0,
                            });
                        }

                        false
                    });

                    // non-capture non-queen promotions
                    self.pos.generate_moves_for(promotion_pawns, |moves| {
                        for t in moves.to & !opp {
                            for p in [Piece::Rook, Piece::Knight, Piece::Bishop] {
                                self.moves.push(ScoredMove {
                                    inner: Move {
                                        from: moves.from,
                                        to: t,
                                        promotion: Some(p),
                                    },
                                    score: 0,
                                });
                            }
                        }

                        false
                    });

                    let mut i = self.captures_end;
                    while i < self.moves.len() {
                        let scored_move = &mut self.moves[i];
                        if self.pv == scored_move.inner {
                            self.moves.swap_remove(i);
                            continue;
                        }

                        // TODO: killer moves
                        // TODO: history heuristic

                        i += 1;
                    }

                    self.sort_moves(self.captures_end, self.moves.len());
                    self.ptr = self.captures_end;
                    self.stage = Stage::GoodQuiet;
                }
                Stage::GoodQuiet => {
                    let mut bad_quiet_len = self.bad_quiet_len;
                    let captures_end = self.captures_end;
                    let next_move = self.pick(self.moves.len(), |moves, i| {
                        if moves[i].score < BAD_QUIET_SCORE {
                            moves.swap(captures_end + bad_quiet_len, i);
                            bad_quiet_len += 1;
                            return false;
                        }

                        return true;
                    });
                    self.bad_quiet_len = bad_quiet_len;

                    if !next_move.is_null() {
                        return next_move;
                    }

                    self.ptr = 0;
                    self.stage = Stage::BadCapture;
                }
                Stage::BadCapture => {
                    let next_move = self.pick(self.bad_capture_len, |_moves, _i| true);
                    if !next_move.is_null() {
                        return next_move;
                    }

                    self.ptr = self.captures_end;
                    self.stage = Stage::BadQuiet;
                }
                Stage::BadQuiet => {
                    let next_move =
                        self.pick(self.bad_quiet_len + self.captures_end, |_moves, _i| true);
                    if !next_move.is_null() {
                        return next_move;
                    }
                    return ScoredMove::NULL_MOVE;
                }
            }
        }
    }
}
