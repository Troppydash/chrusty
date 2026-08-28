use std::{collections::HashSet, fmt::{Debug, Display}};

use arrayvec::ArrayVec;
use cozy_chess::{Color, Piece, Square};

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

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub struct ThreatDelta(u32);

impl Debug for ThreatDelta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.get())
    }
}
impl ThreatDelta {
    pub fn new(
        attacker: Piece,
        attacker_sq: Square,
        target: Piece,
        target_sq: Square,
        attacker_color: Color,
    ) -> Self {
        Self(
            ((attacker as usize)
                | ((attacker_sq as usize) << 3)
                | ((target as usize) << (3 + 6))
                | ((target_sq as usize) << (3 + 6 + 3))
                | (attacker_color as usize) << (3 + 6 + 3 + 6)) as u32,
        )
    }

    pub fn get(&self) -> (Piece, Square, Piece, Square, Color) {
        (
            Piece::ALL[(self.0 & 0b111) as usize],
            Square::ALL[((self.0 >> 3) & 0b111111) as usize],
            Piece::ALL[((self.0 >> (3 + 6)) & 0b111) as usize],
            Square::ALL[((self.0 >> (3 + 6 + 3)) & 0b111111) as usize],
            Color::ALL[(self.0 >> (3 + 6 + 3 + 6)) as usize],
        )
    }
}

pub type ThreatDeltaUpdates = ArrayVec<ThreatDelta, 96>;

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

    pub fn sort(&mut self) {
        self.adds.sort();
        self.subs.sort();

        let mut new_adds = ThreatDeltaUpdates::new();
        let mut new_subs = ThreatDeltaUpdates::new();

        let mut i = 0;
        let mut j = 0;
        while i < self.adds.len() && j < self.subs.len() {
            if self.adds[i].0 == self.subs[j].0 {
                i += 1;
                j += 1;
            } else if self.adds[i].0 < self.subs[j].0 {
                new_adds.push(self.adds[i]);
                i += 1;
            } else {
                new_subs.push(self.subs[j]);
                j += 1;
            }
        }

        while i < self.adds.len() {
            new_adds.push(self.adds[i]);
            i += 1;
        }

        while j < self.subs.len() {
            new_subs.push(self.subs[j]);
            j += 1;
        }

        self.adds = new_adds;
        self.subs = new_subs;
    }
}
