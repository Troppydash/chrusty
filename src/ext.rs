use arrayvec::ArrayVec;
use cozy_chess::{Board, Move, Piece, Square};

use crate::param::NONE_PIECE_INDEX;

// these are stack allocated

pub const MAX_MOVES: usize = 218;
pub type ScoredMoveList = ArrayVec<ScoredMove, MAX_MOVES>;
pub type MoveList = ArrayVec<Move, MAX_MOVES>;

pub trait ExtMove {
    const NULL_MOVE: Move;
    const NULL_MOVE_BITS: u16;

    fn is_null(&self) -> bool;
    fn to_uci(&self, board: &Board) -> String;
    fn from_uci(uci: &str, board: &Board) -> Move;

    fn to_bits(&self) -> u16;
    fn from_bits(value: u16) -> Move;
}

impl ExtMove for Move {
    const NULL_MOVE: Move = Move {
        from: cozy_chess::Square::A1,
        to: cozy_chess::Square::A1,
        promotion: None,
    };
    const NULL_MOVE_BITS: u16 = 6 << 12;

    fn is_null(&self) -> bool {
        *self == Self::NULL_MOVE
    }

    fn to_uci(&self, board: &Board) -> String {
        format!("{}", cozy_chess::util::display_uci_move(board, *self))
    }

    fn from_uci(uci: &str, board: &Board) -> Move {
        cozy_chess::util::parse_uci_move(board, uci).unwrap()
    }

    fn to_bits(&self) -> u16 {
        let from = self.from as u16;
        let to = self.to as u16;
        let promotion = match self.promotion {
            None => 6,
            Some(piece) => piece as u16,
        };

        // 6 bits + 6 bits + 3 bits = 15 bits
        from | (to << 6) | (promotion << 12)
    }

    fn from_bits(value: u16) -> Move {
        let from = value & (0b111111);
        let to = (value >> 6) & (0b111111);
        let promotion = value >> 12;

        Move {
            from: Square::ALL[from as usize],
            to: Square::ALL[to as usize],
            promotion: match promotion {
                6 => None,
                _ => Some(Piece::ALL[promotion as usize]),
            },
        }
    }
}

pub trait ExtBoard {
    fn in_check(&self) -> bool;
    fn any_moves(&self) -> bool;
    fn is_capture(&self, m: &Move) -> bool;
    fn get_captured(&self, m: &Move) -> Piece;
    fn get_captured_index(&self, m: &Move) -> usize;
    fn get_from_index(&self, m: &Move) -> usize;

    fn get_legal_moves(&self) -> MoveList;
}

impl ExtBoard for Board {
    fn in_check(&self) -> bool {
        !self.checkers().is_empty()
    }

    fn any_moves(&self) -> bool {
        self.generate_moves(|_m| true)
    }

    fn is_capture(&self, m: &Move) -> bool {
        !self.piece_on(m.to).is_none()
    }

    fn get_captured(&self, m: &Move) -> Piece {
        self.piece_on(m.to).unwrap()
    }

    fn get_captured_index(&self, m: &Move) -> usize {
        match self.piece_on(m.to) {
            Some(piece) => piece as usize,
            None => NONE_PIECE_INDEX,
        }
    }

    fn get_from_index(&self, m: &Move) -> usize {
        self.piece_on(m.from).unwrap() as usize
    }

    fn get_legal_moves(&self) -> MoveList {
        let mut ml = MoveList::new();
        self.generate_moves(|moves| {
            for m in moves {
                ml.push(m)
            }
            false
        });

        ml
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScoredMove {
    pub inner: Move,
    pub score: i16,
}

impl ScoredMove {
    pub const NULL_MOVE: ScoredMove = ScoredMove {
        inner: Move::NULL_MOVE,
        score: 0,
    };

    pub fn new(inner: Move, score: i16) -> Self {
        Self { inner, score }
    }

    pub fn from_move(inner: Move) -> Self {
        Self::new(inner, 0)
    }

    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }
}
