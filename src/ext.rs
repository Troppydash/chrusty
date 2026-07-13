use arrayvec::ArrayVec;
use cozy_chess::{Board, Move, Piece};

use crate::param::NONE_PIECE_INDEX;

// these are stack allocated

pub const MAX_MOVES: usize = 218;
pub type ScoredMoveList = ArrayVec<ScoredMove, MAX_MOVES>;
pub type MoveList = ArrayVec<Move, MAX_MOVES>;

pub trait ExtMove {
    const NULL_MOVE: Move;

    fn is_null(&self) -> bool;
    fn to_uci(&self, board: &Board) -> String;
    fn from_uci(uci: &str, board: &Board) -> Move;
}

impl ExtMove for Move {
    const NULL_MOVE: Move = Move {
        from: cozy_chess::Square::A1,
        to: cozy_chess::Square::A1,
        promotion: None,
    };

    fn is_null(&self) -> bool {
        *self == Self::NULL_MOVE
    }

    fn to_uci(&self, board: &Board) -> String {
        format!("{}", cozy_chess::util::display_uci_move(board, *self))
    }

    fn from_uci(uci: &str, board: &Board) -> Move {
        cozy_chess::util::parse_uci_move(board, uci).unwrap()
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
        let mut any = false;
        self.generate_moves(|moves| {
            if !moves.is_empty() {
                any = true;
            }

            true
        });

        any
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
