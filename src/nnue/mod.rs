use std::arch::x86_64::*;

use cozy_chess::{Board, Color::White, File, Move, Square};

use crate::{
    nnue::{
        halfka::HalfKA,
        network::{
            Aligned, Aligned16, CM, FT_SHIFT, HL, L1, L1_OUT, L2, L2_OUT, Network, Permute, QA, QB,
            QPST, RawNetwork, SCALE, SimdOps,
        },
        pst::Pst,
        threats::Threats,
    },
    param::MAX_DEPTH_USIZE,
};

mod halfka;
pub mod network;
mod pst;
mod threats;
mod ti;
mod update;

pub fn init() {
    ti::init();
}

#[derive(Clone)]
struct Cache {
    valid: bool,
    ft: Aligned<u8, HL>,
    idx_n: usize,
    idx: Aligned<u16, { HL / 4 }>,
}

impl Cache {
    pub fn new() -> Self {
        Self {
            valid: false,
            ft: Aligned::<u8, HL>::zeroed(),
            idx_n: 0,
            idx: Aligned::zeroed(),
        }
    }
}

pub struct NNUE {
    network: Box<Network>,
    pst: Pst,
    halfka: HalfKA,
    threats: Threats,
    nnz_table: [Aligned16; 256],
    head: usize,
    cache: Cache,
}

impl NNUE {
    // const FT_SHIFT_SCALE: f32 = QA as f32 / (1 << FT_SHIFT) as f32;
    const FT_TUNE: f32 = 1.0;
    const DIVISOR: f32 = Self::FT_TUNE / ((1 << FT_SHIFT) as f32 * (QB as f32));

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
            pst: Pst::new(),
            halfka: HalfKA::new(),
            threats: Threats::new(),
            nnz_table,
            head: 0,
            cache: Cache::new(),
        };
        net.clear();
        net
    }

    pub fn new() -> Self {
        Self::build(&Permute::default())
    }

    pub fn init(&mut self, board: &Board) {
        self.pst.init(board, &self.network);
        self.halfka.init(board, &self.network);
        self.threats.init(board, &self.network);
        self.head = 0;
    }

    pub fn clear(&mut self) {
        self.pst.clear(&self.network);
        self.halfka.clear(&self.network);
    }

    pub fn catchup(&mut self, board: &Board) {
        self.pst.catchup(self.pst.head, board, &self.network);
        self.halfka.catchup(self.halfka.head, board, &self.network);
        self.threats
            .catchup(self.threats.head, board, &self.network);
    }

    pub fn make_move(&mut self, board: &Board, new_board: &Board, m: Move) {
        let update = self.halfka.make_move(board, m);
        self.pst.make_move(update);
        self.threats.make_move(board, new_board, m);
        self.head += 1;
    }

    pub fn make_null_move(&mut self) {}

    fn make_move_slow(&mut self, board: &Board, m: Move) {
        let mut new_board = board.clone();
        new_board.play_unchecked(m);
        self.make_move(board, &new_board, m);
    }

    pub fn unmake_move(&mut self) {
        self.pst.unmake_move();
        self.halfka.unmake_move();
        self.threats.unmake_move();

        assert!(self.head > 0);
        self.head -= 1;
    }

    pub fn unmake_null_move(&mut self) {}

    pub fn evaluate(&mut self, board: &Board) -> i32 {
        self.catchup(board);
        self.evaluate_value(board).0
    }

    pub fn evaluate2(&mut self, board: &Board) -> (i32, i32) {
        self.catchup(board);
        self.evaluate_value(board)
    }

    fn evaluate_value(&mut self, board: &Board) -> (i32, i32) {
        let bucket = Network::get_output_bucket(board);
        let stm = board.side_to_move() as usize;

        unsafe {
            const STEP: usize = 16;

            let divisor = _mm512_set1_ps(Self::DIVISOR);

            let ft = &mut self.cache.ft;
            let idx_n = &mut self.cache.idx_n;
            let idx = &mut self.cache.idx;

            *idx_n = 0;
            for side in 0..=1 {
                let acc = &self.halfka.side[self.head].vals[stm ^ side];
                let threats = &self.threats.side[self.head].vals[stm ^ side];
                for i in (0..HL / 2).step_by(64) {
                    let zero = _mm512_setzero_si512();
                    let one = _mm512_set1_epi16(QA as i16);

                    let clip = |x| _mm512_min_epi16(_mm512_max_epi16(x, zero), one);

                    let mut scaled = [_mm512_setzero_si512(); 2];
                    for part in 0..2 {
                        let j = i + part * 32;
                        let x0 = clip(_mm512_add_epi16(
                            _mm512_load_si512(acc.as_ptr().add(j).cast()),
                            _mm512_load_si512(threats.as_ptr().add(j).cast()),
                        ));
                        let x1 = clip(_mm512_add_epi16(
                            _mm512_load_si512(acc.as_ptr().add(j + HL / 2).cast()),
                            _mm512_load_si512(threats.as_ptr().add(j + HL / 2).cast()),
                        ));
                        scaled[part] =
                            _mm512_srli_epi16::<{ FT_SHIFT as u32 }>(_mm512_mullo_epi16(x0, x1));
                    }

                    // packus interleaves 128-bit lanes; restore consecutive features.
                    let order = _mm512_setr_epi64(0, 2, 4, 6, 1, 3, 5, 7);
                    let bytes =
                        _mm512_permutexvar_epi64(order, _mm512_packus_epi16(scaled[0], scaled[1]));

                    let offset = side * HL / 2 + i;
                    _mm512_store_si512(ft.as_mut_ptr().add(offset).cast(), bytes);

                    let mask = _mm512_test_epi32_mask(bytes, bytes);
                    #[cfg(all(target_feature = "avx512vbmi2", target_feature = "avx512vl"))]
                    {
                        debug_assert!(*idx_n + 16 <= HL / 4);
                        let lanes =
                            _mm256_setr_epi16(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
                        let indices = _mm256_maskz_compress_epi16(
                            mask,
                            _mm256_add_epi16(lanes, _mm256_set1_epi16((offset / 4) as i16)),
                        );
                        _mm256_storeu_si256(idx.as_mut_ptr().add(*idx_n).cast(), indices);
                        *idx_n += mask.count_ones() as usize;
                    }

                    #[cfg(not(all(target_feature = "avx512vbmi2", target_feature = "avx512vl")))]
                    for lookup in (0..16).step_by(8) {
                        debug_assert!(*idx_n + 8 <= HL / 4);
                        let slice = ((mask >> lookup) & 0xff) as u8;
                        let indices =
                            _mm_load_si128(self.nnz_table[slice as usize].as_ptr().cast());
                        _mm_storeu_si128(
                            idx.as_mut_ptr().add(*idx_n).cast(),
                            _mm_add_epi16(_mm_set1_epi16((offset / 4 + lookup) as i16), indices),
                        );
                        *idx_n += slice.count_ones() as usize;
                    }
                }
            }

            // l1
            let mut partial = [[_mm512_setzero_epi32(); L1 / STEP]; 4];
            let l1_weights = &self.network.l1_weights[bucket];
            let mut t = 0;
            while t + 4 <= *idx_n {
                for lane in 0..4 {
                    let c = idx[t + lane] as usize;
                    let f = _mm512_set1_epi32(*(ft.as_ptr() as *const i32).add(c));
                    let weights = l1_weights[c].as_ptr();
                    for (i, q) in (0..L1).step_by(STEP).enumerate() {
                        partial[lane][i] = _mm512_dpbusd_epi32(
                            partial[lane][i],
                            f,
                            _mm512_load_si512(weights.add(q * 4).cast()),
                        );
                    }
                }
                t += 4;
            }
            for t in t..*idx_n {
                let c = idx[t] as usize;
                let f = _mm512_set1_epi32(*(ft.as_ptr() as *const i32).add(c));
                let weights = l1_weights[c].as_ptr();
                for (i, q) in (0..L1).step_by(STEP).enumerate() {
                    partial[0][i] = _mm512_dpbusd_epi32(
                        partial[0][i],
                        f,
                        _mm512_load_si512(weights.add(q * 4).cast()),
                    );
                }
            }
            let mut l1_partial = [_mm512_setzero_epi32(); L1 / STEP];
            for i in 0..L1 / STEP {
                l1_partial[i] = _mm512_add_epi32(
                    _mm512_add_epi32(partial[0][i], partial[1][i]),
                    _mm512_add_epi32(partial[2][i], partial[3][i]),
                );
            }

            // [SiLU(l1), l1]
            let mut l1 = Aligned::<f32, L1_OUT>::zeroed();
            let l1_bias = self.network.l1_bias[bucket].as_ptr();
            for (i, q) in (0..L1).step_by(STEP).enumerate() {
                let s = _mm512_fmadd_ps(
                    _mm512_cvtepi32_ps(l1_partial[i]),
                    divisor,
                    _mm512_load_ps(l1_bias.add(q)),
                );

                _mm512_store_ps(l1.as_mut_ptr().add(q), SimdOps::silu(s));
                _mm512_store_ps(l1.as_mut_ptr().add(L1 + q), s);
            }

            //- l1 -> l2
            let mut l2_partial = [[_mm512_setzero_ps(); L2 / STEP]; 4];
            for i in 0..L1_OUT {
                let activation = _mm512_set1_ps(l1[i]);
                let weights = self.network.l2_weights[bucket][i].as_ptr();
                for (j, q) in (0..L2).step_by(STEP).enumerate() {
                    l2_partial[i % 4][j] = _mm512_fmadd_ps(
                        activation,
                        _mm512_load_ps(weights.add(q)),
                        l2_partial[i % 4][j],
                    );
                }
            }

            // [SiLU(l2), l2, SiLU(l1), l1]
            let mut l2 = Aligned::<f32, L2_OUT>::zeroed();
            let l2_bias = self.network.l2_bias[bucket].as_ptr();
            for (i, q) in (0..L2).step_by(STEP).enumerate() {
                let l2_sum = _mm512_add_ps(
                    _mm512_add_ps(l2_partial[0][i], l2_partial[1][i]),
                    _mm512_add_ps(l2_partial[2][i], l2_partial[3][i]),
                );
                let s = _mm512_add_ps(l2_sum, _mm512_load_ps(l2_bias.add(q)));

                _mm512_store_ps(l2.as_mut_ptr().add(q), SimdOps::silu(s));
                _mm512_store_ps(l2.as_mut_ptr().add(L2 + q), s);
            }
            l2[L2 * 2..].copy_from_slice(&l1[..]);

            //- l2 -> output
            let mut out = _mm512_setzero_ps();
            let out_weights = self.network.output_weights[bucket].as_ptr();
            for i in (0..L2_OUT).step_by(STEP) {
                out = _mm512_fmadd_ps(
                    _mm512_load_ps(l2.as_ptr().add(i)),
                    _mm512_load_ps(out_weights.add(i)),
                    out,
                );
            }
            let output = _mm512_reduce_add_ps(out) + self.network.output_bias[bucket];

            // pst
            let stm = board.side_to_move() as usize;
            let pst = (self.pst.side[self.head].vals[stm][bucket]
                - self.pst.side[self.head].vals[stm ^ 1][bucket]) as f32
                / (2 * QPST) as f32;

            let combined = output + pst;

            (
                (combined * SCALE as f32) as i32,
                (output * SCALE as f32) as i32,
            )
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
                self.cache.ft[side * HL / 2 + i] = ((x0 as u16 * x1 as u16) >> FT_SHIFT) as u8;
            }
        }
    }

    pub fn sort_ft(&self) -> &Aligned<u8, HL> {
        &self.cache.ft
    }
}

#[cfg(test)]
mod tests {
    use cozy_chess::{Color, GameStatus};

    use crate::ext::ExtBoard;

    use super::*;

    #[test]
    fn deferred_evaluation_matches_eager_after_branching_and_null_moves() {
        super::init();
        let mut deferred = NNUE::new();
        let mut eager = NNUE::new();
        let cases = [
            (
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                "e2e4",
            ),
            ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1h1"),
            ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1a1"),
            ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
            ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8n"),
            ("1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7b8q"),
            ("4k2r/pp3ppp/8/8/8/8/PP3PPP/R2K4 w - - 0 1", "d1e1"),
        ];
        let mut seed = 1337u64;
        for (fen, first_move) in cases {
            let root = Board::from_fen(fen, false).unwrap();
            deferred.init(&root);
            eager.init(&root);
            // Each entry records whether the move entering it was null.
            let mut path = vec![(root, false)];
            for step in 0..96 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                if step % 5 == 4 && path.len() > 1 {
                    let (_, was_null) = path.pop().unwrap();
                    if was_null {
                        deferred.unmake_null_move();
                        eager.unmake_null_move();
                    } else {
                        deferred.unmake_move();
                        eager.unmake_move();
                    }
                } else {
                    let (board, was_null) = path.last().unwrap();
                    if step % 11 == 3 && !was_null && !board.in_check() {
                        let next = board.null_move().unwrap();
                        deferred.make_null_move();
                        eager.make_null_move();
                        path.push((next, true));
                    } else {
                        let moves = board.get_legal_moves();
                        if moves.is_empty() {
                            break;
                        }
                        let m = if step == 0 {
                            first_move.parse().unwrap()
                        } else {
                            moves[(seed >> 32) as usize % moves.len()]
                        };
                        assert!(board.is_legal(m));
                        let mut next = board.clone();
                        next.play_unchecked(m);
                        deferred.make_move(board, &next, m);
                        eager.make_move(board, &next, m);
                        path.push((next, false));
                    }
                }
                let board = &path.last().unwrap().0;
                let expected = eager.evaluate2(board);
                if step % 7 == 6 {
                    assert_eq!(
                        deferred.evaluate2(board),
                        expected,
                        "{fen}, step {step}: {board}"
                    );
                }
            }
            let board = &path.last().unwrap().0;
            assert_eq!(
                deferred.evaluate2(board),
                eager.evaluate2(board),
                "{fen}: {board}"
            );
        }
    }

    #[test]
    fn make_unmake_test() {
        super::init();
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
        super::init();
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
        super::init();
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
        super::init();
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
        super::init();
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
        super::init();
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
        super::init();
        let mut net = NNUE::new();
        let board =
            Board::from_fen("6k1/p7/3q1nr1/3p3R/p3r3/8/7P/3Q1R1K w - - 2 52", false).unwrap();
        net.init(&board);
        let eval = net.evaluate(&board);
        assert_eq!(eval, -1593);
    }

    #[test]
    fn test_eval2() {
        super::init();
        let mut net = NNUE::new();
        let board = Board::from_fen(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            false,
        )
        .unwrap();
        net.init(&board);
        let eval = net.evaluate(&board);
        assert_eq!(eval, 19);
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
}
