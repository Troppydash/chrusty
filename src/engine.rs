use shakmaty::{Chess, Move};

use crate::movepick::Movepick;

struct SearchStack {
    ply: i8,
    m: Move,
    pv_list: Vec<Move>,
}

pub struct Engine {
    pos: Chess,
    stack: Vec<SearchStack>,
}

impl Engine {
    pub fn new() -> Self {}

    fn evaluate(&mut self) -> i16 {}

    fn qsearch(&mut self, alpha: i16, beta: i16, depth: i8) -> i16 {}

    fn negamax(
        &mut self,
        alpha: i16,
        beta: i16,
        depth: i8,
        ss: usize,
        is_pv: bool,
        cut_node: bool,
    ) -> i16 {
        if depth <= 0 {
            return self.evaluate();
        }

        let mut movepick = Movepick::new_negamax(&self.pos, None);
        loop {
            let m = movepick.next_move();
            if m.is_none() {
                break;
            }

            // alpha beta
        }

        // rest

        12
    }

    pub fn search(&mut self, // time
    ) {

        // asp window
    }
}
