use std::{
    fs::File,
    io::{BufReader, Read, Write},
    path::Path,
};

use cozy_chess::Board;
use sfbinpack::{
    TrainingDataEntry,
    chess::{r#move::MoveType, piecetype::PieceType},
};

use crate::nnue::{HL_NO_PST, NNUE};

fn filter(entry: &TrainingDataEntry) -> bool {
    entry.ply >= 22
        && !entry.pos.is_checked(entry.pos.side_to_move())
        && entry.score.abs() < 20000
        && entry.mv.mtype() == MoveType::Normal
        && entry.pos.piece_at(entry.mv.to()).piece_type() == PieceType::None
}

fn benchmark(mut net: NNUE, file: &str, iter: usize) -> f64 {
    let file = File::open(file).unwrap();
    let mut reader =
        sfbinpack::CompressedTrainingDataEntryReader::new(BufReader::new(file)).unwrap();

    let mut sparseness = 0;

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

    sparseness as f64 / (HL_NO_PST / 4 * iter) as f64
}

pub fn start(path: &str, iter: usize) {
    println!("{}", permute_name());
    let net = NNUE::new();
    println!("starting sparseness {}", benchmark(net, path, iter));

    let file = File::open(path).unwrap();
    let mut reader =
        sfbinpack::CompressedTrainingDataEntryReader::new(BufReader::new(file)).unwrap();

    let mut net = NNUE::new();
    let mut counts = [0; HL_NO_PST];

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
        for i in 0..HL_NO_PST {
            if ft[i] > 0 {
                counts[i] += 1;
            }
        }
    }

    let mut index = [0; HL_NO_PST];
    for i in 0..HL_NO_PST {
        index[i] = i;
    }

    // TODO: hill climb this
    index[0..(HL_NO_PST / 2)].sort_by_key(|&i| (std::cmp::Reverse(counts[i]), i));
    for i in 0..(HL_NO_PST / 2) {
        index[i + HL_NO_PST / 2] = index[i] + HL_NO_PST / 2;
    }

    // index[i] = j where ith largest count is index j

    let mapping = index;
    let net = NNUE::build(&mapping);
    println!("ending sparseness {}", benchmark(net, path, iter));

    save_permute(&mapping);
}

const fn permute_name() -> &'static str {
    env!("PERMUTE_FILE")
}

pub fn save_permute(mapping: &[usize; HL_NO_PST]) {
    println!("writing to {}", permute_name());
    let mut file = File::create(permute_name()).unwrap();
    let bytes =
        unsafe { std::slice::from_raw_parts(mapping.as_ptr() as *const u8, size_of_val(mapping)) };
    file.write_all(bytes).unwrap();
}

#[cfg(permute_file)]
pub fn load_permute() -> [usize; HL_NO_PST] {
    let data = *include_bytes!(env!("PERMUTE_FILE"));
    unsafe { std::mem::transmute(data) }
}

#[cfg(not(permute_file))]
pub fn load_permute() -> [usize; HL_NO_PST] {
    let mut data = [0; HL_NO_PST];
    for i in 0..HL_NO_PST {
        data[i] = i;
    }
    data
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
        let mut perm_net = NNUE::build(&load_permute());
        for fen in fens {
            let board = Board::from_fen(fen, false).unwrap();
            net.init(&board);
            perm_net.init(&board);

            assert_eq!(net.evaluate(&board), perm_net.evaluate(&board));
        }
    }
}
