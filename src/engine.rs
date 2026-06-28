use shakmaty::{Chess, Move, MoveList, Position};

use crate::{
    ext::NULL_MOVE,
    movepick::Movepick,
    param::{LMR_DEPTH, LMR_MOVE_COUNT, MAX_DEPTH, SS_SIZE, VALUE_INF},
};

#[derive(Clone, Copy)]
struct SearchStack {
    ply: i16,
    m: Move,
    pv_list: [Move; MAX_DEPTH as usize],
}

impl SearchStack {
    pub fn new() -> Self {
        Self {
            ply: 0,
            m: NULL_MOVE,
            pv_list: [NULL_MOVE; MAX_DEPTH as usize],
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

pub struct Engine {
    stack: [SearchStack; SS_SIZE],
    heuristic: Box<Heuristic>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            stack: [SearchStack::new(); SS_SIZE],
            heuristic: Box::new(Heuristic::new()),
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
        beta: i16,
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

            // TODO:

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
                // TODO
            }
        }

        // rest

        



        12
    }

    pub fn search(&mut self, // time
    ) {

        // asp window
    }
}
