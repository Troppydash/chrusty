use crate::param::*;

pub struct Heuristic {
    // lmr[move_count][depth]
    lmr: [[i8; LMR_DEPTH]; LMR_MOVE_COUNT], // TODO: history
}

impl Heuristic {
    pub fn new() -> Self {
        let mut lmr = [[0; LMR_DEPTH]; LMR_MOVE_COUNT];
        for move_count in 0..LMR_MOVE_COUNT {
            for depth in 0..LMR_DEPTH {
                if move_count <= 1 || depth <= 1 {
                    lmr[move_count][depth] = 0;
                } else {
                    lmr[move_count][depth] =
                        (0.99 + f32::ln(move_count as f32) * f32::ln(depth as f32) / 3.14) as i8;
                }
            }
        }

        Self { lmr }
    }

    pub fn clear(&mut self) {

    }

    pub fn get_lmr(&self, move_count: usize, depth: i8) -> i8 {
        assert!(depth >= 0);
        self.lmr[move_count.min(LMR_MOVE_COUNT - 1)][(depth as usize).min(LMR_DEPTH - 1)]
    }
}
