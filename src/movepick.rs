use cozy_chess::{
    BitBoard, Board,
    Color::{Black, White},
    Move, Piece, Rank,
};

use crate::{
    ext::{ExtBoard, ExtMove, ScoredMove, ScoredMoveList},
    heuristic::Heuristic,
    param::{BAD_QUIET_SCORE, MVV_MULTIPLIER, PIECE_VALUE},
};

enum Stage {
    // negamax
    Pv = 0,
    CaptureInit,
    GoodCapture,
    QuietInit,
    GoodQuiet,
    BadCapture,
    BadQuiet,

    // qsearch
    QPv,
    QCaptureInit,
    QCapture,

    EPv,
    ECaptureInit,
    ECapture,
    EQuietInit,
    EQuiet,
}

pub struct Movepick {
    pos: Board,
    pv: Move,
    ply: i8,
    // this is needed to prevent a refcell which is expensive
    heuristic: *const Heuristic,

    // internal
    moves: ScoredMoveList,
    ptr: usize,
    stage: Stage,
    captures_end: usize,
    bad_capture_len: usize,
    bad_quiet_len: usize,
}

impl Movepick {
    pub fn new_negamax(pos: Board, pv: Move, ply: i8, heuristic: &Heuristic) -> Self {
        Self {
            pos,
            pv,
            ply,
            heuristic,
            moves: ScoredMoveList::new(),
            ptr: 0,
            stage: Stage::Pv,
            captures_end: 0,
            bad_capture_len: 0,
            bad_quiet_len: 0,
        }
    }

    pub fn new_qsearch(
        pos: Board,
        pv: Move,
        ply: i8,
        heuristic: &Heuristic,
        in_check: bool,
    ) -> Self {
        Self {
            pos,
            pv,
            ply,
            heuristic,
            moves: ScoredMoveList::new(),
            ptr: 0,
            stage: if in_check { Stage::EPv } else { Stage::QPv },
            captures_end: 0,
            bad_capture_len: 0,
            bad_quiet_len: 0,
        }
    }

    fn get_heuristic(&self) -> &Heuristic {
        unsafe { &*self.heuristic }
    }

    pub fn pick<F>(&mut self, end: usize, mut filter: F) -> ScoredMove
    where
        F: FnMut(&mut ScoredMoveList, usize) -> bool,
    {
        while self.ptr < end {
            let mut best_i = self.ptr;
            for i in (self.ptr + 1)..end {
                if self.moves[i].score > self.moves[best_i].score {
                    best_i = i;
                }
            }
            self.moves.swap(self.ptr, best_i);

            let ok = filter(&mut self.moves, self.ptr);
            self.ptr += 1;
            if ok {
                return self.moves[self.ptr - 1];
            }
        }

        ScoredMove::NULL_MOVE
    }

    /// generate captures into [moves]
    fn generate_captures(&mut self) {
        // CAPTURE, PROMOTION, ENPASSENT
        let opp = self.pos.colors(!self.pos.side_to_move());
        let ep_square = self.pos.ep_square();
        let promotion_pawns = self
            .pos
            .colored_pieces(self.pos.side_to_move(), Piece::Pawn)
            & match self.pos.side_to_move() {
                White => Rank::Seventh.bitboard(),
                Black => Rank::Second.bitboard(),
            };

        self.pos.generate_moves_for(promotion_pawns, |moves| {
            // capture promotions
            for t in moves.to & opp {
                for p in [Piece::Queen, Piece::Rook, Piece::Knight, Piece::Bishop] {
                    self.moves.push(ScoredMove::from_move(Move {
                        from: moves.from,
                        to: t,
                        promotion: Some(p),
                    }));
                }
            }

            // non capture promotions
            for t in moves.to & !opp {
                self.moves.push(ScoredMove::from_move(Move {
                    from: moves.from,
                    to: t,
                    promotion: Some(Piece::Queen),
                }));
            }

            false
        });

        self.pos.generate_moves_for(!promotion_pawns, |moves| {
            // add ep square if pawn
            let opp = if let Some(ep) = ep_square
                && moves.piece == Piece::Pawn
            {
                opp | BitBoard::from(ep)
            } else {
                opp
            };
            for t in moves.to & opp {
                self.moves.push(ScoredMove::from_move(Move {
                    from: moves.from,
                    to: t,
                    promotion: None,
                }));
            }

            false
        });
    }

    /// generate quiets into [moves]
    fn generate_quiets(&mut self) {
        let opp = self.pos.colors(!self.pos.side_to_move());
        let ep_square = self.pos.ep_square();
        let promotion_pawns = self
            .pos
            .colored_pieces(self.pos.side_to_move(), Piece::Pawn)
            & match self.pos.side_to_move() {
                White => Rank::Seventh.bitboard(),
                Black => Rank::Second.bitboard(),
            };

        self.pos.generate_moves_for(promotion_pawns, |moves| {
            // only non-capture non-queen promotions
            for t in moves.to & !opp {
                for p in [Piece::Rook, Piece::Knight, Piece::Bishop] {
                    self.moves.push(ScoredMove::from_move(Move {
                        from: moves.from,
                        to: t,
                        promotion: Some(p),
                    }));
                }
            }

            false
        });

        self.pos.generate_moves_for(!promotion_pawns, |moves| {
            // add ep square if pawn
            let opp = if let Some(ep) = ep_square
                && moves.piece == Piece::Pawn
            {
                opp | BitBoard::from(ep)
            } else {
                opp
            };
            for t in moves.to & !opp {
                self.moves.push(ScoredMove::from_move(Move {
                    from: moves.from,
                    to: t,
                    promotion: None,
                }));
            }

            false
        });
    }

    fn score_captures(&mut self) {
        let mut i = 0;
        while i < self.moves.len() {
            assert!(!self.pos.is_quiet(&self.moves[i].inner));
            if self.pv == self.moves[i].inner {
                self.moves.swap_remove(i);
                continue;
            }

            // mvv-lva
            let heuristic = self.get_heuristic();
            let mut score = heuristic
                .get_capture_history(&self.pos, &self.moves[i].inner)
                .get() as i32;

            score +=
                MVV_MULTIPLIER * PIECE_VALUE[self.pos.get_captured(&self.moves[i].inner) as usize];

            if self.moves[i].inner.promotion.is_some() {
                score += 10000;
            }

            self.moves[i].score = score;

            i += 1;
        }
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
                    self.generate_captures();
                    self.score_captures();

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
                    self.generate_quiets();

                    let mut i = self.captures_end;
                    while i < self.moves.len() {
                        assert!(
                            self.pos.is_quiet(&self.moves[i].inner),
                            "{} {}",
                            self.pos,
                            self.moves[i].inner
                        );

                        if self.pv == self.moves[i].inner {
                            self.moves.swap_remove(i);
                            continue;
                        }

                        let heuristic = self.get_heuristic();

                        let killers = heuristic.get_killers(self.ply);
                        if self.moves[i].inner == killers[0] {
                            self.moves[i].score = i32::MAX;
                            i += 1;
                            continue;
                        }
                        if self.moves[i].inner == killers[1] {
                            self.moves[i].score = i32::MAX - 1;
                            i += 1;
                            continue;
                        }

                        let score = heuristic
                            .get_main_history(&self.pos, &self.moves[i].inner)
                            .get() as i32;

                        self.moves[i].score = score;

                        i += 1;
                    }

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

                Stage::QPv => {
                    self.stage = Stage::QCaptureInit;
                    if !self.pv.is_null() {
                        return ScoredMove::from_move(self.pv);
                    }
                }
                Stage::QCaptureInit => {
                    self.generate_captures();
                    self.score_captures();

                    self.ptr = 0;
                    self.stage = Stage::QCapture;
                }
                Stage::QCapture => {
                    let next_move = self.pick(self.moves.len(), |_list, _i| {
                        // TODO: see
                        true
                    });
                    if !next_move.is_null() {
                        return next_move;
                    }

                    return ScoredMove::NULL_MOVE;
                }

                Stage::EPv => {
                    self.stage = Stage::ECaptureInit;
                    if !self.pv.is_null() {
                        return ScoredMove::from_move(self.pv);
                    }
                }
                Stage::ECaptureInit => {
                    self.generate_captures();
                    self.score_captures();

                    self.ptr = 0;
                    self.captures_end = self.moves.len();
                    self.stage = Stage::ECapture;
                }
                Stage::ECapture => {
                    let next_move = self.pick(self.moves.len(), |_list, _i| {
                        // TODO: see
                        true
                    });
                    if !next_move.is_null() {
                        return next_move;
                    }

                    self.stage = Stage::EQuietInit;
                }
                Stage::EQuietInit => {
                    self.generate_quiets();

                    let mut i = self.captures_end;
                    while i < self.moves.len() {
                        assert!(self.pos.is_quiet(&self.moves[i].inner));

                        if self.pv == self.moves[i].inner {
                            self.moves.swap_remove(i);
                            continue;
                        }

                        let heuristic = self.get_heuristic();

                        let killers = heuristic.get_killers(self.ply);
                        if self.moves[i].inner == killers[0] {
                            self.moves[i].score = i32::MAX;
                            i += 1;
                            continue;
                        }
                        if self.moves[i].inner == killers[1] {
                            self.moves[i].score = i32::MAX - 1;
                            i += 1;
                            continue;
                        }

                        let score = heuristic
                            .get_main_history(&self.pos, &self.moves[i].inner)
                            .get() as i32;
                        self.moves[i].score = score;

                        i += 1;
                    }

                    self.stage = Stage::EQuiet;
                }
                Stage::EQuiet => {
                    let next_move = self.pick(self.moves.len(), |_list, _i| true);
                    if !next_move.is_null() {
                        return next_move;
                    }

                    return ScoredMove::NULL_MOVE;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_movegen(fen: &str) {
        let pos = Board::from_fen(fen, false).unwrap();
        let mut total = vec![];
        pos.generate_moves(|moves| {
            for m in moves {
                total.push(m);
            }
            false
        });

        let heuristic = Heuristic::new();
        for mut mp in vec![
            Movepick::new_negamax(pos.clone(), Move::NULL_MOVE, 0, &heuristic),
            Movepick::new_qsearch(pos.clone(), Move::NULL_MOVE, 0, &heuristic, true),
        ] {
            let mut mp_moves = vec![];
            loop {
                let m = mp.next_move();
                if m.is_null() {
                    break;
                }

                mp_moves.push(m.inner);
            }

            total.sort_by(|a, b| a.to_bits().cmp(&b.to_bits()));
            mp_moves.sort_by(|a, b| a.to_bits().cmp(&b.to_bits()));
            assert!(
                total.len() == mp_moves.len(),
                "length mismatch, {}, {:?}, {:?}",
                fen,
                total,
                mp_moves
            );
            for (a, b) in total.iter().zip(&mp_moves) {
                assert!(a == b);
            }
        }
    }

    #[test]
    fn test_movegen() {
        let fens = vec![
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/N7/PPPPPPPP/R1BQKBNR b KQkq - 1 1",
            "rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b KQkq - 1 1",
            "r3r1k1/pp3pbp/1qp1b1p1/2B5/2BP4/Q1n2N2/P4PPP/3R1K1R w - - 4 18",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
            "3k4/3p4/8/K1P4r/8/8/8/8 b - - 0 1",
            "8/8/4k3/8/2p5/8/B2P2K1/8 w - - 0 1",
            "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1",
            "5k2/8/8/8/8/8/8/4K2R w K - 0 1",
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/4P3/3P1N2/PPP2PPP/RNBQKB1R b KQkq - 0 4",
            "r1b1k2r/pp1p1ppp/2n1pn2/q7/1bP1P3/2N2N2/PP1B1PPP/R2QKB1R w KQkq - 4 8",
            "r2q1rk1/pbpn1ppp/1p2pn2/3p4/1bPP4/2N1PN2/PPQB1PPP/R3KB1R w KQ - 2 9",
            "r1bqkb1r/pppp1ppp/2n5/4p3/2B1n3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 5",
            "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/2N5/PPPP1PPP/R1BQKBNR b KQkq - 1 3",
            "r1bqk2r/pppp1ppp/2n2n2/4p3/1b2P3/2NP1N2/PPP2PPP/R1BQKB1R w KQkq - 1 6",
            "r2qkb1r/ppp2ppp/2np1n2/4p3/4P1b1/2NP1N2/PPP2PPP/R1BQKB1R w KQkq - 2 6",
            "r1bq1rk1/pppn1ppp/4pn2/3p4/1bPP4/2N1PN2/PP1B1PPP/R2QKB1R w KQ - 3 7",
            "r2qk2r/pppnbppp/4pn2/3p1b2/3P4/2N1PN2/PPP1BPPP/R1BQ1RK1 w kq - 4 8",
            "r1bq1rk1/pppn1ppp/4pn2/3p4/1bPP4/2N1PN2/PP1B1PPP/R2QKB1R b KQ - 3 7",
            "r1bqkb1r/pppp1ppp/2n5/4p3/4n3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 5",
            "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/2N5/PPPP1PPP/R1BQKBNR w KQkq - 2 3",
            "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
        ];
        for fen in fens.iter() {
            diff_movegen(fen);
        }
    }
}
