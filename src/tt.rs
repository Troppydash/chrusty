use std::ptr::null_mut;

use cozy_chess::Move;

use crate::{
    ext::ExtMove,
    param::{UNINIT_DEPTH, VALUE_CHECKMATE, VALUE_NONE, is_valid},
};

const AGE_SIZE: usize = 5;
const MAX_AGE: u8 = 1 << AGE_SIZE;
const FLAG_NONE: u8 = 0;
const FLAG_ALPHA: u8 = 1;
const FLAG_BETA: u8 = 2;
const FLAG_EXACT: u8 = 3;

// can [value] derived using [flag] cutoff given [alpha, beta]
pub fn get_can_use(value: i16, flag: u8, alpha: i16, beta: i16) -> bool {
    if flag == FLAG_EXACT {
        return true;
    }

    if flag == FLAG_ALPHA {
        return value <= alpha;
    }

    if flag == FLAG_BETA {
        return value >= beta;
    }

    return false;
}

fn key_matches(key: u64, hash: u16) -> bool {
    key as u16 == hash
}

pub struct EntryValue {
    hit: bool,
    can_use: bool,
    pv: Move,
    depth: i8,
    static_score: i16,
    score: i16,
    is_pv: bool,
    flag: u8,
}

#[derive(Clone, Copy)]
pub struct Entry {
    hash: u16,
    pv: u16,
    depth: i8,
    static_score: i16,
    score: i16,
    mask: u8, // pv|flag|age
}

impl Entry {
    fn new() -> Self {
        Self {
            hash: 0,
            pv: 0,
            depth: UNINIT_DEPTH,
            static_score: VALUE_NONE,
            score: VALUE_NONE,
            mask: 0,
        }
    }

    fn get_flag(&self) -> u8 {
        (self.mask >> AGE_SIZE) & 0b11
    }

    fn get_age(&self) -> u8 {
        self.mask & ((1 << AGE_SIZE) - 1)
    }

    fn get_pv(&self) -> bool {
        (self.mask >> (AGE_SIZE + 2)) == 1
    }

    fn set_mask(&mut self, flag: u8, age: u8, pv: bool) {
        self.mask = age | (flag << AGE_SIZE) | (pv as u8) << (AGE_SIZE + 2);
    }

    pub fn get(&self, key: u64, ply: i8, depth: i8, alpha: i16, beta: i16) -> EntryValue {
        if key_matches(key, self.hash) && self.depth != UNINIT_DEPTH {
            let mut adjusted_score = VALUE_NONE;
            let mut can_use = false;

            if is_valid(self.score) {
                adjusted_score = self.score;

                if adjusted_score > VALUE_CHECKMATE {
                    adjusted_score -= ply as i16;
                } else if adjusted_score < -VALUE_CHECKMATE {
                    adjusted_score += ply as i16;
                }

                if self.depth >= depth {
                    let flag = self.get_flag();
                    can_use = get_can_use(adjusted_score, flag, alpha, beta);
                }
            }

            return EntryValue {
                hit: true,
                can_use,
                pv: Move::from_bits(self.pv),
                depth: self.depth,
                static_score: self.static_score,
                score: adjusted_score,
                is_pv: self.get_pv(),
                flag: self.get_flag(),
            };
        }

        return EntryValue {
            hit: false,
            can_use: false,
            pv: Move::NULL_MOVE,
            depth: UNINIT_DEPTH,
            static_score: VALUE_NONE,
            score: VALUE_NONE,
            is_pv: false,
            flag: FLAG_NONE,
        };
    }

    pub fn set(
        &mut self,
        key: u64,
        pv: &Move,
        ply: i8,
        depth: i8,
        flag: u8,
        score: i16,
        static_score: i16,
        is_pv: bool,
        age: u8,
    ) {
        if self.pv != Move::NULL_MOVE_BITS || !key_matches(key, self.hash) {
            self.pv = pv.to_bits();
        }

        let age_diff = (MAX_AGE + age - self.get_age()) % MAX_AGE;
        if flag == FLAG_EXACT
            || !key_matches(key, self.hash)
            || depth + 4 + 2 * (is_pv as i8) > self.depth
            || age_diff >= 1
        {
            self.hash = key as u16;
            self.depth = depth;
            self.static_score = static_score;

            self.score = score;
            if is_valid(score) {
                self.score = if score > VALUE_CHECKMATE {
                    score + ply as i16
                } else if score < -VALUE_CHECKMATE {
                    score - ply as i16
                } else {
                    score
                }
            }

            self.set_mask(flag, age, is_pv);
        }
    }

    fn clear(&mut self) {
        self.hash = 0;
        self.depth = UNINIT_DEPTH;
    }
}

#[derive(Clone, Copy)]
#[repr(align(32))]
pub struct Bucket {
    values: [Entry; 3],
}

impl Bucket {
    fn new() -> Self {
        Self {
            values: [Entry::new(); 3],
        }
    }

    /// Returns (reader, writer)
    fn get(&mut self, key: u64, age: u8) -> (Entry, &mut Entry) {
        for i in 0..3 {
            if key_matches(key, self.values[i].hash) {
                return (self.values[i].clone(), &mut self.values[i]);
            }
        }

        // try to find least bad
        let mut best_slot = 0;
        for i in 1..3 {
            let best_slot_score = self.values[best_slot].depth
                - ((MAX_AGE + age - self.values[best_slot].get_age()) % MAX_AGE) as i8;
            let slot_score =
                self.values[i].depth - ((MAX_AGE + age - self.values[i].get_age()) % MAX_AGE) as i8;
            if slot_score < best_slot_score {
                best_slot = i;
            }
        }

        (self.values[best_slot].clone(), &mut self.values[best_slot])
    }

    fn clear(&mut self) {
        for e in self.values.iter_mut() {
            e.clear();
        }
    }
}

// TODO: support resizing tt
pub struct Table {
    buckets: Vec<Bucket>,
    age: u8,
}

impl Table {
    pub fn new(size_in_mbytes: usize) -> Self {
        let buckets = size_in_mbytes * 1024 * 1024 / std::mem::size_of::<Bucket>();
        Self {
            buckets: vec![Bucket::new(); buckets],
            age: 0,
        }
    }

    pub fn clear(&mut self) {
        self.age = 0;
        for bucket in self.buckets.iter_mut() {
            bucket.clear();
        }
    }

    pub fn next_search(&mut self) {
        self.age = (self.age + 1) % MAX_AGE;
    }

    pub fn get(&mut self, key: u64) -> (Entry, &mut Entry) {
        let index = ((key as u128 * self.buckets.len() as u128) >> 64) as usize;
        self.buckets[index].get(key, self.age)
    }

    pub fn prefetch(&self, key: u64) {
        todo!()
    }
}

pub struct TablePtr(pub *mut Table);
impl TablePtr {
    pub const NULL_PTR: TablePtr = TablePtr(null_mut());

    pub fn from_table(table: &mut Table) -> TablePtr {
        TablePtr(table as *mut Table)
    }

    pub fn get(&mut self) -> &mut Table {
        assert!(!self.0.is_null());
        unsafe { &mut *self.0 }
    }
}
unsafe impl Send for TablePtr {}
unsafe impl Sync for TablePtr {}
