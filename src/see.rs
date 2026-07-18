use cozy_chess::{BitBoard, Board, Color, Move, Piece, Square};

use crate::ext::{ExtBoard, MoveType::NORMAL};

pub fn see_ge(pos: &Board, m: &Move, value: i32) -> bool {
    if pos.move_type(m) != NORMAL {
        return true;
    }

    // copy stockfish
    todo!()
}
