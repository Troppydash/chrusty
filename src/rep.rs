const REP_SIZE: usize = 1 << 15;
const UNSET: u64 = 1;

#[derive(Clone, Copy)]
#[repr(align(32))]
struct RepEntry {
    keys: [u64; 3],
    counts: [bool; 3],
}

impl RepEntry {
    fn new() -> Self {
        Self {
            keys: [UNSET; 3],
            counts: [false; 3],
        }
    }

    fn clear(&mut self) {
        self.keys = [UNSET; 3];
        self.counts = [false; 3];
    }

    fn add_history(&mut self, key: u64) {
        for i in 0..3 {
            if self.keys[i] == key {
                assert!(!self.counts[i]);
                self.counts[i] = true;
                return;
            }
        }

        for i in 0..3 {
            if self.keys[i] == UNSET {
                self.keys[i] = key;
                assert!(!self.counts[i]);
                return;
            }
        }

        // reaching here is bad
        assert!(false, "ran out of room");
    }

    fn add(&mut self, key: u64) {
        for i in 0..3 {
            if self.keys[i] == UNSET {
                self.keys[i] = key;
                assert!(!self.counts[i]);
                return;
            }
        }

        // reaching here is bad
        assert!(false, "ran out of room");
    }

    fn remove(&mut self, key: u64) {
        for i in 0..3 {
            if self.keys[i] == key {
                assert!(!self.counts[i]);
                self.keys[i] = UNSET;
                self.counts[i] = false;
                return;
            }
        }

        assert!(false, "not found");
    }

    fn check(&self, key: u64, count: bool) -> bool {
        for i in 0..3 {
            if self.keys[i] == key {
                return self.counts[i] as i8 >= count as i8;
            }
        }

        return false;
    }
}

pub struct RepTable {
    history: Box<[RepEntry]>,
    search: Box<[RepEntry]>,
}

impl RepTable {
    pub fn new() -> Self {
        let history = vec![RepEntry::new(); REP_SIZE].into_boxed_slice();
        let search = vec![RepEntry::new(); REP_SIZE].into_boxed_slice();
        Self { history, search }
    }

    pub fn add_history(&mut self, key: u64) {
        self.history[(key as usize) % REP_SIZE].add_history(key);
    }

    pub fn add(&mut self, key: u64) {
        self.search[(key as usize) % REP_SIZE].add(key);
    }

    pub fn remove(&mut self, key: u64) {
        self.search[(key as usize) % REP_SIZE].remove(key);
    }

    pub fn check(&self, key: u64) -> bool {
        let index = (key as usize) % REP_SIZE;

        // search is more likely so it is first
        return self.search[index].check(key, false) || self.history[index].check(key, true);
    }

    pub fn clear(&mut self) {
        for c in self.history.iter_mut() {
            c.clear();
        }

        for c in self.search.iter_mut() {
            c.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rep_add() {
        let mut rep = RepTable::new();
        rep.add(0);
        assert!(!rep.check(5));
        rep.add(1);
        assert!(!rep.check(5));
        rep.add(2);
        assert!(!rep.check(5));
        rep.add(3);
        assert!(rep.check(0));
    }

    #[test]
    fn test_rep_add_remove() {
        let mut rep = RepTable::new();
        rep.add(0);
        rep.add(1);
        rep.add(2);
        rep.remove(2);
        rep.remove(1);
        rep.add(5);
        assert!(rep.check(0));
    }
}
