use arrayvec::ArrayVec;
use cozy_chess::{Board, Color, Move, Piece};

use crate::{
    ext::{BitBoardExt, ExtBoard, ExtMove, zobrist_pst},
    param::{MAX_DEPTH_USIZE, VALUE_NONE},
};

#[derive(Clone, Debug)]
pub struct PvList {
    moves: ArrayVec<Move, MAX_DEPTH_USIZE>,
}

impl PvList {
    pub fn new() -> Self {
        Self {
            moves: ArrayVec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.moves.clear();
    }

    pub fn set(&mut self, m: &Move, other: &PvList) {
        self.moves.clear();
        self.moves.push(*m);

        for m in other.moves.iter() {
            self.moves.push(*m);
        }
    }

    pub fn pv(&self) -> Move {
        assert!(self.moves.len() > 0);
        return self.moves[0];
    }

    pub fn get(&self, i: usize) -> Move {
        assert!(i < self.moves.len());
        return self.moves[i];
    }

    pub fn get_moves(&self) -> &ArrayVec<Move, MAX_DEPTH_USIZE> {
        &self.moves
    }

    pub fn len(&self) -> usize {
        self.moves.len()
    }
}
#[derive(Clone, Debug)]
pub struct SearchStack {
    pub ply: i8,
    pub m: Move,
    pub pv_list: PvList,
    pub adjusted_static: i16,
    pub tt_pv: bool,
    pub excluded: Move,
    pub conseq_checks: i32,
    pub verify_null: bool,
}

impl SearchStack {
    pub fn new() -> Self {
        Self {
            ply: 0,
            m: Move::NULL_MOVE,
            pv_list: PvList::new(),
            adjusted_static: VALUE_NONE,
            tt_pv: false,
            excluded: Move::NULL_MOVE,
            conseq_checks: 0,
            verify_null: false,
        }
    }

    pub fn new_ply(ply: i8) -> Self {
        Self {
            ply,
            m: Move::NULL_MOVE,
            pv_list: PvList::new(),
            adjusted_static: VALUE_NONE,
            tt_pv: false,
            excluded: Move::NULL_MOVE,
            conseq_checks: 0,
            verify_null: false,
        }
    }
}

pub struct KeyStack {
    pub keys: [u64; 1000],
    pub head: usize,
}

impl KeyStack {
    pub fn new() -> Self {
        Self {
            keys: [0; 1000],
            head: 0,
        }
    }
    pub fn push(&mut self, key: u64) {
        self.keys[self.head] = key;
        self.head += 1;
    }

    pub fn pop(&mut self) {
        self.head -= 1;
    }

    pub fn clear(&mut self) {
        self.head = 0;
    }
}

pub struct PawnKey {
    pub pawns: [u64; MAX_DEPTH_USIZE],
    pub colored: [[u64; 2]; MAX_DEPTH_USIZE],
    pub size: usize,
}

impl PawnKey {
    pub fn new() -> Self {
        let pawns = [0; MAX_DEPTH_USIZE];
        let colored = [[0; 2]; MAX_DEPTH_USIZE];
        Self {
            pawns,
            colored,
            size: 0,
        }
    }

    pub fn init(&mut self, board: &Board) {
        let mut pawn = 0;
        let mut colored = [0; 2];
        for color in [Color::White, Color::Black] {
            let mut pawns = board.colored_pieces(color, Piece::Pawn);
            while !pawns.is_empty() {
                let sq = pawns.pop();
                pawn ^= zobrist_pst(color, Piece::Pawn, sq);
            }

            let mut pieces = board.colors(color) & !board.colored_pieces(color, Piece::Pawn);
            while !pieces.is_empty() {
                let sq = pieces.pop();
                colored[color as usize] ^= zobrist_pst(color, board.piece_on(sq).unwrap(), sq);
            }
        }

        self.pawns[0] = pawn;
        self.colored[0] = colored;
        self.size = 1;
    }

    pub fn push(&mut self, board: &Board, m: Move) {
        if m.is_null() {
            self.pawns[self.size] = self.pawns[self.size - 1];
            self.colored[self.size] = self.colored[self.size - 1];
            self.size += 1;
            return;
        }

        let mut next_pawn = self.pawns[self.size - 1];
        let mut next_colored = self.colored[self.size - 1];

        let piece = board.piece_on(m.from).unwrap();
        let target = board.piece_on(m.to);

        if piece == Piece::Pawn {
            next_pawn ^= zobrist_pst(board.side_to_move(), Piece::Pawn, m.from);
            next_pawn ^= zobrist_pst(board.side_to_move(), Piece::Pawn, m.to);
        } else {
            next_colored[board.side_to_move() as usize] ^=
                zobrist_pst(board.side_to_move(), piece, m.from);
            next_colored[board.side_to_move() as usize] ^=
                zobrist_pst(board.side_to_move(), piece, m.to);
        }

        if let Some(target) = target {
            if target == Piece::Pawn {
                next_pawn ^= zobrist_pst(!board.side_to_move(), Piece::Pawn, m.to);
            } else {
                next_colored[!board.side_to_move() as usize] ^=
                    zobrist_pst(!board.side_to_move(), target, m.to);
            }
        } else if board.is_ep(m) {
            // enpassent
            next_pawn ^= zobrist_pst(
                !board.side_to_move(),
                Piece::Pawn,
                board.ep_square().unwrap(),
            );
        }

        self.pawns[self.size] = next_pawn;
        self.colored[self.size] = next_colored;
        self.size += 1;
    }

    pub fn pop(&mut self) {
        self.size -= 1;
    }

    pub fn get(&self) -> u64 {
        assert!(self.size > 0);
        self.pawns[self.size - 1]
    }

    pub fn get_colored(&self) -> [u64; 2] {
        assert!(self.size > 0);
        self.colored[self.size - 1]
    }
}
