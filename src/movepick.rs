use arrayvec::ArrayVec;
use shakmaty::{Board, Chess, Move, MoveList};

use crate::ext::ScoredMove;

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
}

impl<'a> Movepick<'a> {
    pub fn new_negamax(pos: &'a Chess, pv: Option<Move>) -> Self {
        Self {
            pos,
            moves: ScoredMoveList::new(),
            ptr: 0,
            pv,
            stage: Stage::Pv,
        }
    }

    // [..20]
    // pv  | capture | quiet |
    // 1.    2.        3.

    pub fn next_move(&mut self) -> Option<Move> {
        loop {
            match self.stage {
                Stage::Pv => {
                    self.stage = Stage::CaptureInit;
                    if self.pv.is_some() {
                        return self.pv;
                    }
                }
                Stage::CaptureInit => {}
                Stage::GoodCapture => {}
                Stage::QuietInit => {}
                Stage::GoodQuiet => {}
                Stage::BadCapture => {}
                Stage::BadQuiet => {}
            }
        }
    }
}
