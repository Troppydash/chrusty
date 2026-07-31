use arrayvec::ArrayVec;
use cozy_chess::Move;

use crate::{
    ext::ExtMove,
    param::{MAX_DEPTH_USIZE, VALUE_NONE},
};

#[derive(Clone, Debug)]
pub struct PvList {
    moves: ArrayVec<Move, MAX_DEPTH_USIZE>,
}

impl PvList {
    pub fn new() -> Self {
        Self {
            moves: ArrayVec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.moves.clear();
    }

    pub fn set(&mut self, m: &Move, other: &PvList) {
        self.moves.clear();
        self.moves.push(*m);

        for m in other.moves.iter() {
            self.moves.push(*m);
        }
    }

    pub fn pv(&self) -> Move {
        assert!(self.moves.len() > 0);
        return self.moves[0];
    }

    pub fn get(&self, i: usize) -> Move {
        assert!(i < self.moves.len());
        return self.moves[i];
    }

    pub fn get_moves(&self) -> &ArrayVec<Move, MAX_DEPTH_USIZE> {
        &self.moves
    }

    pub fn len(&self) -> usize {
        self.moves.len()
    }
}
#[derive(Clone, Debug)]
pub struct SearchStack {
    pub ply: i8,
    pub m: Move,
    pub pv_list: PvList,
    pub adjusted_static: i16,
    pub tt_pv: bool,
    pub excluded: Move,
}

impl SearchStack {
    pub fn new() -> Self {
        Self {
            ply: 0,
            m: Move::NULL_MOVE,
            pv_list: PvList::new(),
            adjusted_static: VALUE_NONE,
            tt_pv: false,
            excluded: Move::NULL_MOVE,
        }
    }

    pub fn new_ply(ply: i8) -> Self {
        Self {
            ply,
            m: Move::NULL_MOVE,
            pv_list: PvList::new(),
            adjusted_static: VALUE_NONE,
            tt_pv: false,
            excluded: Move::NULL_MOVE,
        }
    }
}

pub struct KeyStack {
    pub keys: [u64; 1000],
    pub head: usize,
}

impl KeyStack {
    pub fn new() -> Self {
        Self {
            keys: [0; 1000],
            head: 0,
        }
    }
    pub fn push(&mut self, key: u64) {
        self.keys[self.head] = key;
        self.head += 1;
    }

    pub fn pop(&mut self) {
        self.head -= 1;
    }

    pub fn clear(&mut self) {
        self.head = 0;
    }
}
