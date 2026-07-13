pub const MAX_DEPTH: i8 = 64;
pub const VALUE_NONE: i16 = 32701;
pub const VALUE_INF: i16 = 32700;
pub const VALUE_CHECKMATE: i16 = VALUE_INF - (MAX_DEPTH as i16) - 1;
pub const VALUE_EVAL: i16 = VALUE_CHECKMATE - 1;
pub const VALUE_DRAW: i16 = 0;
pub const MAX_TIME: u128 = 1_000_000_000_000_000;
pub const MAX_NODES: i64 = 1_000_000_000_000_000;

pub fn is_decisive(value: i16) -> bool {
    value.abs() > VALUE_CHECKMATE
}

pub fn is_win(value: i16) -> bool {
    value > VALUE_CHECKMATE
}

pub fn is_loss(value: i16) -> bool {
    value < -VALUE_CHECKMATE
}

pub fn win_in(ply: i8) -> i16 {
    VALUE_INF - ply as i16
}

pub fn lose_in(ply: i8) -> i16 {
    -VALUE_INF + ply as i16
}

pub fn is_valid(value: i16) -> bool {
    value != VALUE_NONE
}

pub const QDEPTH: i8 = 0;
pub const UNSEARCH_DEPTH: i8 = -10;
pub const UNINIT_DEPTH: i8 = -20;

pub const ALL_PIECES: usize = 6;
pub const PIECE_VALUE: [i16; ALL_PIECES + 1] = [100, 300, 300, 500, 900, 0, 0];

pub const NONE_PIECE_INDEX: usize = ALL_PIECES;
pub const MVV_MULTIPLIER: i16 = 2;
pub const BAD_QUIET_SCORE: i16 = -15000;

pub const LMR_MOVE_COUNT: usize = 96;
pub const LMR_DEPTH: usize = MAX_DEPTH as usize;

pub const SS_SIZE_PRE: usize = 10 as usize;
pub const SS_SIZE: usize = MAX_DEPTH as usize + SS_SIZE_PRE;

pub const ASP_WINDOW: i16 = 15;
pub const ASP_WINDOW_SCORE_SCALE: i16 = 13000;
pub const ASP_WINDOW_MIN_DEPTH: i8 = 3;
pub const ASP_WINDOW_MAX_SIZE: i16 = 4000;
pub const ASP_WINDOW_SCALE: i16 = 2;
