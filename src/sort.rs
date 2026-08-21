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

use crate::nnue::{
    NNUE,
    network::{HL_NO_PST, Permute},
};

fn filter(entry: &TrainingDataEntry) -> bool {
    entry.ply >= 14
        && !entry.pos.is_checked(entry.pos.side_to_move())
        && entry.score.abs() < 10000
}

fn get_boards(file: &str, iter: usize) -> Vec<Board> {
    let file = File::open(file).unwrap();
    let mut reader =
        sfbinpack::CompressedTrainingDataEntryReader::new(BufReader::new(file)).unwrap();

    let mut boards = vec![];
    let mut it = 0;
    while it < iter {
        let entry = reader.next();
        if !rand::random_bool(1.0 / 20.0) {
            continue;
        }
        if !filter(&entry) {
            continue;
        }

        it += 1;

        let board = Board::from_fen(&entry.pos.fen().unwrap(), false).unwrap();
        boards.push(board);
    }

    boards
}
fn benchmark(mut net: NNUE, boards: &Vec<Board>) -> f64 {
    let mut sparseness = 0;
    for board in boards.iter() {
        net.init(board);
        net.sort_eval(board);

        let ft = net.sort_ft();
        for i in (0..HL_NO_PST).step_by(4) {
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

    sparseness as f64 / (HL_NO_PST / 4 * boards.len()) as f64
}

pub fn compute_co_occurrence_mapping(path: &str, iter: usize) -> [usize; HL_NO_PST] {
    let file = File::open(path).unwrap();
    let mut reader =
        sfbinpack::CompressedTrainingDataEntryReader::new(BufReader::new(file)).unwrap();

    let half_hl = HL_NO_PST / 2;

    let mut net = NNUE::new();
    let mut co_matrix = vec![0u64; half_hl * half_hl];
    let mut counts = vec![0u64; half_hl];

    // Collect co-occurrence statistics for 0..HL_NO_PST / 2
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
                counts[i] += 1;
            }
        }

        // Increment symmetric co-occurrence pairs
        let len = active.len();
        for i in 0..len {
            let a = active[i];
            for j in (i + 1)..len {
                let b = active[j];
                co_matrix[a * half_hl + b] += 1;
                co_matrix[b * half_hl + a] += 1;
            }
        }
    }

    // Initialize mapping with identity mapping for the full array
    let mut mapping = [0usize; HL_NO_PST];
    for i in 0..HL_NO_PST {
        mapping[i] = i;
    }

    // Greedy 4-element block packing strictly on 0..HL_NO_PST / 2
    let mut used = vec![false; half_hl];
    let mut write_head = 0;

    while write_head < half_hl {
        // Pick unmapped neuron with highest activity to anchor the block
        let mut seed = None;
        let mut max_count = 0;
        for i in 0..half_hl {
            if !used[i] && (seed.is_none() || counts[i] > max_count) {
                max_count = counts[i];
                seed = Some(i);
            }
        }

        let seed_idx = match seed {
            Some(idx) => idx,
            None => break,
        };

        let mut block = Vec::with_capacity(4);
        block.push(seed_idx);
        used[seed_idx] = true;

        // Fill remaining slots in the 4-element block with highest shared co-occurrence
        while block.len() < 4 && write_head + block.len() < half_hl {
            let mut best_candidate = None;
            let mut max_co = 0;

            for candidate in 0..half_hl {
                if used[candidate] {
                    continue;
                }

                // Sum co-occurrence with all current block members
                let co_sum: u64 = block
                    .iter()
                    .map(|&b| co_matrix[candidate * half_hl + b])
                    .sum();

                if best_candidate.is_none() || co_sum > max_co {
                    max_co = co_sum;
                    best_candidate = Some(candidate);
                }
            }

            if let Some(candidate_idx) = best_candidate {
                block.push(candidate_idx);
                used[candidate_idx] = true;
            } else {
                break;
            }
        }

        // Write permuted block into mapping
        for &neuron in &block {
            mapping[write_head] = neuron;
            write_head += 1;
        }
    }

    mapping
}

pub fn start(path: &str, iter: usize) {
    let boards = get_boards(path, iter);
    let net = NNUE::new();
    println!("starting raw sparseness {}", benchmark(net, &boards));
    let net = NNUE::build(&Permute::load());
    let baseline = benchmark(net, &boards);
    println!("starting sparseness {}", baseline);

    // let mut local_best = baseline;
    // let mut mapping = Permute::load().mapping;

    let mapping = compute_co_occurrence_mapping(path, iter);
    // let mut rng = rand::rng();
    // for it in 0..10000 {
    //     if it % 100 == 0 {
    //         println!("iter {}, best {}, old {}", it, local_best, baseline);
    //     }

    //     let mut new_mapping = mapping;
    //     let mut a = 0;
    //     let mut b = 0;
    //     loop {
    //         a = rng.next_u64() as usize % (HL_NO_PST / 2);
    //         b = rng.next_u64() as usize % (HL_NO_PST / 2);
    //         if a != b {
    //             break;
    //         }
    //     }

    //     new_mapping[a] = mapping[b];
    //     new_mapping[b] = mapping[a];

    //     let net = NNUE::build(&Permute::new(new_mapping.clone()));
    //     let score = benchmark(net, &boards);
    //     if score > local_best {
    //         mapping = new_mapping;
    //         local_best = score;
    //         Permute::new(mapping).save();
    //         println!("improve {}", score);
    //     }
    // }

    let net = NNUE::build(&Permute::new(mapping));
    println!("ending sparseness {}", benchmark(net, &boards));

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
