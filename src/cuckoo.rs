use cozy_chess::{BitBoard, Board, Color, Move, Piece, Square};

use crate::{
    ext::{ExtBoard, ExtMove, ZOBRIST},
    stack::KeyStack,
};

use std::sync::OnceLock;

static KEYS: OnceLock<[u64; 8192]> = OnceLock::new();
static MOVES: OnceLock<[Move; 8192]> = OnceLock::new();

#[inline(always)]
fn h1(key: u64) -> usize {
    (key & 0x1fff) as usize
}

#[inline(always)]
fn h2(key: u64) -> usize {
    ((key >> 16) & 0x1fff) as usize
}

pub fn init() {
    let mut keys = [0u64; 8192];
    let mut moves = [Move::NULL_MOVE; 8192];
    let mut count = 0;

    let pieces = [
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ];

    let colors = [Color::White, Color::Black];

    for piece in pieces {
        for color in colors {
            for s1 in 0..64 {
                for s2 in (s1 + 1)..64 {
                    let s1 = Square::ALL[s1];
                    let s2 = Square::ALL[s2];

                    let attacks = match piece {
                        Piece::Pawn => unreachable!(),
                        Piece::Knight => cozy_chess::get_knight_moves(s1),
                        Piece::Bishop => cozy_chess::get_bishop_moves(s1, BitBoard::EMPTY),
                        Piece::Rook => cozy_chess::get_rook_moves(s1, BitBoard::EMPTY),
                        Piece::Queen => {
                            cozy_chess::get_bishop_moves(s1, BitBoard::EMPTY)
                                | cozy_chess::get_rook_moves(s1, BitBoard::EMPTY)
                        }
                        Piece::King => cozy_chess::get_king_moves(s1),
                    };

                    if attacks.has(s2) {
                        let mut m = Move {
                            from: s1,
                            to: s2,
                            promotion: None,
                        };
                        let mut key = ZOBRIST.color[color as usize].pieces[piece as usize]
                            [s1 as usize]
                            ^ ZOBRIST.color[color as usize].pieces[piece as usize][s2 as usize]
                            ^ ZOBRIST.black_to_move;

                        let mut slot = h1(key);

                        loop {
                            let tmp_key = keys[slot];
                            let tmp_m = moves[slot];

                            keys[slot] = key;
                            moves[slot] = m;

                            key = tmp_key;
                            m = tmp_m;

                            if m.is_null() {
                                break;
                            }

                            if slot == h1(key) {
                                slot = h2(key);
                            } else {
                                slot = h1(key);
                            }
                        }

                        count += 1;
                    }
                }
            }
        }
    }

    if count != 3668 {
        panic!("rip cuckoo");
    }

    KEYS.set(keys).unwrap();
    MOVES.set(moves).unwrap();
}

pub fn is_upcoming_rep(pos: &Board, stack: &KeyStack, ply: i8) -> bool {
    let keys = KEYS.get().unwrap();
    let moves = MOVES.get().unwrap();

    let occ = pos.occupied();
    let max_dist = std::cmp::min(pos.halfmove_clock() as usize, stack.head);
    let pos_hash = pos.correct_hash();
    for i in (3..=max_dist).step_by(2) {
        let move_key = pos_hash ^ stack.keys[stack.head - i];

        let mut hash = h1(move_key);
        if keys[hash] != move_key {
            hash = h2(move_key);
        }

        if keys[hash] != move_key {
            continue; // neither slot matches
        }

        let m = moves[hash];
        let (from, to) = if pos.color_on(m.from) == Some(pos.side_to_move()) {
            (m.from, m.to)
        } else if pos.color_on(m.to) == Some(pos.side_to_move()) {
            (m.to, m.from)
        } else {
            continue;
        };

        // obstructed
        if !((cozy_chess::get_between_rays(from, to) | to.bitboard()) & occ).is_empty() {
            continue;
        }

        if ply > i as i8 {
            return true;
        }

        for j in ((i + 4)..=max_dist).step_by(2) {
            if stack.keys[stack.head - j] == stack.keys[stack.head - i] {
                return true;
            }
        }
    }

    false
}
