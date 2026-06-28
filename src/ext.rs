use shakmaty::{Move, Role::King, Square};

#[derive(Debug, Clone, Copy)]
pub struct ScoredMove {
    pub m: Move,
    pub score: i16,
}

impl ScoredMove {
    pub fn new(m: Move, score: i16) -> Self {
        Self { m, score }
    }

    pub fn from_move(m: Move) -> Self {
        Self::new(m, 0)
    }
}

pub const NULL_MOVE: Move = Move::Normal {
    role: King,
    from: Square::A1,
    capture: None,
    to: Square::A1,
    promotion: None,
};

pub fn is_null(m: &Move) -> bool {
    m == &NULL_MOVE
}
