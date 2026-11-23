use cozy_chess::{BitBoard, Board, Color, File, Piece};

pub const PAWN_LAYOUTS: usize = 3;
pub const OFFICER_STATES: usize = 2;
pub const QUEEN_STATES: usize = 4;
pub const RAW_BUCKETS: usize = 8 * QUEEN_STATES * PAWN_LAYOUTS * OFFICER_STATES;

const OUTPUT_BUCKETS: [u8; 33] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, //
    1, 1, 1, 1, //
    2, 2, 2, 2, //
    3, 3, 3, //
    4, 4, 4, //
    5, 5, 5, //
    6, 6, 6, //
    7, 7, 7, 7, //
];

const fn raw_bucket(material: u8, queens: u8, pawns: u8, officers: u8) -> u8 {
    ((material * QUEEN_STATES as u8 + queens) * PAWN_LAYOUTS as u8 + pawns) * OFFICER_STATES as u8
        + officers
}

const fn raw_bucket_keys(bucket: u8) -> (u8, u8, u8, u8) {
    let o = bucket % OFFICER_STATES as u8;
    let rest = bucket / OFFICER_STATES as u8;
    let p = rest % PAWN_LAYOUTS as u8;
    let rest = rest / PAWN_LAYOUTS as u8;
    (rest / 4, rest % 4, p, o)
}

fn raw_bucket_map(board: &Board) -> u8 {
    let stm_queen = !(board.colored_pieces(board.side_to_move(), Piece::Queen)).is_empty();
    let ntm_queen = !(board.colored_pieces(!board.side_to_move(), Piece::Queen)).is_empty();

    let q = 2 * stm_queen as u8 + ntm_queen as u8;

    const LEFT_HALF: u64 =
        File::A.bitboard().0 | File::B.bitboard().0 | File::C.bitboard().0 | File::D.bitboard().0;
    const RIGHT_HALF: u64 = BitBoard::FULL.0 & !LEFT_HALF;

    let pawns = board.pieces(Piece::Pawn).0;
    let pawns_one_half_only = (pawns & LEFT_HALF) == pawns || (pawns & RIGHT_HALF) == pawns;
    let p = if pawns.count_ones() == 0 {
        2
    } else if pawns_one_half_only {
        1
    } else {
        0
    };

    let officers = board.occupied() - board.pieces(Piece::King) - board.pieces(Piece::Pawn);
    let at_least_2 = (officers & board.colors(Color::White))
        .len()
        .max((officers & board.colors(Color::Black)).len())
        >= 2;
    let o = at_least_2 as u8;

    let m = OUTPUT_BUCKETS[board.occupied().len() as usize];
    raw_bucket(m, q, p, o)
}

// everything except the 32 most frequent raw buckets (20M sample, 2026-11-22);
// the 160 dropped hold ~2.6% of training positions (107 never occur)
const LOW_FREQ_COUNT: usize = 160;

#[rustfmt::skip]
const LOW_FREQ_BUCKETS: [u8; LOW_FREQ_COUNT] = [
    6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 19, 21, 22, 23,
    28, 29, 30, 32, 33, 34, 35, 36, 38, 39, 40, 41, 44, 46, 47, 50,
    52, 53, 54, 56, 57, 58, 59, 60, 62, 63, 64, 65, 66, 68, 69, 70,
    71, 72, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87,
    88, 89, 90, 92, 93, 94, 95, 96, 98, 99, 100, 101, 102, 103, 104, 105,
    106, 107, 108, 109, 110, 111, 112, 113, 114, 116, 117, 118, 119, 120, 122, 123,
    124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 140,
    141, 142, 143, 144, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157,
    158, 159, 160, 161, 162, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174,
    175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 188, 189, 190, 191,
];

const IS_LOW_FREQ: [bool; RAW_BUCKETS] = {
    let mut out = [false; RAW_BUCKETS];
    let mut k = 0;
    while k < LOW_FREQ_COUNT {
        assert!(
            !out[LOW_FREQ_BUCKETS[k] as usize],
            "duplicate low-frequency bucket"
        );
        out[LOW_FREQ_BUCKETS[k] as usize] = true;
        k += 1;
    }
    out
};

const fn is_low_freq(bucket: u8) -> bool {
    IS_LOW_FREQ[bucket as usize]
}

const fn abs_diff(a: u8, b: u8) -> u32 {
    (if a > b { a - b } else { b - a }) as u32
}

const fn remap_cost(from: u8, to: u8) -> u32 {
    let (m, q, p, o) = raw_bucket_keys(from);
    let (m2, q2, p2, o2) = raw_bucket_keys(to);

    let dq = match (q, q2) {
        _ if q == q2 => 0,
        (1 | 2, 3) => 10,
        (1 | 2, 0) => 12,
        (1, 2) | (2, 1) => 200,
        (0, 3) | (3, 0) => 150,
        _ => 300, // QQ / no queens -> imbalance
    };
    // 0: both halves, 1: one half, 2: no pawns
    let dp = match (p, p2) {
        _ if p == p2 => 0,
        (0, 1) | (1, 0) => 3,
        (1, 2) | (2, 1) => 4,
        _ => 6,
    };
    let do_ = if o == o2 { 0 } else { 2 };

    100 * abs_diff(m, m2) + dq + dp + do_
}

const RAW_REMAP: [u8; RAW_BUCKETS] = {
    let mut out = [0; RAW_BUCKETS];
    let mut i = 0;
    while i < RAW_BUCKETS {
        if !is_low_freq(i as u8) {
            out[i] = i as u8;
        } else {
            let mut best = u8::MAX;
            let mut best_cost = u32::MAX;
            let mut j = 0;
            while j < RAW_BUCKETS {
                if !is_low_freq(j as u8) {
                    let c = remap_cost(i as u8, j as u8);
                    if c < best_cost {
                        best_cost = c;
                        best = j as u8;
                    }
                }
                j += 1;
            }
            assert!(best != u8::MAX, "no kept bucket");
            out[i] = best;
        }
        i += 1;
    }
    out
};

/// Raw bucket -> compacted output bucket in 0..CustomBuckets::BUCKETS.
pub const COMPACTED_BUCKETS_MAP: [u8; RAW_BUCKETS] = {
    // rank[r] = index of raw bucket r among the kept buckets
    let mut rank = [u8::MAX; RAW_BUCKETS];
    let mut next = 0u8;
    let mut r = 0;
    while r < RAW_BUCKETS {
        if !is_low_freq(r as u8) {
            rank[r] = next;
            next += 1;
        }
        r += 1;
    }
    assert!(next as usize == RAW_BUCKETS - LOW_FREQ_COUNT);

    let mut out = [0; RAW_BUCKETS];
    let mut i = 0;
    while i < RAW_BUCKETS {
        out[i] = rank[RAW_REMAP[i] as usize];
        assert!(out[i] != u8::MAX);
        i += 1;
    }
    out
};

pub const BUCKET_COUNT: usize = RAW_BUCKETS - LOW_FREQ_COUNT;

pub fn get_output_bucket(board: &Board) -> usize {
    COMPACTED_BUCKETS_MAP[raw_bucket_map(board) as usize] as usize
}
