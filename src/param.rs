const MAX_DEPTH: i16 = 64;
const VALUE_INF: i16 = 32700;
const VALUE_NONE: i16 = 32701;
const VALUE_CHECKMATE: i16 = VALUE_INF - MAX_DEPTH - 1;
const VALUE_EVAL: i16 = VALUE_CHECKMATE - 1;

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
