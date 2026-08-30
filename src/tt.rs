use std::{
    alloc::{Layout, alloc_zeroed, dealloc},
    arch::x86_64::{_MM_HINT_T0, _MM_HINT_T1, _mm_prefetch},
    ptr::{NonNull, null_mut},
};

use cozy_chess::Move;

use crate::{
    ext::ExtMove,
    param::{UNINIT_DEPTH, VALUE_CHECKMATE, VALUE_NONE, is_valid},
};

const AGE_SIZE: usize = 5;
const MAX_AGE: u8 = 1 << AGE_SIZE;
pub const FLAG_NONE: u8 = 0;
pub const FLAG_ALPHA: u8 = 1;
pub const FLAG_BETA: u8 = 2;
pub const FLAG_EXACT: u8 = 3;

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

fn key_matches(key: u64, hash: u64) -> bool {
    key as u64 == hash
}

const NUM_ENTRIES: usize = 4;
pub struct EntryValue {
    pub hit: bool,
    pub can_use: bool,
    pub pv: Move,
    pub depth: i8,
    pub static_score: i16,
    pub score: i16,
    pub is_pv: bool,
    pub flag: u8,
}

#[derive(Clone, Copy)]
pub struct Entry {
    hash: u64,
    pv: u16,
    depth: i8,
    static_score: i16,
    score: i16,
    // pv_node|flag|age
    mask: u8,
}

impl Entry {
    fn new() -> Self {
        Self {
            hash: 1,
            pv: Move::NULL_MOVE_BITS,
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
        pv: Move,
        ply: i8,
        depth: i8,
        flag: u8,
        score: i16,
        static_score: i16,
        is_pv: bool,
        age: u8,
    ) {
        if pv.to_bits() != Move::NULL_MOVE_BITS || !key_matches(key, self.hash) {
            self.pv = pv.to_bits();
        }

        let age_diff = (MAX_AGE + age - self.get_age()) % MAX_AGE;
        if flag == FLAG_EXACT
            || !key_matches(key, self.hash)
            || depth + 4 + 2 * (is_pv as i8) > self.depth
            || age_diff >= 1
        {
            self.hash = key as u64;
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
        self.hash = 1;
        self.depth = UNINIT_DEPTH;
        self.pv = Move::NULL_MOVE_BITS;
        self.static_score = VALUE_NONE;
        self.score = VALUE_NONE;
        self.mask = 0;
    }
}

#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Bucket {
    values: [Entry; NUM_ENTRIES],
}

impl Bucket {
    fn new() -> Self {
        Self {
            values: [Entry::new(); NUM_ENTRIES],
        }
    }

    /// Returns (reader, writer)
    fn get(&mut self, key: u64, age: u8) -> (Entry, &mut Entry) {
        for i in 0..NUM_ENTRIES {
            if key_matches(key, self.values[i].hash) {
                return (self.values[i].clone(), &mut self.values[i]);
            }
        }

        // try to find least bad
        let mut best_slot = 0;
        for i in 1..NUM_ENTRIES {
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

const LARGE_PAGE: usize = 4096;

pub struct Table {
    buckets: NonNull<Bucket>,
    size: usize,
    age: u8,
}

impl Table {
    fn get_layout(size: usize) -> Layout {
        Layout::array::<Bucket>(size)
            .unwrap()
            .align_to(LARGE_PAGE)
            .unwrap()
    }

    pub fn new(size_in_mbytes: usize) -> Self {
        let size = (size_in_mbytes * 1024 * 1024 / std::mem::size_of::<Bucket>()).max(1);
        let layout = Self::get_layout(size);
        let buckets = unsafe {
            let ptr = alloc_zeroed(layout) as *mut Bucket;
            NonNull::new(ptr).expect(&format!("Failed to allocate tt with {}M", size_in_mbytes))
        };
        let mut table = Self {
            buckets,
            size,
            age: 0,
        };
        table.clear();
        table
    }

    fn index(&self, i: usize) -> &Bucket {
        unsafe { &*self.buckets.as_ptr().add(i) }
    }

    fn index_mut(&mut self, i: usize) -> &mut Bucket {
        unsafe { &mut *self.buckets.as_ptr().add(i) }
    }

    fn dealloc(&mut self) {
        let layout = Self::get_layout(self.size);
        unsafe {
            dealloc(self.buckets.as_ptr() as *mut u8, layout);
        }
    }

    pub fn clear(&mut self) {
        self.age = 0;
        for i in 0..self.size {
            self.index_mut(i).clear();
        }
    }

    pub fn next_search(&mut self) {
        self.age = (self.age + 1) % MAX_AGE;
    }

    pub fn get(&mut self, key: u64) -> (Entry, &mut Entry) {
        let index = ((key as u128 * self.size as u128) >> 64) as usize;
        let age = self.age;
        self.index_mut(index).get(key, age)
    }

    pub fn prefetch(&self, key: u64) {
        let index = ((key as u128 * self.size as u128) >> 64) as usize;
        unsafe {
            _mm_prefetch(self.buckets.as_ptr().add(index) as *const i8, _MM_HINT_T0);
        }
    }

    pub fn get_age(&self) -> u8 {
        self.age
    }

    pub fn resize(&mut self, size_in_mbytes: usize) {
        self.dealloc();
        self.size = (size_in_mbytes * 1024 * 1024 / std::mem::size_of::<Bucket>()).max(1);
        let layout = Self::get_layout(self.size);
        self.buckets = unsafe {
            let ptr = alloc_zeroed(layout) as *mut Bucket;
            NonNull::new(ptr).expect(&format!("Failed to allocate tt with {}M", size_in_mbytes))
        };
        self.age = 0;
        self.clear();
    }

    pub fn hashfull(&self) -> i64 {
        let mut count = 0i64;
        for i in 0..100 {
            for j in 0..NUM_ENTRIES {
                let entry = &self.index(i).values[j];
                if entry.get_age() == self.age && entry.depth > UNINIT_DEPTH {
                    count += 1;
                }
            }
        }

        count * 10 / NUM_ENTRIES as i64
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        self.dealloc();
    }
}

#[derive(Clone)]
pub struct TablePtr(pub *mut Table);
impl TablePtr {
    pub const NULL_PTR: TablePtr = TablePtr(null_mut());

    pub fn from_table(table: &mut Table) -> TablePtr {
        TablePtr(table as *mut Table)
    }

    pub fn get(&self) -> &mut Table {
        assert!(!self.0.is_null());
        unsafe { &mut *self.0 }
    }
}
unsafe impl Send for TablePtr {}
unsafe impl Sync for TablePtr {}
