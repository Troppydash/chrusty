use cozy_chess::{BitBoard, Color, Piece, Rank, Square};
use std::sync::OnceLock;

const NUM_ATTACKER_PIECES: usize = 5;
const NUM_TARGET_PIECES: usize = 5;

const ATTACKER_PIECES: [Piece; NUM_ATTACKER_PIECES] = [
    Piece::Pawn,
    Piece::Knight,
    Piece::Bishop,
    Piece::Rook,
    Piece::Queen,
];

#[rustfmt::skip]
// [attacker][target]
// pawn only targets N+R
// B+R don't target Q
const TARGET_MAP: [[i8; 6]; 6] = [
//    P   N   B   R   Q   K
    [-1,  0, -1,  1, -1, -1], // P
    [ 0,  1,  2,  3,  4, -1], // K
    [ 0,  1,  2,  3, -1, -1], // B
    [ 0,  1,  2,  3, -1, -1], // R
    [ 0,  1,  2,  3,  4, -1], // Q
    [-1, -1, -1, -1, -1, -1], // K
];

const fn count_targets(attacker: usize) -> usize {
    let mut n = 0;
    let mut t = 0;
    while t < 6 {
        if TARGET_MAP[attacker][t] >= 0 {
            n += 1;
        }
        t += 1;
    }

    n
}

pub const NUM_TARGETS: [usize; NUM_ATTACKER_PIECES] = [
    count_targets(0),
    count_targets(1),
    count_targets(2),
    count_targets(3),
    count_targets(4),
];

// sum(pos * attacks) for all pieces
const GEO_COUNTS: [usize; NUM_ATTACKER_PIECES] = [84, 336, 560, 896, 1456];
const fn half_size() -> usize {
    let mut total = 0;
    let mut i = 0;
    while i < NUM_ATTACKER_PIECES {
        total += GEO_COUNTS[i] * NUM_TARGETS[i];
        i += 1;
    }

    total
}

const HALF_THREATS: usize = half_size();
const THREATS: usize = 2 * HALF_THREATS;
pub const FULL_THREATS: usize = 2 * THREATS;

fn geometric_attacks(piece: Piece, sq: Square, attacker_is_us: bool) -> BitBoard {
    match piece {
        Piece::Pawn => {
            if matches!(sq.rank(), Rank::First | Rank::Eighth) {
                BitBoard::EMPTY
            } else {
                if attacker_is_us {
                    cozy_chess::get_pawn_attacks(sq, Color::White)
                } else {
                    cozy_chess::get_pawn_attacks(sq, Color::Black)
                }
            }
        }
        Piece::Knight => cozy_chess::get_knight_moves(sq),
        Piece::Bishop => cozy_chess::get_bishop_moves(sq, BitBoard::EMPTY),
        Piece::Rook => cozy_chess::get_rook_moves(sq, BitBoard::EMPTY),
        Piece::Queen => {
            cozy_chess::get_bishop_moves(sq, BitBoard::EMPTY)
                | cozy_chess::get_rook_moves(sq, BitBoard::EMPTY)
        }
        Piece::King => unreachable!(),
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    attacks: BitBoard,
    base: u32,
    stride: u32,
}

impl Entry {
    const EMPTY: Self = Self {
        attacks: BitBoard::EMPTY,
        base: 0,
        stride: 0,
    };
}

#[derive(Debug)]
pub struct ThreatLut {
    // [attacker_is_us][attacker][from]
    index: [[[Entry; 64]; NUM_ATTACKER_PIECES]; 2],
}

impl ThreatLut {
    fn build() -> Self {
        let mut index = [[[Entry::EMPTY; 64]; NUM_ATTACKER_PIECES]; 2];
        let mut running = 0;

        for half in 0..2 {
            let attacker_is_us = half == 0;

            for (pi, &piece) in ATTACKER_PIECES.iter().enumerate() {
                let stride = NUM_TARGETS[pi];
                for from in Square::ALL {
                    let attacks = geometric_attacks(piece, from, attacker_is_us);
                    index[half][pi][from as usize] = Entry {
                        attacks,
                        base: running as u32,
                        stride: stride as u32,
                    };

                    let n = attacks.len() as usize;
                    running += stride * n;
                }
            }
            assert_eq!(running, (half + 1) * HALF_THREATS);
        }

        Self { index }
    }

    // outputs [0, THREATS)
    #[inline]
    fn index(
        &self,
        half: usize,
        attacker_pi: usize,
        from: Square,
        to: Square,
        target_slot: usize,
    ) -> usize {
        let e = &self.index[half][attacker_pi][from as usize];
        debug_assert!(e.attacks.has(to), "{attacker_pi} {from} {to} {target_slot}");

        let below = e.attacks.0 & ((1u64 << (to as u32)) - 1);
        e.base as usize + below.count_ones() as usize * e.stride as usize + target_slot
    }
}

static THREAT_LUT: OnceLock<ThreatLut> = OnceLock::new();

fn threat_lut() -> &'static ThreatLut {
    THREAT_LUT.get_or_init(ThreatLut::build)
}

// outputs [0..FULL_THREATS)
#[inline]
pub fn threat_feature_index(
    attacker_is_us: bool,
    is_attack: bool,
    attacker_pi: usize,
    from: usize,
    to: usize,
    target_pi: usize,
) -> usize {
    let slot = TARGET_MAP[attacker_pi][target_pi];
    debug_assert!(slot >= 0);

    let index = threat_lut().index(
        (!attacker_is_us) as usize,
        attacker_pi,
        Square::ALL[from],
        Square::ALL[to],
        slot as usize,
    );
    if is_attack {
        index
    } else {
        index + THREATS
    }
}

#[inline]
pub const fn is_feature(attacker: Piece, target: Piece) -> bool {
    TARGET_MAP[attacker as usize][target as usize] >= 0
}
