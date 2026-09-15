use std::{
    fs::File,
    io::{BufReader, Read, Write},
    path::Path,
};

use cozy_chess::Board;
use rand::Rng;
use sfbinpack::{
    TrainingDataEntry,
    chess::{r#move::MoveType, piecetype::PieceType},
};

use crate::{
    ext::ExtBoard,
    nnue::{
        NNUE,
        network::{HL, Permute},
    },
};

// TODO: improve this asw

fn filter(entry: &TrainingDataEntry) -> bool {
    !entry.pos.is_checked(entry.pos.side_to_move()) && entry.score.abs() < 5000
}

fn get_boards(file: &str, skip: usize, iter: usize) -> Vec<Board> {
    let file = File::open(file).unwrap();
    let mut reader =
        sfbinpack::CompressedTrainingDataEntryReader::new(BufReader::new(file)).unwrap();

    let mut boards = vec![];
    let mut it = 0;
    while it < skip + iter {
        let entry = reader.next();
        if !rand::random_bool(1.0 / 20.0) {
            continue;
        }
        if !filter(&entry) {
            continue;
        }

        it += 1;
        if it > skip {
            let board = Board::from_fen(&entry.pos.fen().unwrap(), false).unwrap();
            boards.push(board);
        }
    }

    boards
}

fn benchmark(mut net: NNUE, boards: &Vec<Board>) -> f64 {
    let mut sparseness = 0;
    for board in boards.iter() {
        net.init(board);
        net.sort_eval(board);

        let ft = net.sort_ft();
        for i in (0..HL).step_by(4) {
            let mut all_zero = true;
            for k in 0..4 {
                if ft[i + k] > 0 {
                    all_zero = false;
                    break;
                }
            }

            if all_zero {
                sparseness += 1;
            }
        }
    }

    sparseness as f64 / (HL / 4 * boards.len()) as f64
}

pub fn compute_co_occurrence_mapping(path: &str, iter: usize) -> [usize; HL] {
    let file = File::open(path).unwrap();
    let mut reader =
        sfbinpack::CompressedTrainingDataEntryReader::new(BufReader::new(file)).unwrap();

    let half_hl = HL / 2;

    let mut net = NNUE::new();
    // Exact activity signatures. Each sampled position owns one bit, with a
    // separate u64 word for every group of 64 positions.
    let words = iter.div_ceil(32);
    let mut activity = vec![vec![0u64; words]; half_hl];

    // Collect co-occurrence statistics for 0..HL / 2
    let mut it = 0;
    while it < iter {
        let entry = reader.next();

        if !filter(&entry) {
            continue;
        }

        it += 1;

        let board = Board::from_fen(&entry.pos.fen().unwrap(), false).unwrap();
        net.init(&board);
        net.sort_eval(&board);

        let ft = net.sort_ft();

        // Track active feature indices only in the first half
        let mut active = Vec::with_capacity(half_hl);
        for i in 0..half_hl {
            if ft[i] > 0 {
                active.push(i);
            }
        }

        let hash = board.correct_hash();
        let sample = it - 1;
        let sample_word = sample / 32;
        let sample_bit = 1u64 << (hash & 63);
        for neuron in active {
            activity[neuron][sample_word] |= sample_bit;
        }
    }

    // Initialize mapping with identity mapping for the full array
    let mut mapping = [0usize; HL];
    for i in 0..HL {
        mapping[i] = i;
    }

    // Pack the first half into 4-neuron SIMD blocks.  A block is useful when
    // its neurons are active in as few distinct positions as possible, so the
    // objective is popcount(activity[a] | activity[b] | ...).
    let mut blocks: Vec<[usize; 4]> = Vec::with_capacity(half_hl / 4);
    let mut used = vec![false; half_hl];

    while blocks.len() * 4 < half_hl {
        // Start with the densest remaining signature.  This makes the choice
        // deterministic and gives later candidates a useful anchor.
        let seed = (0..half_hl)
            .filter(|&i| !used[i])
            .max_by_key(|&i| activity[i].iter().map(|x| x.count_ones()).sum::<u32>())
            .expect("there must be an unused feature");

        let mut block = [seed; 4];
        used[seed] = true;

        for slot in 1..4 {
            let mut candidate = None;
            let mut best_score = usize::MAX;
            let mut best_activity = 0;
            for i in 0..half_hl {
                if used[i] {
                    continue;
                }
                let mut score = 0;
                for word in 0..words {
                    let mut union = activity[i][word];
                    for j in 0..slot {
                        union |= activity[block[j]][word];
                    }
                    score += union.count_ones() as usize;
                }
                let active = activity[i].iter().map(|x| x.count_ones()).sum();
                if score < best_score || (score == best_score && active > best_activity) {
                    candidate = Some(i);
                    best_score = score;
                    best_activity = active;
                }
            }
            let candidate = candidate.expect("a 4-neuron block must be fillable");
            block[slot] = candidate;
            used[candidate] = true;
        }

        blocks.push(block);
    }

    // Greedy construction depends on the order in which blocks are seeded.
    // Improve it with 2-opt swaps between blocks until no single swap lowers
    // the union objective.
    let block_score = |block: &[usize; 4]| -> u64 {
        let mut score = 0;
        for word in 0..words {
            let mut union = 0;
            for &neuron in block {
                union |= activity[neuron][word];
            }
            score += union.count_ones() as u64;
        }
        score
    };

    loop {
        let mut improved = false;
        'search: for a in 0..blocks.len() {
            for b in a + 1..blocks.len() {
                let old_score = block_score(&blocks[a]) + block_score(&blocks[b]);
                for ia in 0..4 {
                    for ib in 0..4 {
                        let old_a = blocks[a][ia];
                        let old_b = blocks[b][ib];
                        blocks[a][ia] = old_b;
                        blocks[b][ib] = old_a;

                        let new_score = block_score(&blocks[a]) + block_score(&blocks[b]);
                        if new_score < old_score {
                            improved = true;
                            continue 'search;
                        }

                        // Undo a rejected swap.
                        blocks[a][ia] = old_a;
                        blocks[b][ib] = old_b;
                    }
                }
            }
        }
        if !improved {
            break;
        }
    }

    for (block_idx, block) in blocks.iter().enumerate() {
        mapping[block_idx * 4..block_idx * 4 + 4].copy_from_slice(block);
    }

    mapping
}

pub fn start(path: &str, iter: usize) {
    let boards = get_boards(path, 0, iter);
    let net = NNUE::new();
    println!("starting raw sparseness {}", benchmark(net, &boards));
    let net = NNUE::build(&Permute::load());
    let baseline = benchmark(net, &boards);
    println!("starting sparseness {}", baseline);

    let mapping = compute_co_occurrence_mapping(path, iter);

    let net = NNUE::build(&Permute::new(mapping));
    println!("ending sparseness {}", benchmark(net, &boards));

    let boards = get_boards(path, iter, iter);
    let net = NNUE::build(&Permute::new(mapping));
    println!("ending sparseness2 {}", benchmark(net, &boards));

    Permute::new(mapping).save();
}

mod tests {
    use super::*;

    #[test]
    fn test_permute() {
        let fens = vec![
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/N7/PPPPPPPP/R1BQKBNR b KQkq - 1 1",
            "rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b KQkq - 1 1",
            "r3r1k1/pp3pbp/1qp1b1p1/2B5/2BP4/Q1n2N2/P4PPP/3R1K1R w - - 4 18",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
            "3k4/3p4/8/K1P4r/8/8/8/8 b - - 0 1",
            "8/8/4k3/8/2p5/8/B2P2K1/8 w - - 0 1",
            "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1",
            "5k2/8/8/8/8/8/8/4K2R w K - 0 1",
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/4P3/3P1N2/PPP2PPP/RNBQKB1R b KQkq - 0 4",
            "r1b1k2r/pp1p1ppp/2n1pn2/q7/1bP1P3/2N2N2/PP1B1PPP/R2QKB1R w KQkq - 4 8",
            "r2q1rk1/pbpn1ppp/1p2pn2/3p4/1bPP4/2N1PN2/PPQB1PPP/R3KB1R w KQ - 2 9",
            "r1bqkb1r/pppp1ppp/2n5/4p3/2B1n3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 5",
            "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/2N5/PPPP1PPP/R1BQKBNR b KQkq - 1 3",
            "r1bqk2r/pppp1ppp/2n2n2/4p3/1b2P3/2NP1N2/PPP2PPP/R1BQKB1R w KQkq - 1 6",
            "r2qkb1r/ppp2ppp/2np1n2/4p3/4P1b1/2NP1N2/PPP2PPP/R1BQKB1R w KQkq - 2 6",
            "r1bq1rk1/pppn1ppp/4pn2/3p4/1bPP4/2N1PN2/PP1B1PPP/R2QKB1R w KQ - 3 7",
            "r2qk2r/pppnbppp/4pn2/3p1b2/3P4/2N1PN2/PPP1BPPP/R1BQ1RK1 w kq - 4 8",
            "r1bq1rk1/pppn1ppp/4pn2/3p4/1bPP4/2N1PN2/PP1B1PPP/R2QKB1R b KQ - 3 7",
            "r1bqkb1r/pppp1ppp/2n5/4p3/4n3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 5",
            "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/2N5/PPPP1PPP/R1BQKBNR w KQkq - 2 3",
            "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
        ];

        let mut net = NNUE::new();
        let mut perm_net = NNUE::build(&Permute::load());
        for fen in fens {
            let board = Board::from_fen(fen, false).unwrap();
            net.init(&board);
            perm_net.init(&board);

            assert_eq!(net.evaluate(&board), perm_net.evaluate(&board));
        }
    }
}
