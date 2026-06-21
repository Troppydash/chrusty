use shakmaty::{Chess, Move};

struct SearchStack {
    ply: i8,
    m: Move,
    pv_list: Vec<Move>
}

pub struct Engine {
    pos: Chess,
    stack: Vec<SearchStack>
}

impl Engine {
    pub fn new() -> Self {
        
    }

    fn evaluate(&mut self) -> i16 {

    }

    fn qsearch(&mut self, alpha: i16, beta: i16, depth: i8) -> i16 {}

    fn negamax(&mut self, alpha: i16, beta: i16, depth: i8, ss: usize, is_pv: bool, cut_node: bool) -> i16 {
        if (depth <= 0) {
            return self.evaluate();
        }

        let bestVal = -VALUE_INF;
        for () {
            
        }
        
    }

    pub fn search(
        &mut self
        // time
    ) {
        
        // asp window
    }

}