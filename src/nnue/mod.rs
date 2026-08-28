use std::{arch::x86_64::*, mem::MaybeUninit};

use cozy_chess::{Board, Move};

use crate::nnue::{
    halfka::HalfKA,
    network::{Aligned, FT_SHIFT, HL_NO_PST, L1, L2, Network, Permute, QA, QB, RawNetwork, SCALE},
    threats::Threats,
};

mod halfka;
pub mod network;
mod threats;
mod ti;
mod update;

pub struct NNUE {
    network: Box<Network>,
    halfka: HalfKA,
    threats: Threats,
    nnz_table: [Aligned<u16, 8>; 256],
    // this is just a temp cache
    ft: Aligned<u8, HL_NO_PST>,
}

impl NNUE {
    pub fn build(permute: &Permute) -> Self {
        let mut raw = RawNetwork::load();
        raw.permute(permute);

        // nnz_table[bits][i] = ith bit in bits offset
        let mut nnz_table: [Aligned<u16, 8>; 256] = unsafe { MaybeUninit::zeroed().assume_init() };
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
            ft: Aligned::<u8, HL_NO_PST>::zeroed(),
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
    }

    pub fn clear(&mut self) {
        self.halfka.clear(&self.network);
    }

    pub fn catchup(&mut self, board: &Board) {
        self.halfka.catchup(board, &self.network);
        self.threats.catchup(board, &self.network);
    }

    pub fn make_move(&mut self, board: &Board, new_board: &Board, m: Move) {
        self.halfka.make_move(board, m);
        self.threats.make_move(board, new_board, m);
    }

    pub fn make_move_slow(&mut self, board: &Board, m: Move) {
        let mut new_board = board.clone();
        new_board.play_unchecked(m);
        self.make_move(board, &new_board, m);
    }

    pub fn unmake_move(&mut self) {
        self.halfka.unmake_move();
        self.threats.unmake_move();
    }

    pub fn evaluate(&mut self, board: &Board) -> i32 {
        self.catchup(board);
        unsafe { self.avx512_evaluate(board) }
    }

    #[target_feature(enable = "avx512f,avx512bw,avx512vnni")]
    unsafe fn avx512_evaluate(&mut self, board: &Board) -> i32 {
        let bucket = Network::get_output_bucket(board);
        let stm = board.side_to_move() as usize;

        unsafe {
            const DIVISOR: f32 = (1.0 / ((QA * QA * QB) >> FT_SHIFT) as f32) as f32;
            const ZERO: i16 = 0i16;
            const ONE: i16 = QA as i16;
            const ZEROF: f32 = 0.0f32;
            const ONEF: f32 = 1.0f32;

            // pst
            // let pst = (self.halfka.side[self.halfka.head].vals[stm][HL_NO_PST + bucket] as i32
            //     - self.halfka.side[self.halfka.head].vals[stm ^ 1][HL_NO_PST + bucket] as i32
            //     + self.threats.side[self.threats.head].vals[stm][HL_NO_PST + bucket] as i32
            //     - self.threats.side[self.threats.head].vals[stm ^ 1][HL_NO_PST + bucket] as i32)
            //     as f32
            //     / (2.0 * QA as f32);

            //- ft cleanup
            for side in 0..=1 {
                let acc = &self.halfka.side[self.halfka.head].vals[stm ^ side];
                let acc_threats = &self.threats.side[self.threats.head].vals[stm ^ side];

                for i in 0..HL_NO_PST / 2 {
                    let x0 = (acc[i] + acc_threats[i]).clamp(ZERO, ONE);
                    let x1 =
                        (acc[i + HL_NO_PST / 2] + acc_threats[i + HL_NO_PST / 2]).clamp(ZERO, ONE);
                    self.ft[side * HL_NO_PST / 2 + i] = ((x0 as u16 * x1 as u16) >> FT_SHIFT) as u8;
                }
            }

            let mut idx = Aligned::<u16, { HL_NO_PST / 4 }>::uninit();
            let mut base = _mm_setzero_si128();
            let lookup_inc = _mm_set1_epi16(8);
            let mut n = 0;
            for b in (0..HL_NO_PST).step_by(64) {
                let v = *(self.ft.as_ptr().add(b) as *const __m512i);

                // skip if all 64 u8 are zero
                // if _mm512_test_epi64_mask(v, v) == 0 {
                //     base = _mm_add_epi16(base, _mm_set1_epi16(16));
                //     continue;
                // }

                let mask = _mm512_cmpgt_epu32_mask(v, _mm512_setzero_si512());
                for lookup in (0..16).step_by(8) {
                    let slice = ((mask >> lookup) & 0xff) as u8;
                    let indices = *(self.nnz_table[slice as usize].as_ptr() as *const __m128i);
                    _mm_storeu_si128(
                        idx.as_mut_ptr().add(n) as *mut __m128i,
                        _mm_add_epi16(base, indices),
                    );
                    n += slice.count_ones() as usize;
                    base = _mm_add_epi16(base, lookup_inc);
                }
            }

            const STEP: usize = 16;
            let mut l1_sum_acc = [_mm512_setzero_epi32(); L1 / STEP];
            let l1_weights = &self.network.l1_weights[bucket];
            for t in 0..n {
                let c = idx[t] as usize;
                let f = _mm512_set1_epi32(*(self.ft.as_ptr() as *const i32).add(c));
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

            // for i in 0..L1 {
            //     for j in 0..HL_NO_PST {
            //         l1_sum[i] += self.ft[j] as i32 * self.network.l1_weights[bucket][i][j] as i32;
            //     }
            // }

            let mut l1 = Aligned::<f32, L1>::uninit();
            for i in 0..L1 {
                let s = (l1_sum[i] as f32 * DIVISOR + self.network.l1_bias[bucket][i])
                    .clamp(ZEROF, ONEF);
                l1[i] = s * s;
            }

            //- l1 -> l2
            let mut l2_sum = Aligned::<f32, L2>::zeroed();
            for i in 0..L1 {
                for j in 0..L2 {
                    l2_sum[j] += l1[i] * self.network.l2_weights[bucket][i][j];
                }
            }

            let mut l2 = Aligned::<f32, L2>::uninit();
            for i in 0..L2 {
                let s = (l2_sum[i] + self.network.l2_bias[bucket][i]).clamp(ZEROF, ONEF);
                l2[i] = s * s;
            }

            // TODO: this might be slow

            //- l2 -> output

            // let mut output = self.network.output_bias[bucket];
            // for i in 0..L2 {
            //     output += l2[i] * self.network.output_weights[bucket][i];
            // }
            let mut out = _mm512_setzero_ps();
            let out_weights = self.network.output_weights[bucket].as_ptr();
            for i in (0..L2).step_by(16) {
                let l2_vec = _mm512_load_ps(l2.as_ptr().add(i));
                let w_vec = _mm512_load_ps(out_weights.add(i));
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

            for i in 0..HL_NO_PST / 2 {
                let x0 = (acc[i] + acc_threats[i]).clamp(ZERO, ONE);
                let x1 = (acc[i + HL_NO_PST / 2] + acc_threats[i + HL_NO_PST / 2]).clamp(ZERO, ONE);
                self.ft[side * HL_NO_PST / 2 + i] = ((x0 as u16 * x1 as u16) >> FT_SHIFT) as u8;
            }
        }
    }

    pub fn sort_ft(&self) -> &Aligned<u8, HL_NO_PST> {
        &self.ft
    }
}

#[cfg(test)]
mod tests {
    use cozy_chess::GameStatus;

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
        assert_eq!(eval, -1532);
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
        assert_eq!(eval, 35);
    }
}
