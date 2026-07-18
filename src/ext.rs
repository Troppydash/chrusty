use arrayvec::ArrayVec;
use cozy_chess::{
    Board,
    Color::{Black, White},
    Move,
    Piece::{self, Queen},
    Rank, Square,
};

use crate::ext::MoveType::{CASTLE, ENPASSENT, NONE, NORMAL};

// these are stack allocated

pub const MAX_MOVES: usize = 218;
pub type ScoredMoveList = ArrayVec<ScoredMove, MAX_MOVES>;
pub type MoveList = ArrayVec<Move, MAX_MOVES>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MoveType {
    NORMAL,
    ENPASSENT,
    CASTLE,
    PROMOTION,
    NONE,
}

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

    fn get_captured(&self, m: &Move) -> Piece;
    fn is_quiet(&self, m: &Move) -> bool;
    fn ep_square(&self) -> Option<Square>;
    fn is_ep(&self, m: &Move) -> bool;
    fn is_castle(&self, m: &Move) -> bool;

    fn get_legal_moves(&self) -> MoveList;

    fn move_type(&self, m: &Move) -> MoveType;
}

impl ExtBoard for Board {
    fn in_check(&self) -> bool {
        !self.checkers().is_empty()
    }

    fn any_moves(&self) -> bool {
        self.generate_moves(|_m| true)
    }

    fn get_captured(&self, m: &Move) -> Piece {
        // queen promotions treated as pawn capture
        match self.piece_on(m.to) {
            Some(piece) => piece,
            None => {
                assert!(m.promotion == Some(Queen) || self.is_ep(m));
                Piece::Pawn
            }
        }
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

    fn is_quiet(&self, m: &Move) -> bool {
        // special moves are CASTLE, PROMOTION, ENPASSENT
        if self.is_ep(m) {
            return false;
        }

        if self.is_castle(m) {
            return true;
        }

        // a quiet move is not a capture and not a queen promotion
        self.piece_on(m.to).is_none() && m.promotion != Some(Piece::Queen)
    }

    fn ep_square(&self) -> Option<Square> {
        match self.en_passant() {
            Some(file) => {
                let ep_rank = match self.side_to_move() {
                    White => Rank::Sixth,
                    Black => Rank::Third,
                };
                Some(Square::new(file, ep_rank))
            }
            None => None,
        }
    }

    fn is_ep(&self, m: &Move) -> bool {
        self.piece_on(m.from) == Some(Piece::Pawn) && self.ep_square() == Some(m.to)
    }

    fn is_castle(&self, m: &Move) -> bool {
        self.piece_on(m.to) == Some(Piece::Rook) && self.color_on(m.to) == Some(self.side_to_move())
    }

    fn move_type(&self, m: &Move) -> MoveType {
        if m.is_null() {
            return NONE;
        }

        if self.is_ep(m) {
            return ENPASSENT;
        }

        if self.is_castle(m) {
            return CASTLE;
        }

        return NORMAL;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScoredMove {
    pub inner: Move,
    pub score: i32,
}

impl ScoredMove {
    pub const NULL_MOVE: ScoredMove = ScoredMove {
        inner: Move::NULL_MOVE,
        score: 0,
    };

    pub fn new(inner: Move, score: i32) -> Self {
        Self { inner, score }
    }

    pub fn from_move(inner: Move) -> Self {
        Self::new(inner, 0)
    }

    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }
}
