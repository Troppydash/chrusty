use arrayvec::ArrayVec;
use shakmaty::{Board, Move, MoveList};

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
    pos: &'a Board,
    moves: ScoredMoveList,
    ptr: usize,
    pv: Option<Move>,
    stage: Stage,
}

impl<'a> Movepick<'a> {
    pub fn new_negamax(pos: &'a Board, pv: Option<Move>) -> Self {
        Self {
            pos,
            moves: ScoredMoveList::new(),
            ptr: 0,
            pv,
            stage: Stage::Pv,
        }
    }

    pub fn next_move(&mut self) -> Option<Move> {
        loop {
            match self.stage {
                Stage::Pv => {
                    // return pv move, stage++
                }
                Stage::CaptureInit => {}
                Stage::QuietInit => {}
                Stage::GoodCapture => {}
                Stage::GoodQuiet => {}
                Stage::BadCapture => {}
                Stage::BadQuiet => {}
            }
        }
    }
}
