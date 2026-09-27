use std::fmt::Debug;

use arrayvec::ArrayVec;
use cozy_chess::{Color, Square};

use crate::{ext::ColoredPiece, nnue::ti::MAX_TI_INPUTS};

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum UpdateType {
    Move,
    Capture,
    Castle,
}

#[derive(Clone)]
pub struct Update {
    pub king_sq: [Square; 2],
    pub add1: (Square, ColoredPiece),
    pub add2: (Square, ColoredPiece),
    pub sub1: (Square, ColoredPiece),
    pub sub2: (Square, ColoredPiece),
    pub update_type: UpdateType,
}

#[derive(Debug, Clone)]
pub struct ThreatDelta {
    pub attacker: ColoredPiece,
    pub attacker_sq: Square,
    pub target: ColoredPiece,
    pub target_sq: Square,
}

impl ThreatDelta {
    pub fn new(
        attacker: ColoredPiece,
        attacker_sq: Square,
        target: ColoredPiece,
        target_sq: Square,
    ) -> Self {
        Self {
            attacker,
            attacker_sq,
            target,
            target_sq,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PinnedDelta {
    pub piece: ColoredPiece,
    pub square: Square,
    pub pinned_king_color: Color,
}

#[derive(Debug, Clone)]
pub enum ThreatDeltaGeneral {
    ThreatDelta(ThreatDelta),
    PinnedDelta(PinnedDelta),
}

pub type ThreatDeltaUpdates = ArrayVec<ThreatDeltaGeneral, MAX_TI_INPUTS>;

#[derive(Clone, Default, Debug)]
pub struct ThreatUpdate {
    pub adds: ThreatDeltaUpdates,
    pub subs: ThreatDeltaUpdates,
}

impl ThreatUpdate {
    pub fn clear(&mut self) {
        self.adds.clear();
        self.subs.clear();
    }
}
