use arrayvec::ArrayVec;
use shakmaty::{Board, Chess, Move, MoveList, Position, Role};

use crate::{
    ext::ScoredMove,
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

type ScoredMoveList = ArrayVec<ScoredMove, 270>;

pub struct Movepick<'a> {
    pos: &'a Chess,
    moves: ScoredMoveList,
    ptr: usize,
    pv: Option<Move>,
    stage: Stage,
    captures_end: usize,
    bad_capture_len: usize,
    bad_quiet_len: usize,
}

// [bad_captures, good_captures, bad_quiets, good_quiets]
//                             ^ capture_end
// ___________ <- bad_capture_len
//                              ___________ <- bad_quiet_len

impl<'a> Movepick<'a> {
    pub fn new_negamax(pos: &'a Chess, pv: Option<Move>) -> Self {
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

    pub fn is_quiet(m: &Move) -> bool {
        // a quiet move is not a capture and not a queen promotion
        let is_queen_promotion = m
            .promotion()
            .map(|role| role == Role::Queen)
            .unwrap_or(false);

        !m.is_capture() && !is_queen_promotion
    }

    pub fn get_captured(capture_move: &Move) -> usize {
        capture_move
            .capture()
            .map(|role| role as usize)
            .unwrap_or(NONE_PIECE_INDEX)
    }

    /// Sort by descending
    pub fn sort_moves(&mut self, start: usize, end: usize) {
        for i in (start + 1)..end {
            let temp = self.moves[i];
            let mut j = i - 1;
            while j >= start && self.moves[j].score < temp.score {
                self.moves[j + 1] = self.moves[j];
                j -= 1;
            }
            self.moves[j + 1] = temp;
        }
    }

    pub fn pick<F>(&mut self, end: usize, mut filter: F) -> Option<ScoredMove>
    where
        F: FnMut(&mut ScoredMoveList, usize) -> bool,
    {
        while self.ptr < end {
            let ok = filter(&mut self.moves, self.ptr);
            self.ptr += 1;
            if ok {
                return Some(self.moves[self.ptr - 1]);
            }
        }

        None
    }

    pub fn next_move(&mut self) -> Option<ScoredMove> {
        loop {
            match self.stage {
                Stage::Pv => {
                    self.stage = Stage::CaptureInit;
                    match self.pv {
                        None => {}
                        Some(pv) => {
                            return Some(ScoredMove::from_move(pv));
                        }
                    }
                }
                Stage::CaptureInit => {
                    // generate captures into [moves]
                    let mut captures = self.pos.legal_moves();
                    captures.retain(|m| !Self::is_quiet(m));

                    for i in 0..captures.len() {
                        let m = captures[i];

                        if self.pv.map(|pv| pv == m).unwrap_or(false) {
                            continue;
                        }

                        let mut scores_move = ScoredMove::from_move(m);

                        // mvv-lva
                        scores_move.score = PIECE_VALUE[Self::get_captured(&m)]
                            - PIECE_VALUE[m.role() as usize - 1];

                        self.moves.push(scores_move);
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

                    match next_move {
                        None => {
                            self.stage = Stage::QuietInit;
                        }
                        Some(next_move) => {
                            return Some(next_move);
                        }
                    }
                }
                Stage::QuietInit => {
                    // generate quiets into [moves]
                    let mut quiets = self.pos.legal_moves();
                    quiets.retain(|m| Self::is_quiet(m));

                    for i in 0..quiets.len() {
                        let m = quiets[i];

                        if self.pv.map(|pv| pv == m).unwrap_or(false) {
                            continue;
                        }

                        // TODO: killer moves

                        let mut scores_move = ScoredMove::from_move(m);

                        // TODO: history heuristic

                        self.moves.push(scores_move);
                    }

                    // [...captures; quiets]
                    // [bad_captures; good_captures; bad_quiets; good_quiet]
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

                    match next_move {
                        None => {
                            self.ptr = 0;
                            self.stage = Stage::BadCapture;
                        }
                        Some(next_move) => {
                            return Some(next_move);
                        }
                    }
                }
                Stage::BadCapture => {
                    let next_move = self.pick(self.bad_capture_len, |_moves, _i| true);

                    match next_move {
                        None => {
                            self.ptr = self.captures_end;
                            self.stage = Stage::BadQuiet;
                        }
                        Some(next_move) => {
                            return Some(next_move);
                        }
                    }
                }
                Stage::BadQuiet => {
                    let next_move =
                        self.pick(self.bad_quiet_len + self.captures_end, |_moves, _i| true);

                    match next_move {
                        None => {
                            return None;
                        }
                        Some(next_move) => {
                            return Some(next_move);
                        }
                    }
                }
            }
        }
    }
}
