use crate::param::{UNINIT_DEPTH, VALUE_NONE};

const AGE_SIZE: usize = 5;
const FLAG_NONE: u8 = 0;
const FLAG_ALPHA: u8 = 1;
const FLAG_BETA: u8 = 2;
const FLAG_EXACT: u8 = 3;

pub fn get_flag(mask: u8) -> u8 {
    (mask >> AGE_SIZE) & 0b11
}

pub fn get_age(mask: u8) -> u8 {
    mask & ((1 << AGE_SIZE) - 1)
}

pub fn get_pv(mask: u8) -> bool {
    (mask >> (AGE_SIZE + 2)) == 1
}

// can [value] derived using [flag] cutoff given [alpha, beta]
pub fn can_use(value: i16, flag: u8, alpha: i16, beta: i16) -> bool {
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

pub struct EntryValue {
    hash: u16,
    pv: u16,
    depth: i8,
    static_score: i16,
    score: i16,
    is_pv: bool,
    flag: u8,
    age: u8,
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

    pub fn get(&self) -> EntryValue {
        todo!()
    }

    pub fn set(&mut self) {}

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

    fn get(&mut self, key: u64) -> &mut Entry {
        // try to find matching

        // try to find least bad

        &mut self.values[0]
    }

    fn clear(&mut self) {
        for e in self.values.iter_mut() {
            e.clear();
        }
    }
}

pub struct Table {
    buckets: Vec<Bucket>,
}

impl Table {
    pub fn new(size_in_mbytes: usize) -> Self {
        let buckets = size_in_mbytes * 1024 * 1024 / std::mem::size_of::<Bucket>();
        Self {
            buckets: vec![Bucket::new(); buckets],
        }
    }

    pub fn clear(&mut self) {
        for bucket in self.buckets.iter_mut() {
            bucket.clear();
        }
    }

    pub fn get(&mut self, key: u64) -> &mut Entry {
        let index = ((key as u128 * self.buckets.len() as u128) >> 64) as usize;
        self.buckets[index].get(key)
    }
}
