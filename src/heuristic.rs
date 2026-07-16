use cozy_chess::{Board, Move};

use crate::{
    ext::{ExtBoard, ExtMove, MoveList},
    param::*,
};

#[derive(Clone, Copy)]
pub struct History<const LIMIT: i16> {
    value: i16,
}

impl<const LIMIT: i16> History<LIMIT> {
    fn new() -> History<LIMIT> {
        Self { value: 0 }
    }

    fn add(&mut self, value: i16) {
        let clamped = value.clamp(-LIMIT, LIMIT) as i32;
        self.value = (value as i32 + clamped - value as i32 * clamped.abs() / LIMIT as i32) as i16;
    }

    pub fn get(&self) -> i16 {
        self.value
    }
}

type MainHistory = History<20000>;
type CaptureHistory = History<20000>;
pub const NUM_KILLERS: usize = 2;

pub struct Heuristic {
    // lmr[move_count][depth]
    lmr: Box<[[i8; LMR_DEPTH]; LMR_MOVE_COUNT]>,
    // history heuristic [side][from][to]
    main_history: Box<[[[MainHistory; 64]; 64]; 2]>,
    // capture history [colored_piece][to][captured_piece]
    capture_history: Box<[[[CaptureHistory; 6]; 64]; 12]>,
    // killer moves [ply][n]
    killer_moves: Box<[[Move; NUM_KILLERS]; MAX_DEPTH as usize]>,
}

impl Heuristic {
    pub fn new() -> Self {
        let mut lmr = Box::new([[0; LMR_DEPTH]; LMR_MOVE_COUNT]);
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

        let main_history = Box::new([[[MainHistory::new(); 64]; 64]; 2]);
        let capture_history = Box::new([[[CaptureHistory::new(); 6]; 64]; 12]);
        let killer_moves = Box::new([[Move::NULL_MOVE; NUM_KILLERS]; MAX_DEPTH as usize]);

        Self {
            lmr,
            main_history,
            capture_history,
            killer_moves,
        }
    }

    pub fn clear(&mut self) {
        self.main_history = Box::new([[[MainHistory::new(); 64]; 64]; 2]);
        self.capture_history = Box::new([[[CaptureHistory::new(); 6]; 64]; 12]);
        self.killer_moves = Box::new([[Move::NULL_MOVE; NUM_KILLERS]; MAX_DEPTH as usize]);
    }

    pub fn get_lmr(&self, move_count: usize, depth: i8) -> i8 {
        assert!(depth >= 0);
        self.lmr[move_count.min(LMR_MOVE_COUNT - 1)][(depth as usize).min(LMR_DEPTH - 1)]
    }

    pub fn get_main_history(&self, pos: &Board, m: &Move) -> &MainHistory {
        &self.main_history[pos.side_to_move() as usize][m.from as usize][m.to as usize]
    }

    pub fn get_main_history_mut(&mut self, pos: &Board, m: &Move) -> &mut MainHistory {
        &mut self.main_history[pos.side_to_move() as usize][m.from as usize][m.to as usize]
    }

    pub fn get_capture_history(&self, pos: &Board, m: &Move) -> &MainHistory {
        assert!(!pos.is_quiet(m));

        &self.capture_history
            [pos.piece_on(m.from).unwrap() as usize + 6 * pos.side_to_move() as usize]
            [m.to as usize][pos.get_captured(m) as usize]
    }

    pub fn get_capture_history_mut(&mut self, pos: &Board, m: &Move) -> &mut MainHistory {
        assert!(!pos.is_quiet(m));

        &mut self.capture_history
            [pos.piece_on(m.from).unwrap() as usize + 6 * pos.side_to_move() as usize]
            [m.to as usize][pos.get_captured(m) as usize]
    }

    pub fn get_killers(&self, ply: i8) -> &[Move; NUM_KILLERS] {
        &self.killer_moves[ply as usize]
    }

    pub fn get_killers_mut(&mut self, ply: i8) -> &mut [Move; NUM_KILLERS] {
        &mut self.killer_moves[ply as usize]
    }

    pub fn update_history(
        &mut self,
        pos: &Board,
        depth: i8,
        ply: i8,
        best_move: &Move,
        captures: &MoveList,
        quiets: &MoveList,
    ) {
        assert!(!best_move.is_null(), "best move null in history update");

        let bonus = i32::min(180 * depth as i32 - 100, 1000) as i16;
        let malus = i32::min(180 * depth as i32 - 100, 1000) as i16;

        if pos.is_quiet(best_move) {
            self.get_main_history_mut(pos, best_move).add(bonus);

            for m in quiets.iter() {
                assert!(!m.is_null());
                self.get_main_history_mut(pos, m).add(-malus);
            }

            let killers = self.get_killers_mut(ply);
            if &killers[0] != best_move {
                killers[1] = killers[0];
            }
            killers[0] = *best_move;
        } else {
            self.get_capture_history_mut(pos, best_move).add(bonus);
        }

        for m in captures.iter() {
            assert!(!m.is_null());
            self.get_capture_history_mut(pos, m).add(-malus);
        }
    }
}
