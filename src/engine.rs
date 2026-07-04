use shakmaty::{Chess, Move, MoveList, Position};

use crate::{
    ext::NULL_MOVE,
    movepick::Movepick,
    param::{
        ASP_WINDOW, LMR_DEPTH, LMR_MOVE_COUNT, MAX_DEPTH, SS_SIZE, VALUE_DRAW, VALUE_INF,
        VALUE_NONE, lose_in, win_in,
    },
};

#[derive(Clone, Copy)]
struct SearchStack {
    ply: i16,
    m: Move,
    pv_list: [Move; MAX_DEPTH as usize],
    in_check: bool,
    adjusted_static: i16,
}

impl SearchStack {
    pub fn new() -> Self {
        Self {
            ply: 0,
            m: NULL_MOVE,
            pv_list: [NULL_MOVE; MAX_DEPTH as usize],
            in_check: false,
            adjusted_static: VALUE_NONE,
        }
    }
}

struct Heuristic {
    // lmr[move_count][depth]
    lmr: [[i8; LMR_MOVE_COUNT]; LMR_DEPTH], // TODO: history
}

impl Heuristic {
    pub fn new() -> Self {
        let mut lmr = [[0; LMR_MOVE_COUNT]; LMR_DEPTH];
        for move_count in 0..LMR_MOVE_COUNT {
            for depth in 0..LMR_DEPTH {
                lmr[move_count][depth] =
                    (0.99 + f32::ln(move_count as f32) * f32::ln(depth as f32) / 3.14) as i8;
            }
        }

        Self { lmr }
    }

    pub fn get_lmr(&self, move_count: usize, depth: i8) -> i8 {
        assert!(depth >= 0);
        self.lmr[move_count.min(LMR_MOVE_COUNT - 1)][(depth as usize).min(LMR_DEPTH - 1)]
    }
}

pub struct RootMove {
    pub pv: [Move; MAX_DEPTH as usize],
    average_score: i16,
    score: i16,
}

pub struct SearchResult {
    pub root: RootMove,
    pub depth: i8,
}

impl SearchResult {
    pub fn new(root: RootMove) -> Self {
        Self { root, depth: 0 }
    }
}

pub struct Timer {
    start: u64,
}

impl Timer {}

pub struct Engine {
    stack: [SearchStack; SS_SIZE],
    heuristic: Box<Heuristic>,
    nodes: u64,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            stack: [SearchStack::new(); SS_SIZE],
            heuristic: Box::new(Heuristic::new()),
            nodes: 0,
        }
    }

    fn evaluate(&mut self) -> i16 {
        0
    }

    fn qsearch(&mut self, pos: Chess, alpha: i16, beta: i16, depth: i8) -> i16 {
        0
    }

    fn negamax(
        &mut self,
        pos: &mut Chess,
        mut alpha: i16,
        mut beta: i16,
        depth: i8,
        ss: usize,
        is_pv: bool,
        cut_node: bool,
    ) -> i16 {
        let ply = self.stack[ss].ply;
        let is_root = ply == 0;

        if depth <= 0 {
            return self.evaluate();
        }

        self.nodes += 1;

        // draw checks
        if !is_root {
            if pos.has_insufficient_material(pos.turn()) {
                return VALUE_DRAW;
            }

            if pos.halfmoves() >= 50 {
                // TODO: fix
                return VALUE_DRAW;
            }

            alpha = alpha.max(win_in(ply));
            beta = beta.min(lose_in(ply + 1));
            if alpha >= beta {
                return alpha;
            }

            // TODO: cuckoo table
        }

        // TODO: tt

        let mut unadjusted_static = VALUE_NONE;
        self.stack[ss].in_check = pos.is_check();
        let in_check = self.stack[ss].in_check;
        if in_check {
            self.stack[ss].adjusted_static = VALUE_NONE;
        } else {
            unadjusted_static = self.evaluate();
            self.stack[ss].adjusted_static = unadjusted_static;
        }

        if !in_check {
            // TODO: pruning
        }

        let mut move_count = 0;
        let mut best_score = -VALUE_INF;
        let mut best_move = NULL_MOVE;

        let mut movepick = Movepick::new_negamax(&pos, None);
        loop {
            let m = movepick.next_move();
            if m.is_none() {
                break;
            }
            let m = m.unwrap();

            move_count += 1;

            let mut new_pos = pos.clone();
            new_pos.play_unchecked(m.m);

            let new_depth = depth - 1;
            let mut score = 0;

            if depth >= 2 && move_count > 1 + 2 * is_root as usize {
                let reduction = self.heuristic.get_lmr(move_count, depth);
                let reduced_depth = (new_depth - reduction).clamp(1, new_depth + 1);

                score = -self.negamax(
                    &mut new_pos,
                    -(alpha + 1),
                    -alpha,
                    reduced_depth,
                    ss + 1,
                    false,
                    true,
                );

                if score > alpha && reduced_depth < new_depth {
                    score = -self.negamax(
                        &mut new_pos,
                        -(alpha + 1),
                        -alpha,
                        new_depth,
                        ss + 1,
                        false,
                        true,
                    );
                }
            } else if !is_pv || move_count > 1 {
                score = -self.negamax(
                    &mut new_pos,
                    -(alpha + 1),
                    -alpha,
                    new_depth,
                    ss + 1,
                    false,
                    true,
                );
            }

            if is_pv && (move_count == 1 || score > alpha) {
                score = -self.negamax(&mut new_pos, -beta, -alpha, new_depth, ss + 1, false, true);
            }

            if score > best_score {
                best_score = score;

                if score > alpha {
                    best_move = m.m;

                    if score >= beta {
                        break;
                    }

                    alpha = score;
                }
            }

            if m.m != best_move {
                // TODO history
            }
        }

        if move_count == 0 {
            if self.stack[ss].in_check {
                best_score = lose_in(ply);
            } else {
                best_score = VALUE_DRAW;
            }
        } else if best_score >= beta {
            // TODO: history update
        }

        // TODO: tt

        best_score
    }

    pub fn search(&mut self, pos: &mut Chess, opt_time: u64, max_time: u64) -> SearchResult {
        // setup a bunch of things

        // TODO: root move list
        let moves = pos.legal_moves();
        let mut result = SearchResult::new(
            
        );

        // iterative deepening
        let mut depth = 1;
        while depth < MAX_DEPTH {
            let mut alpha = -VALUE_INF;
            let mut beta = VALUE_INF;

            let window = ASP_WINDOW;

            if depth >= 3 {}

            // asp window
            loop {
                break;
            }

            depth += 1;
        }

        todo!()
    }
}
