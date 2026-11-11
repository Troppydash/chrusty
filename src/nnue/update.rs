use std::{arch::x86_64::*, fmt::Debug};

use arrayvec::ArrayVec;
use cozy_chess::{Color, Square};

use crate::{
    ext::ColoredPiece,
    nnue::{
        network::{HL, Network},
        ti::{self, MAX_TI_INPUTS_DELTA, TI_INPUTS},
    },
};

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

#[derive(Clone, Debug)]
pub struct ThreatRows {
    /// row indices into threat_weights by side
    pub rows: [ArrayVec<u32, MAX_TI_INPUTS_DELTA>; 2],
    /// flip mask by side
    flip: [u8; 2],
    /// address of threat_weights
    weights: usize,
}

impl Default for ThreatRows {
    fn default() -> Self {
        Self {
            rows: [ArrayVec::new(), ArrayVec::new()],
            flip: [0; 2],
            weights: 0,
        }
    }
}

impl ThreatRows {
    fn reset(&mut self, king_sq: [Square; 2], network: &Network) {
        self.rows[0].clear();
        self.rows[1].clear();
        self.flip = [
            Network::flip_mask(king_sq[0], Color::White) as u8,
            Network::flip_mask(king_sq[1], Color::Black) as u8,
        ];
        self.weights = network.threat_weights.as_ptr() as usize;
    }

    #[inline(always)]
    fn push_row(&mut self, side: usize, index: usize) {
        debug_assert!(index < TI_INPUTS);
        self.rows[side].push(index as u32);
        unsafe {
            let row = (self.weights as *const i8).add(index * HL);
            // every other line prefetch
            for line in (0..HL / 64).step_by(2) {
                _mm_prefetch::<_MM_HINT_T0>(row.add(line * 64));
            }
        }
    }

    /// Records `attacker` on `attacker_sq` attacking `target` on `target_sq`.
    #[inline(always)]
    pub fn push_threat(
        &mut self,
        attacker: ColoredPiece,
        attacker_sq: Square,
        target: ColoredPiece,
        target_sq: Square,
    ) {
        debug_assert!(ti::is_feature(attacker.piece, target.piece));
        for side in 0..2 {
            let flip = self.flip[side] as usize;
            let index = ti::threat_feature_index(
                side == attacker.color as usize,
                attacker.color != target.color,
                attacker.piece as usize,
                attacker_sq as usize ^ flip,
                target_sq as usize ^ flip,
                target.piece as usize,
            );
            self.push_row(side, index);
        }
    }

    /// Records `piece` on `square` being pinned to the king of `pinned_king_color`.
    #[inline(always)]
    pub fn push_pinned(&mut self, piece: ColoredPiece, square: Square, pinned_king_color: Color) {
        for side in 0..2 {
            let index = ti::pinned_feature_index(
                self.flip[side] as usize,
                side == pinned_king_color as usize,
                side == piece.color as usize,
                piece.piece as usize,
                square as usize,
            );
            self.push_row(side, index);
        }
    }

    /// Records a pair of pawns on the same or adjacent files.
    #[inline(always)]
    pub fn push_pp(&mut self, pawn1: (Color, Square), pawn2: (Color, Square)) {
        for side in 0..2 {
            let index = ti::pp_feature_index(
                self.flip[side] as usize,
                side != pawn1.0 as usize,
                pawn1.1 as usize,
                side != pawn2.0 as usize,
                pawn2.1 as usize,
            );
            self.push_row(side, index);
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct ThreatUpdate {
    pub adds: ThreatRows,
    pub subs: ThreatRows,
}

impl ThreatUpdate {
    pub fn reset(&mut self, king_sq: [Square; 2], network: &Network) {
        self.adds.reset(king_sq, network);
        self.subs.reset(king_sq, network);
    }
}
