use shakmaty::{Chess, Role};

pub const MAX_DEPTH: i16 = 64;
pub const VALUE_INF: i16 = 32700;
pub const VALUE_NONE: i16 = 32701;
pub const VALUE_CHECKMATE: i16 = VALUE_INF - MAX_DEPTH - 1;
pub const VALUE_EVAL: i16 = VALUE_CHECKMATE - 1;

pub fn is_decisive(value: i16) -> bool {
    value.abs() > VALUE_CHECKMATE
}

pub fn is_win(value: i16) -> bool {
    value > VALUE_CHECKMATE
}

pub fn is_loss(value: i16) -> bool {
    value < -VALUE_CHECKMATE
}

pub fn win_in(ply: i16) -> i16 {
    VALUE_INF - ply
}

pub fn lose_in(ply: i16) -> i16 {
    -VALUE_INF + ply
}

pub const PIECE_VALUE: [i16; Role::ALL.len() + 1] = [100, 300, 300, 500, 900, 0, 0];

pub const NONE_PIECE_INDEX: usize = Role::ALL.len();
pub const MVV_MULTIPLIER: i16 = 2;
pub const BAD_QUIET_SCORE: i16 = -15000;

pub const LMR_MOVE_COUNT: usize = 96;
pub const LMR_DEPTH: usize = MAX_DEPTH as usize;

pub const SS_SIZE: usize = MAX_DEPTH as usize + 10;
