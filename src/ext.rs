use shakmaty::Move;

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
