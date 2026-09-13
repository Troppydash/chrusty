use std::{arch::x86_64::*, mem::MaybeUninit};

use cozy_chess::{Board, Color::White, Move, Square};

use crate::{
    ext::ExtBoard, nnue::{
        halfka::HalfKA, network::{Aligned, Aligned16, CM, FT_SHIFT, HL, L1, L2, Network, Permute, QA, QB, RawNetwork, SCALE}, threats::Threats,
    }, param::{MAX_DEPTH, MAX_DEPTH_USIZE},
};

mod halfka;
pub mod network;
mod threats;
mod ti;
mod update;

#[derive(Clone)]
struct Stack {
    valid: bool,
    ft: Aligned<u8, HL>,
    idx_n: usize,
    idx: Aligned<u16, { HL / 4 }>,
}

impl Stack {
    pub fn new() -> Self {
        Self {
            valid: false,
            ft: Aligned::<u8, HL>::zeroed(),
            idx_n: 0,
            idx: Aligned::zeroed(),
        }
    }
}

#[derive(Clone)]
struct CMCacheBucket {
    key: u64,
    policy_from: [f32; CM],
    policy_to: [f32; CM],
}

impl CMCacheBucket {
    fn new() -> Self {
        Self {
            key: 0,
            policy_from: [0.; CM],
            policy_to: [0.; CM],
        }
    }
}

const CM_SIZE: usize = 1 << 12;

struct CMCache {
    buckets: Box<[CMCacheBucket]>,
    size: usize,
}

impl CMCache {
    pub fn new() -> Self {
        let size = CM_SIZE;
        let buckets = vec![CMCacheBucket::new(); size].into_boxed_slice();
        Self { buckets, size }
    }

    fn index(&self, key: u64) -> usize {
        key as usize % CM_SIZE
    }

    pub fn set(&mut self, key: u64, policy_from: &[f32; CM], policy_to: &[f32; CM]) {
        let entry = &mut self.buckets[self.index(key)];
        entry.key = key;
        entry.policy_from = *policy_from;
        entry.policy_to = *policy_to;
    }

    pub fn get(&self, key: u64) -> Option<([f32; CM], [f32; CM])> {
        let entry = &self.buckets[self.index(key)];

        if entry.key != key {
            return None;
        }

        Some((entry.policy_from, entry.policy_to))
    }

    fn clear(&mut self) {
        for i in 0..self.size {
            self.buckets[i].key = 0;
        }
    }
}

pub struct NNUE {
    network: Box<Network>,
    halfka: HalfKA,
    threats: Threats,
    nnz_table: [Aligned16; 256],
    // this is just a temp cache
    head: usize,
    stack: Box<[Stack]>,
    // cache: CMCache,
}

impl NNUE {
    // const FT_SHIFT_SCALE: f32 = QA as f32 / (1 << FT_SHIFT) as f32;
    const FT_TUNE : f32 = 1.0;
    const DIVISOR: f32 = Self::FT_TUNE / ((1 << FT_SHIFT) as f32 * (QB as f32));

    pub fn head(&self) -> (usize, usize, usize) {
        (self.head, self.halfka.head, self.threats.head)
    }

    pub fn build(permute: &Permute) -> Self {
        let mut raw = RawNetwork::load();
        raw.permute(permute);

        // nnz_table[bits][i] = ith bit in bits offset
        let mut nnz_table: [Aligned16; 256] = [Aligned16([0u16; 8]); 256];
        for i in 0..256 {
            let mut j = 0;
            let mut bits = i as u8;
            while bits > 0 {
                let lsb = bits.trailing_zeros();
                nnz_table[i][j] = lsb as u16;
                bits &= bits - 1;
                j += 1;
            }
        }

        let mut net = Self {
            network: Network::load(raw),
            halfka: HalfKA::new(),
            threats: Threats::new(),
            nnz_table,
            head: 0,
            stack: vec![Stack::new(); MAX_DEPTH_USIZE].into_boxed_slice(),
            // cache: CMCache::new(),
        };
        net.clear();
        net
    }

    pub fn new() -> Self {
        Self::build(&Permute::default())
    }

    pub fn init(&mut self, board: &Board) {
        self.halfka.init(board, &self.network);
        self.threats.init(board, &self.network);
        self.head = 0;
        self.stack[self.head].valid = false;
    }

    pub fn clear(&mut self) {
        self.halfka.clear(&self.network);
        // self.cache.clear();
    }

    pub fn catchup(&mut self, board: &Board) {
        self.halfka.catchup(self.halfka.head, board, &self.network);
        self.threats
            .catchup(self.threats.head, board, &self.network);
    }

    pub fn catchup_at(&mut self, head: (usize, usize, usize), board: &Board) {
        self.halfka.catchup(head.1, board, &self.network);
        self.threats.catchup(head.2, board, &self.network);
    }

    pub fn make_move(&mut self, board: &Board, new_board: &Board, m: Move) {
        self.halfka.make_move(board, m);
        self.threats.make_move(board, new_board, m);
        self.head += 1;
        self.stack[self.head].valid = false;
    }

    pub fn make_null_move(&mut self) {
        self.head += 1;
        self.stack[self.head].valid = false;
    }

    pub fn make_move_slow(&mut self, board: &Board, m: Move) {
        let mut new_board = board.clone();
        new_board.play_unchecked(m);
        self.make_move(board, &new_board, m);
    }

    pub fn unmake_move(&mut self) {
        self.halfka.unmake_move();
        self.threats.unmake_move();
        self.head -= 1;
    }

    pub fn unmake_null_move(&mut self) {
        self.head -= 1;
    }

    pub fn evaluate(&mut self, board: &Board) -> i32 {
        self.catchup(board);
        unsafe { self.evaluate_value(self.head(), board) }
    }

    unsafe fn evaluate_head(&mut self, head: (usize, usize, usize), board: &Board) {
        let stm = board.side_to_move() as usize;
        self.stack[head.0].valid = true;

        unsafe {
            const ZERO: i16 = 0i16;
            const ONE: i16 = QA as i16;
            let ft = &mut self.stack[head.0].ft;
            let idx_n = &mut self.stack[head.0].idx_n;
            let idx = &mut self.stack[head.0].idx;

            //- ft cleanup
            for side in 0..=1 {
                let acc = &self.halfka.side[head.1].vals[stm ^ side];
                let acc_threats = &self.threats.side[head.2].vals[stm ^ side];

                for i in 0..HL / 2 {
                    let x0 = (acc[i] + acc_threats[i]).clamp(ZERO, ONE);
                    let x1 = (acc[i + HL / 2] + acc_threats[i + HL / 2]).clamp(ZERO, ONE);
                    ft[side * HL / 2 + i] = ((x0 as u16 * x1 as u16) >> FT_SHIFT) as u8;
                }
            }

            let mut base = _mm_setzero_si128();
            let lookup_inc = _mm_set1_epi16(8);
            *idx_n = 0;
            for b in (0..HL).step_by(64) {
                let v = *(ft.as_ptr().add(b) as *const __m512i);

                // 1 if non zero
                let mask = _mm512_test_epi32_mask(v, v);
                for lookup in (0..16).step_by(8) {
                    debug_assert!(*idx_n + 16 <= HL / 4);
                    let slice = ((mask >> lookup) & 0xff) as u8;
                    let indices =
                        _mm_load_si128(self.nnz_table[slice as usize].as_ptr() as *const __m128i);
                    _mm_storeu_si128(
                        idx.as_mut_ptr().add(*idx_n) as *mut __m128i,
                        _mm_add_epi16(base, indices),
                    );
                    *idx_n += slice.count_ones() as usize;
                    base = _mm_add_epi16(base, lookup_inc);
                }
            }
        }
    }

    pub fn cm(&mut self, head: (usize, usize, usize), board: &Board) -> ([f32; CM], [f32; CM]) {
        // if let Some(entry) = self.cache.get(board.correct_hash()) {
        //     return entry;
        // }

        if !self.stack[head.0].valid {
            self.catchup_at(head, board);

            unsafe {
                self.evaluate_head(head, board);
            }
        }
        let (cm_from, cm_to) = self.evaluate_policy(head, board);
        // self.cache.set(board.correct_hash(), &cm_from, &cm_to);
        (cm_from, cm_to)
    }

    fn quantize_policy(logit: f32) -> f32 {
        logit
        // assert!(logit > -CM_MAX, "{}", logit);
        // ((logit + 2.0).clamp(-CM_MAX, CM_MAX) * CM_MULT) as i16
    }

    fn evaluate_policy(
        &mut self,
        head: (usize, usize, usize),
        board: &Board,
    ) -> ([f32; CM], [f32; CM]) {
        let bucket = Network::get_output_bucket(board);
        let mut cm_from = self.network.cm_from_bias[bucket].clone();
        let mut cm_to = self.network.cm_to_bias[bucket].clone();

        unsafe {
            let ft = &self.stack[head.0].ft;
            let idx_n = &self.stack[head.0].idx_n;
            let idx = &self.stack[head.0].idx;

            const STEP: usize = 16;
            let mut from_acc = [_mm512_setzero_epi32(); CM / STEP];
            let mut to_acc = [_mm512_setzero_epi32(); CM / STEP];
            let from_weights = &self.network.cm_from_weights[bucket];
            let to_weights = &self.network.cm_to_weights[bucket];
            for t in 0..*idx_n {
                let c = idx[t] as usize;
                let f = _mm512_set1_epi32(*(ft.as_ptr() as *const i32).add(c));

                let w_from = from_weights[c].as_ptr() as *const __m512i;
                let w_to = to_weights[c].as_ptr() as *const __m512i;
                for q in (0..CM).step_by(STEP) {
                    from_acc[q / STEP] =
                        _mm512_dpbusd_epi32(from_acc[q / STEP], f, *(w_from.add(q / STEP)));
                    to_acc[q / STEP] =
                        _mm512_dpbusd_epi32(to_acc[q / STEP], f, *(w_to.add(q / STEP)));
                }
            }

            let mut from_sum = Aligned::<i32, { CM }>::uninit();
            let mut to_sum = Aligned::<i32, { CM }>::uninit();
            for q in (0..CM).step_by(STEP) {
                *(from_sum.as_mut_ptr().add(q) as *mut __m512i) = from_acc[q / STEP];
                *(to_sum.as_mut_ptr().add(q) as *mut __m512i) = to_acc[q / STEP];
            }

            for i in 0..CM {
                cm_from[i] += from_sum[i] as f32 * Self::DIVISOR;
                cm_to[i] += to_sum[i] as f32 * Self::DIVISOR;
            }
        }

        (cm_from.0, cm_to.0)
    }

    unsafe fn evaluate_value(&mut self, head: (usize, usize, usize), board: &Board) -> i32 {
        let bucket = Network::get_output_bucket(board);

        unsafe {
            const ZEROF: f32 = 0.0f32;
            const ONEF: f32 = 1.0f32;

            self.evaluate_head(head, board);

            let ft = &self.stack[head.0].ft;
            let idx_n = &self.stack[head.0].idx_n;
            let idx = &self.stack[head.0].idx;

            const STEP: usize = 16;
            let mut l1_sum_acc = [_mm512_setzero_epi32(); L1 / STEP];
            let l1_weights = &self.network.l1_weights[bucket];
            for t in 0..*idx_n {
                let c = idx[t] as usize;
                let f = _mm512_set1_epi32(*(ft.as_ptr() as *const i32).add(c));
                let w = l1_weights[c].as_ptr() as *const __m512i;
                for q in (0..L1).step_by(STEP) {
                    l1_sum_acc[q / STEP] =
                        _mm512_dpbusd_epi32(l1_sum_acc[q / STEP], f, *(w.add(q / STEP)));
                }
            }

            let mut l1_sum = Aligned::<i32, L1>::uninit();
            for q in (0..L1).step_by(STEP) {
                *(l1_sum.as_mut_ptr().add(q) as *mut __m512i) = l1_sum_acc[q / STEP];
            }

            let mut l1 = Aligned::<f32, { L1 * 2 }>::uninit();
            for i in 0..L1 {
                let s = l1_sum[i] as f32 * Self::DIVISOR + self.network.l1_bias[bucket][i];
                let c = s.clamp(ZEROF, ONEF);
                l1[i] = c;
                l1[i + L1] = c * c;
            }

            //- l1 -> l2
            let mut l2_sum = Aligned::<f32, L2>::zeroed();
            for i in 0..(L1 * 2) {
                for j in 0..L2 {
                    l2_sum[j] += l1[i] * self.network.l2_weights[bucket][i][j];
                }
            }

            let mut l2 = Aligned::<f32, L2>::uninit();
            for i in 0..L2 {
                let s = (l2_sum[i] + self.network.l2_bias[bucket][i]).clamp(ZEROF, ONEF);
                l2[i] = s;
            }

            //- l2 -> output
            let mut out = _mm512_setzero_ps();
            let out_weights = self.network.output_weights[bucket].as_ptr();
            for i in (0..L2).step_by(16) {
                let l2_vec = *(l2.as_ptr().add(i) as *const __m512);
                let w_vec = *(out_weights.add(i) as *const __m512);
                out = _mm512_fmadd_ps(l2_vec, w_vec, out);
            }
            let output = _mm512_reduce_add_ps(out) + self.network.output_bias[bucket];
            (output * SCALE as f32) as i32
        }
    }

    pub fn sort_eval(&mut self, board: &Board) {
        let stm = board.side_to_move() as usize;
        const ZERO: i16 = 0i16;
        const ONE: i16 = QA as i16;
        for side in 0..=1 {
            let acc = &self.halfka.side[self.halfka.head].vals[stm ^ side];
            let acc_threats = &self.threats.side[self.threats.head].vals[stm ^ side];

            for i in 0..HL / 2 {
                let x0 = (acc[i] + acc_threats[i]).clamp(ZERO, ONE);
                let x1 = (acc[i + HL / 2] + acc_threats[i + HL / 2]).clamp(ZERO, ONE);
                self.stack[self.head].ft[side * HL / 2 + i] =
                    ((x0 as u16 * x1 as u16) >> FT_SHIFT) as u8;
            }
        }
    }

    pub fn sort_ft(&self) -> &Aligned<u8, HL> {
        &self.stack[self.head].ft
    }
}

pub fn policy_display(policy: &[f32; CM], board: &Board, is_from: bool) -> String {
    let mask = if is_from {
        board.colors(board.side_to_move())
    } else {
        !board.occupied()
    };
    let mut out = "".to_string();
    for i in 0..CM {
        let i = if board.side_to_move() == White {
            i ^ 56
        } else {
            i
        };
        if mask.has(Square::ALL[i]) {
            out += &format!("{:.2}", policy[i] as f32);
        } else {
            out += "0.00";
        }
        out += " ";

        if i % 8 == 7 {
            out += "\n";
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use cozy_chess::{Color, GameStatus};

    use crate::ext::ExtBoard;

    use super::*;

    #[test]
    fn make_unmake_test() {
        let mut net = NNUE::new();
        let board = Board::startpos();
        net.init(&board);
        let eval = net.evaluate(&board);

        let random_move = board.get_legal_moves()[12];
        net.make_move_slow(&board, random_move);
        net.unmake_move();
        assert_eq!(net.evaluate(&board), eval);
    }

    #[test]
    fn symmetry_test() {
        let mut net = NNUE::new();

        let board = Board::startpos();
        net.init(&board);
        let eval = net.evaluate(&board);

        let new_board = board.null_move().unwrap();
        net.init(&new_board);
        let eval_null = net.evaluate(&new_board);

        assert_eq!(eval, eval_null);
    }

    #[test]
    fn make_unmake_catchup_test() {
        let mut net = NNUE::new();
        let board = Board::startpos();
        net.init(&board);
        let eval = net.evaluate(&board);

        net.catchup(&board);
        let random_move = board.get_legal_moves()[12];
        net.make_move_slow(&board, random_move);

        let mut new_board = board.clone();
        new_board.play_unchecked(random_move);
        net.catchup(&new_board);

        net.unmake_move();
        net.catchup(&board);

        assert_eq!(net.evaluate(&board), eval);
    }

    #[test]
    fn random_make_unmake_catchup_test() {
        let mut net = NNUE::new();

        let sequence = vec![4, -2, 3, -2, -1, 10, -5, -2];

        let board = Board::startpos();
        net.init(&board);

        let mut evals = vec![None; 256];
        let mut boards = vec![None; 256];

        let mut sp = 0;
        evals[0] = Some(net.evaluate(&board));
        boards[0] = Some(board.clone());

        for op in sequence {
            if op > 0 {
                for _ in 0..op {
                    let board = boards[sp].clone().unwrap();
                    let moves = board.get_legal_moves();
                    let random_move = moves[1337 % moves.len()];

                    net.make_move_slow(&board, random_move);

                    let mut next_board = board.clone();
                    next_board.play_unchecked(random_move);

                    sp += 1;
                    boards[sp] = Some(next_board.clone());
                    net.catchup(&next_board);
                    evals[sp] = Some(net.evaluate(&next_board));
                }
            } else {
                for _ in 0..-op {
                    net.unmake_move();

                    sp -= 1;

                    let board = boards[sp].clone().unwrap();
                    net.catchup(&board);
                    assert_eq!(net.evaluate(&board), evals[sp].unwrap());
                }
            }
        }
    }

    #[test]
    fn random_make_unmake_test() {
        let mut net = NNUE::new();

        let sequence = vec![4, -2, 3, -2, -1, 10, -5, -2, 20, -1, -2, -5, -10];

        let board = Board::startpos();
        net.init(&board);

        let mut evals = vec![None; 256];
        let mut boards = vec![None; 256];

        let mut sp = 0;
        evals[0] = Some(net.evaluate(&board));
        boards[0] = Some(board.clone());

        for op in sequence {
            if op > 0 {
                for _ in 0..op {
                    let board = boards[sp].clone().unwrap();
                    let moves = board.get_legal_moves();
                    let random_move = moves[1337 % moves.len()];

                    net.make_move_slow(&board, random_move);

                    let mut next_board = board.clone();
                    next_board.play_unchecked(random_move);

                    sp += 1;
                    boards[sp] = Some(next_board.clone());
                    evals[sp] = Some(net.evaluate(&next_board));
                }
            } else {
                for _ in 0..-op {
                    net.unmake_move();

                    sp -= 1;

                    let board = boards[sp].clone().unwrap();
                    assert_eq!(net.evaluate(&board), evals[sp].unwrap());
                }
            }
        }
    }

    #[test]
    fn random_make_unmake_init_test() {
        let mut net = NNUE::new();

        for op in 0..100 {
            let mut board = Board::startpos();
            net.init(&board);

            while board.status() == GameStatus::Ongoing {
                let moves = board.get_legal_moves();
                let random_move = moves[op % moves.len()];

                net.make_move_slow(&board, random_move);

                let mut next_board = board.clone();
                next_board.play_unchecked(random_move);
                let incr_eval = net.evaluate(&next_board);
                net.init(&next_board);
                let err = incr_eval - net.evaluate(&next_board);
                assert!(
                    err.abs() < 2,
                    "{} against {}, fen {}, move {}",
                    incr_eval,
                    net.evaluate(&next_board),
                    board,
                    random_move
                );

                board = next_board;
            }
        }
    }

    #[test]
    fn test_eval() {
        let mut net = NNUE::new();
        let board =
            Board::from_fen("6k1/p7/3q1nr1/3p3R/p3r3/8/7P/3Q1R1K w - - 2 52", false).unwrap();
        net.init(&board);
        let eval = net.evaluate(&board);
        assert_eq!(eval, -1469);
    }

    #[test]
    fn test_eval2() {
        let mut net = NNUE::new();
        let board = Board::from_fen(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            false,
        )
        .unwrap();
        net.init(&board);
        let eval = net.evaluate(&board);
        assert_eq!(eval, 38);
    }

    fn grid_to_string(grid: &[f32], board: &Board) -> String {
        let mut out = "".to_string();
        for i in 0..CM {
            let i = if board.side_to_move() == Color::White {
                i ^ 56
            } else {
                i
            };
            if board.colors(board.side_to_move()).has(Square::ALL[i]) {
                out += &format!("{:.2}", grid[i]);
            } else {
                out += "0.00";
            }
            out += " ";

            if i % 8 == 7 {
                out += "\n";
            }
        }

        out
    }

    fn grid_to_string2(grid: &[f32], board: &Board) -> String {
        let mut out = "".to_string();
        for i in 0..CM {
            let i = if board.side_to_move() == Color::White {
                i ^ 56
            } else {
                i
            };
            if !board.occupied().has(Square::ALL[i]) {
                out += &format!("{:.2}", grid[i]);
            } else {
                out += "0.00";
            }
            out += " ";

            if i % 8 == 7 {
                out += "\n";
            }
        }

        out
    }

    #[test]
    fn test_cm() {
        let mut net = NNUE::new();
        let board = Board::from_fen(
            // "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bqkbnr/pp2pppp/3p4/2p5/3NP3/2N5/PPP2PPP/R1BQKB1R b KQkq - 0 5",
            false,
        )
        .unwrap();
        net.init(&board);
        net.evaluate_policy(net.head(), &board);

        let (cm_from, cm_to) = net.cm(net.head(), &board);

        let cm_from = policy_display(&cm_from, &board, true);
        assert!(cm_from == "", "\n{}", cm_from);
    }
}
