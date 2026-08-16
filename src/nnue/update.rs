use arrayvec::ArrayVec;
use cozy_chess::Square;

use crate::ext::{ColoredPiece, MoveType};

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum UpdateType {
    Move,
    Capture,
    Castle,
}

#[derive(Copy, Clone)]
pub struct Update {
    pub king_sq: [Square; 2],
    pub add1: (Square, ColoredPiece),
    pub add2: (Square, ColoredPiece),
    pub sub1: (Square, ColoredPiece),
    pub sub2: (Square, ColoredPiece),
    pub update_type: UpdateType,
}

#[derive(Clone, Copy, Debug)]
pub struct ThreatDelta {
    pub p1: ColoredPiece,
    pub sq1: Square,
    pub p2: ColoredPiece,
    pub sq2: Square,
}

pub type ThreatDeltaUpdates = ArrayVec<ThreatDelta, 128>;

#[derive(Clone, Default)]
pub struct ThreatUpdate {
    pub adds: ThreatDeltaUpdates,
    pub subs: ThreatDeltaUpdates,
    pub move_type: MoveType,
}

impl ThreatUpdate {
    pub fn clear(&mut self) {
        self.move_type = MoveType::default();
        self.adds.clear();
        self.subs.clear();
    }
}
