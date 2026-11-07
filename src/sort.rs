use std::{fs::File, io::BufReader};

use cozy_chess::Board;
use sfbinpack::TrainingDataEntry;

use crate::nnue::{
    NNUE,
    network::{HL, Permute},
};

fn filter(entry: &TrainingDataEntry) -> bool {
    !entry.pos.is_checked(entry.pos.side_to_move()) && entry.ply > 8
}

fn get_boards(path: &str, iter: usize) -> (Vec<Board>, Vec<Board>) {
    let file = File::open(path).unwrap();
    let mut reader =
        sfbinpack::CompressedTrainingDataEntryReader::new(BufReader::new(file)).unwrap();

    let mut train = Vec::with_capacity(iter);
    let mut validation = Vec::with_capacity(iter);
    while reader.has_next() && validation.len() < iter {
        let entry = reader.next();
        if !rand::random_bool(1.0 / 20.0) || !filter(&entry) {
            continue;
        }

        let board = Board::from_fen(&entry.pos.fen().unwrap(), false).unwrap();
        if train.len() == validation.len() {
            train.push(board);
        } else {
            validation.push(board);
        }
    }

    assert_eq!(
        validation.len(),
        iter,
        "not enough sampled positions: got {} training and {} validation positions",
        train.len(),
        validation.len()
    );
    (train, validation)
}

fn benchmark(mut net: NNUE, boards: &[Board]) -> f64 {
    let mut zeros = 0;
    for board in boards {
        net.init(board);
        net.sort_eval(board);

        for i in (0..HL).step_by(4) {
            if (0..4).all(|k| net.sort_ft()[i + k] == 0) {
                zeros += 1;
            }
        }
    }

    zeros as f64 / (HL / 4 * boards.len()) as f64
}

/// Returns [neuron][lo-sample] = hi-sample mask
fn collect_activity(boards: &[Board]) -> Vec<Vec<u64>> {
    let half_hl = HL / 2;
    let words = boards.len().checked_mul(2).unwrap().div_ceil(64);
    let mut activity = vec![vec![0u64; words]; half_hl];
    let mut net = NNUE::new();

    for (sample, board) in boards.iter().enumerate() {
        net.init(board);
        net.sort_eval(board);
        let ft = net.sort_ft();

        // Both perspectives use the same permutation, but own separate bits.
        for side in 0..2 {
            let bit = sample * 2 + side;
            for i in 0..half_hl {
                if ft[side * half_hl + i] > 0 {
                    activity[i][bit / 64] |= 1u64 << (bit % 64);
                }
            }
        }
    }

    activity
}

fn greedy_blocks(activity: &[Vec<u64>]) -> Vec<[usize; 4]> {
    let counts: Vec<usize> = activity
        .iter()
        .map(|row| row.iter().map(|word| word.count_ones() as usize).sum())
        .collect();
    let mut used = vec![false; activity.len()];
    let mut blocks = Vec::with_capacity(activity.len() / 4);

    while blocks.len() * 4 < activity.len() {
        let seed = (0..activity.len())
            .filter(|&i| !used[i])
            .max_by_key(|&i| counts[i])
            .unwrap();
        let mut block = [seed; 4];
        let mut union = activity[seed].clone();
        used[seed] = true;

        for slot in 1..4 {
            let mut best: Option<(usize, usize, usize)> = None;
            for i in 0..activity.len() {
                if used[i] {
                    continue;
                }

                let added: usize = union
                    .iter()
                    .zip(&activity[i])
                    .map(|(&old, &new)| (new & !old).count_ones() as usize)
                    .sum();
                let key = (added, usize::MAX - counts[i], i);
                if best.is_none_or(|old| key < old) {
                    best = Some(key);
                }
            }

            let (_, _, neuron) = best.unwrap();
            block[slot] = neuron;
            used[neuron] = true;
            for (word, &active) in union.iter_mut().zip(&activity[neuron]) {
                *word |= active;
            }
        }

        blocks.push(block);
    }

    blocks
}

struct Block {
    neurons: [usize; 4],
    // unions[mask] caches the activity of the neurons selected by mask.
    unions: Vec<Vec<u64>>,
    score: usize,
}

impl Block {
    fn new(neurons: [usize; 4], activity: &[Vec<u64>]) -> Self {
        let mut block = Self {
            neurons,
            unions: vec![vec![0; activity[0].len()]; 16],
            score: 0,
        };
        block.refresh(activity);
        block
    }

    fn refresh(&mut self, activity: &[Vec<u64>]) {
        for mask in 1usize..16 {
            let slot = mask.trailing_zeros() as usize;
            let rest = mask & (mask - 1);
            for word in 0..activity[0].len() {
                self.unions[mask][word] =
                    self.unions[rest][word] | activity[self.neurons[slot]][word];
            }
        }
        self.score = self.unions[15]
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum();
    }
}

fn improve_blocks(neurons: Vec<[usize; 4]>, activity: &[Vec<u64>]) -> Vec<Block> {
    let mut blocks: Vec<Block> = neurons
        .into_iter()
        .map(|neurons| Block::new(neurons, activity))
        .collect();

    loop {
        let mut improved = false;
        for a in 0..blocks.len() {
            for b in a + 1..blocks.len() {
                let mut best_score = blocks[a].score + blocks[b].score;
                let mut best_swap = None;

                // Try every distinct split of these eight neurons into two blocks.
                // Single swaps cover 16 splits, double swaps cover another 18.
                for mask_a in 1usize..15 {
                    let count = mask_a.count_ones();
                    if count > 2 || (count == 2 && mask_a & 1 == 0) {
                        continue;
                    }
                    for mask_b in 1usize..15 {
                        if mask_b.count_ones() != count {
                            continue;
                        }

                        let mut score = 0;
                        for word in 0..activity[0].len() {
                            let new_a = blocks[a].unions[15 ^ mask_a][word]
                                | blocks[b].unions[mask_b][word];
                            let new_b = blocks[b].unions[15 ^ mask_b][word]
                                | blocks[a].unions[mask_a][word];
                            score += new_a.count_ones() as usize + new_b.count_ones() as usize;
                            if score >= best_score {
                                break;
                            }
                        }
                        if score < best_score {
                            best_score = score;
                            best_swap = Some((mask_a, mask_b));
                        }
                    }
                }

                if let Some((mut mask_a, mut mask_b)) = best_swap {
                    while mask_a != 0 {
                        let ia = mask_a.trailing_zeros() as usize;
                        let ib = mask_b.trailing_zeros() as usize;
                        let old_a = blocks[a].neurons[ia];
                        blocks[a].neurons[ia] = blocks[b].neurons[ib];
                        blocks[b].neurons[ib] = old_a;
                        mask_a &= mask_a - 1;
                        mask_b &= mask_b - 1;
                    }
                    blocks[a].refresh(activity);
                    blocks[b].refresh(activity);
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }

    blocks
}

fn compute_co_occurrence_mapping(boards: &[Board], current: &Permute) -> [usize; HL] {
    let activity = collect_activity(boards);
    let mut best = improve_blocks(greedy_blocks(&activity), &activity);
    let mut best_score: usize = best.iter().map(|block| block.score).sum();

    // Keep the existing layout as a second starting point.
    let current_blocks = current.mapping[..HL / 2]
        .chunks_exact(4)
        .map(|block| block.try_into().unwrap())
        .collect();
    let candidate = improve_blocks(current_blocks, &activity);
    let score: usize = candidate.iter().map(|block| block.score).sum();
    if score < best_score {
        best_score = score;
        best = candidate;
    }

    println!(
        "training sparseness {}",
        1.0 - best_score as f64 / ((HL / 4) * boards.len()) as f64
    );

    let mut mapping = [0; HL];
    for (i, block) in best.iter().enumerate() {
        for j in 0..4 {
            mapping[i * 4 + j] = block.neurons[j];
            mapping[HL / 2 + i * 4 + j] = HL / 2 + block.neurons[j];
        }
    }
    mapping
}

pub fn start(path: &str, iter: usize) {
    assert!(iter > 0, "position count must be greater than zero");
    assert!(HL % 8 == 0, "hidden layer must contain whole paired blocks");

    let (train, validation) = get_boards(path, iter);
    let current = Permute::load();
    println!("info raw sparseness {}", benchmark(NNUE::new(), &train));
    println!(
        "info start sparseness {}",
        benchmark(NNUE::build(&current), &train)
    );

    let mapping = compute_co_occurrence_mapping(&train, &current);
    let candidate = Permute::new(mapping);
    println!(
        "info end sparseness {}",
        benchmark(NNUE::build(&candidate), &train)
    );

    let baseline = benchmark(NNUE::build(&current), &validation);
    let score = benchmark(NNUE::build(&candidate), &validation);
    println!("info start validation sparseness {}", baseline);
    println!("info end validation sparseness {}", score);

    if score > baseline {
        candidate.save();
    } else {
        println!("validation sparseness did not improve, keeping current permutation");
    }
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
