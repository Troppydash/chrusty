use cozy_chess::{Board, Move};

use crate::{
    ext::{ColoredPiece, ExtBoard, ExtMove, MoveList, index_with_option},
    param::*,
};

#[derive(Clone, Copy, Debug)]
pub struct History<const LIMIT: i16> {
    value: i16,
}

impl<const LIMIT: i16> History<LIMIT> {
    fn new() -> History<LIMIT> {
        Self { value: 0 }
    }

    pub fn add(&mut self, bonus: i16) {
        let clamped = bonus.clamp(-LIMIT, LIMIT) as i32;
        self.value =
            (self.value as i32 + clamped - self.value as i32 * clamped.abs() / LIMIT as i32) as i16;
    }

    pub fn get(&self) -> i16 {
        self.value
    }
}

pub const CORR_LIMIT: i16 = 1024;
type MainHistory = History<20000>;
type CaptureHistory = History<20000>;
type PawnHistory = History<20000>;
pub type PawnCorr = History<CORR_LIMIT>;
pub const NUM_KILLERS: usize = 2;
pub const LOW_PLY: usize = 6;
pub const PAWN_HASH: usize = 1 << 14;

// TODO: cont hist

pub struct Heuristic {
    // lmr[move_count][depth]
    lmr: Box<[[i8; LMR_DEPTH]; LMR_MOVE_COUNT]>,
    // history heuristic [side][from][to]
    main_history: Box<[[[MainHistory; 64]; 64]; 2]>,
    // capture history [colored_piece][to][captured_piece]
    capture_history: Box<[[[CaptureHistory; 6]; 64]; 12]>,
    // killer moves [ply][n]
    killer_moves: Box<[[Move; NUM_KILLERS]; MAX_DEPTH as usize]>,
    // countermove [colored_piece][to]
    counter: Box<[[Move; 64]; 12]>,
    // lowply heuristic [ply][side][from][to]
    low_ply: Box<[[[[MainHistory; 64]; 64]; 2]; LOW_PLY]>,
    // pawn [hash][colored_piece][to]
    pawn: Box<[[[PawnHistory; 64]; 12]; PAWN_HASH]>,
    // pawn corrhist [hash][stm]
    pawn_corrhist: Box<[[PawnCorr; 2]; PAWN_HASH]>,
    // colored pawn corrhist [hash][stm]
    white_corrhist: Box<[[PawnCorr; 2]; PAWN_HASH]>,
    black_corrhist: Box<[[PawnCorr; 2]; PAWN_HASH]>,
    // major corrhist [hash][stm]
    major_corrhist: Box<[[PawnCorr; 2]; PAWN_HASH]>,
    minor_corrhist: Box<[[PawnCorr; 2]; PAWN_HASH]>,
    // cont corrhist [colored_piece][to][colored_piece][to]
    cont_corrhist: Box<[[[[PawnCorr; 64]; 12]; 64]; 13]>,
}

impl Heuristic {
    pub fn new() -> Self {
        let mut lmr = Box::new([[0; LMR_DEPTH]; LMR_MOVE_COUNT]);
        for move_count in 0..LMR_MOVE_COUNT {
            for depth in 0..LMR_DEPTH {
                if move_count <= 1 || depth <= 1 {
                    lmr[move_count][depth] = 1;
                } else {
                    lmr[move_count][depth] =
                        (0.99 + f32::ln(move_count as f32) * f32::ln(depth as f32) / 3.14) as i8;
                }
            }
        }

        let main_history = Box::new([[[MainHistory::new(); 64]; 64]; 2]);
        let capture_history = Box::new([[[CaptureHistory::new(); 6]; 64]; 12]);
        let killer_moves = Box::new([[Move::NULL_MOVE; NUM_KILLERS]; MAX_DEPTH as usize]);
        let counter = Box::new([[Move::NULL_MOVE; 64]; 12]);
        let low_ply = vec![[[[MainHistory::new(); 64]; 64]; 2]; LOW_PLY]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        let pawn = vec![[[PawnHistory::new(); 64]; 12]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();

        let pawn_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();

        let white_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();

        let black_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();

        let major_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();

        let minor_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();

        let cont_corrhist = vec![[[[PawnCorr::new(); 64]; 12]; 64]; 13]
            .into_boxed_slice()
            .try_into()
            .unwrap();

        Self {
            lmr,
            main_history,
            capture_history,
            killer_moves,
            counter,
            low_ply,
            pawn,
            pawn_corrhist,
            white_corrhist,
            black_corrhist,
            major_corrhist,
            minor_corrhist,
            cont_corrhist,
        }
    }

    pub fn next_search(&mut self) {
        self.killer_moves = Box::new([[Move::NULL_MOVE; NUM_KILLERS]; MAX_DEPTH as usize]);
    }

    pub fn clear(&mut self) {
        self.main_history = Box::new([[[MainHistory::new(); 64]; 64]; 2]);
        self.capture_history = Box::new([[[CaptureHistory::new(); 6]; 64]; 12]);
        self.killer_moves = Box::new([[Move::NULL_MOVE; NUM_KILLERS]; MAX_DEPTH as usize]);
        self.counter = Box::new([[Move::NULL_MOVE; 64]; 12]);
        self.low_ply = vec![[[[MainHistory::new(); 64]; 64]; 2]; LOW_PLY]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        self.pawn = vec![[[PawnHistory::new(); 64]; 12]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        self.pawn_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        self.white_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        self.black_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        self.major_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        self.minor_corrhist = vec![[PawnCorr::new(); 2]; PAWN_HASH]
            .into_boxed_slice()
            .try_into()
            .unwrap();
        self.cont_corrhist = vec![[[[PawnCorr::new(); 64]; 12]; 64]; 13]
            .into_boxed_slice()
            .try_into()
            .unwrap();
    }

    pub fn get_lmr(&self, move_count: usize, depth: i8) -> i8 {
        assert!(depth >= 0);
        self.lmr[move_count.min(LMR_MOVE_COUNT - 1)][(depth as usize).min(LMR_DEPTH - 1)]
    }

    pub fn get_main_history(&self, pos: &Board, m: Move) -> &MainHistory {
        &self.main_history[pos.side_to_move() as usize][m.from as usize][m.to as usize]
    }

    fn get_main_history_mut(&mut self, pos: &Board, m: Move) -> &mut MainHistory {
        &mut self.main_history[pos.side_to_move() as usize][m.from as usize][m.to as usize]
    }

    pub fn get_capture_history(&self, pos: &Board, m: Move) -> &MainHistory {
        debug_assert!(!pos.is_quiet(m));

        &self.capture_history[pos.color_piece_on(m.from).unwrap().index()][m.to as usize]
            [pos.get_captured(m) as usize]
    }

    fn get_capture_history_mut(&mut self, pos: &Board, m: Move) -> &mut MainHistory {
        debug_assert!(!pos.is_quiet(m));

        &mut self.capture_history[pos.color_piece_on(m.from).unwrap().index()][m.to as usize]
            [pos.get_captured(m) as usize]
    }

    pub fn get_killers(&self, ply: i8) -> &[Move; NUM_KILLERS] {
        &self.killer_moves[ply as usize]
    }

    fn get_killers_mut(&mut self, ply: i8) -> &mut [Move; NUM_KILLERS] {
        &mut self.killer_moves[ply as usize]
    }

    pub fn get_low_ply(&self, pos: &Board, m: Move, ply: i8) -> &MainHistory {
        assert!((ply as usize) < LOW_PLY);
        &self.low_ply[ply as usize][pos.side_to_move() as usize][m.from as usize][m.to as usize]
    }

    fn get_low_ply_mut(&mut self, pos: &Board, m: Move, ply: i8) -> &mut MainHistory {
        assert!((ply as usize) < LOW_PLY);
        &mut self.low_ply[ply as usize][pos.side_to_move() as usize][m.from as usize][m.to as usize]
    }

    pub fn get_counter(&self, prev_move: Move, prev_piece: Option<ColoredPiece>) -> Move {
        if prev_move.is_null() {
            return Move::NULL_MOVE;
        }

        if let Some(colored_piece) = prev_piece {
            // [to] because previous move
            return self.counter[colored_piece.index()][prev_move.to as usize];
        } else {
            return Move::NULL_MOVE;
        }
    }

    pub fn get_counter_mut(
        &mut self,
        prev_move: Move,
        prev_piece: Option<ColoredPiece>,
    ) -> Option<&mut Move> {
        if prev_move.is_null() {
            return None;
        }

        if let Some(colored_piece) = prev_piece {
            // [to] because previous move
            return Some(&mut self.counter[colored_piece.index()][prev_move.to as usize]);
        } else {
            return None;
        }
    }

    fn get_pawn_mut(&mut self, pos: &Board, m: Move, pawn_key: u64) -> &mut PawnHistory {
        &mut self.pawn[pawn_key as usize % PAWN_HASH][pos.color_piece_on(m.from).unwrap().index()]
            [m.to as usize]
    }

    pub fn get_pawn(&self, pos: &Board, m: Move, pawn_key: u64) -> &PawnHistory {
        &self.pawn[pawn_key as usize % PAWN_HASH][pos.color_piece_on(m.from).unwrap().index()]
            [m.to as usize]
    }

    pub fn get_pawn_corrhist(&mut self, pos: &Board, pawn_key: u64) -> &mut PawnCorr {
        &mut self.pawn_corrhist[pawn_key as usize % PAWN_HASH][pos.side_to_move() as usize]
    }

    pub fn get_white_corrhist(&mut self, pos: &Board, key: u64) -> &mut PawnCorr {
        &mut self.white_corrhist[key as usize % PAWN_HASH][pos.side_to_move() as usize]
    }

    pub fn get_black_corrhist(&mut self, pos: &Board, key: u64) -> &mut PawnCorr {
        &mut self.black_corrhist[key as usize % PAWN_HASH][pos.side_to_move() as usize]
    }

    pub fn get_major_corrhist(&mut self, pos: &Board, key: u64) -> &mut PawnCorr {
        &mut self.major_corrhist[key as usize % PAWN_HASH][pos.side_to_move() as usize]
    }

    pub fn get_minor_corrhist(&mut self, pos: &Board, key: u64) -> &mut PawnCorr {
        &mut self.minor_corrhist[key as usize % PAWN_HASH][pos.side_to_move() as usize]
    }

    pub fn get_cont_corrhist_index(&mut self, pos: &Board, m: Move) -> (usize, usize) {
        (
            index_with_option(&pos.color_piece_on(m.from)),
            m.to as usize,
        )
    }

    pub fn get_cont_corrhist(&mut self, idx: (usize, usize)) -> &mut [[PawnCorr; 64]; 12] {
        &mut self.cont_corrhist[idx.0][idx.1]
    }

    pub fn update_history(
        &mut self,
        pos: &Board,
        depth: i8,
        div: i32,
        ply: i8,
        best_move: Move,
        prev_move: Move,
        prev_piece: Option<ColoredPiece>,
        pawn_key: u64,
        captures: &MoveList,
        quiets: &MoveList,
    ) {
        assert!(!best_move.is_null(), "best move null in history update");

        let bonus = (i32::min(180 * depth as i32 + 15, 2000) / div) as i16;
        let malus = (i32::min(190 * depth as i32 - 30, 2000) / div) as i16;

        if pos.is_quiet(best_move) {
            self.get_main_history_mut(pos, best_move).add(bonus);
            if ply < LOW_PLY as i8 {
                self.get_low_ply_mut(pos, best_move, ply).add(bonus);
            }
            self.get_pawn_mut(pos, best_move, pawn_key).add(bonus);

            for m in quiets.iter() {
                assert!(!m.is_null());
                self.get_main_history_mut(pos, *m).add(-malus);

                if ply < LOW_PLY as i8 {
                    self.get_low_ply_mut(pos, *m, ply).add(-malus);
                }

                self.get_pawn_mut(pos, *m, pawn_key).add(-malus);
            }

            let killers = self.get_killers_mut(ply);
            if killers[0] != best_move {
                killers[1] = killers[0];
            }
            killers[0] = best_move;

            if let Some(counter) = self.get_counter_mut(prev_move, prev_piece) {
                *counter = best_move;
            }
        } else {
            self.get_capture_history_mut(pos, best_move).add(bonus);
        }

        for m in captures.iter() {
            assert!(!m.is_null());
            self.get_capture_history_mut(pos, *m).add(-malus);
        }
    }
}
