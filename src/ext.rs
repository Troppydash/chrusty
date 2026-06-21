use shakmaty::Move;

#[derive(Debug, Clone, Copy)]
pub struct ScoredMove {
    m: Move,
    score: i16
}

impl ScoredMove {
    pub fn new(m: Move, score: i16) -> Self {
        Self {
            m,
            score 
        }
    }
}