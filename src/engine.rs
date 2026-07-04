use std::time::{SystemTime, UNIX_EPOCH};

use shakmaty::{Chess, Move, MoveList, Position};

use crate::{ext::NULL_MOVE, movepick::Movepick, param::*, pesto};

#[derive(Clone, Copy)]
struct SearchStack {
    ply: i8,
    m: Move,
    pv_list: [Move; MAX_DEPTH as usize],
    pv_size: usize,
    in_check: bool,
    adjusted_static: i16,
}

impl SearchStack {
    pub fn new() -> Self {
        Self {
            ply: 0,
            m: NULL_MOVE,
            pv_list: [NULL_MOVE; MAX_DEPTH as usize],
            pv_size: 0,
            in_check: false,
            adjusted_static: VALUE_NONE,
        }
    }

    pub fn new_ply(ply: i8) -> Self {
        Self {
            ply,
            m: NULL_MOVE,
            pv_list: [NULL_MOVE; MAX_DEPTH as usize],
            pv_size: 0,
            in_check: false,
            adjusted_static: VALUE_NONE,
        }
    }
}

struct Heuristic {
    // lmr[move_count][depth]
    lmr: [[i8; LMR_DEPTH]; LMR_MOVE_COUNT], // TODO: history
}

impl Heuristic {
    pub fn new() -> Self {
        let mut lmr = [[0; LMR_DEPTH]; LMR_MOVE_COUNT];
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

#[derive(Clone, Copy)]
pub struct RootMove {
    pv_list: [Move; MAX_DEPTH as usize],
    pv_size: usize,
    average_score: i16,
    score: i16,
}

impl RootMove {
    fn new(m: Move) -> Self {
        let mut pv_list = [NULL_MOVE; MAX_DEPTH as usize];
        pv_list[0] = m;
        Self {
            pv_list,
            pv_size: 1,
            average_score: VALUE_NONE,
            score: 0,
        }
    }
}

pub struct SearchResult {
    pub root: RootMove,
    pub depth: i8,
}

pub struct Timer {
    start: u128,
    duration: u128,
    stopped: bool,
}

impl Timer {
    fn new() -> Self {
        Self {
            start: 0,
            duration: 0,
            stopped: false,
        }
    }

    fn now() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }

    fn start(&mut self, duration: u128) {
        self.start = Self::now();
        self.duration = duration;
    }

    fn check(&mut self) {
        if self.stopped {
            return;
        }

        if Self::now() >= self.start + self.duration {
            self.stopped = true;
        }
    }

    fn stopped(&self) -> bool {
        self.stopped
    }

    fn test(&self, duration: u128) -> bool {
        Self::now() >= self.start + duration
    }

    fn delta(&self) -> u128 {
        Self::now() - self.start
    }
}

pub struct Engine {
    stack: [SearchStack; SS_SIZE],
    heuristic: Box<Heuristic>,
    nodes: i64,
    root_moves: Vec<RootMove>, // only allocated once so Vec is ok
    timer: Timer,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            stack: [SearchStack::new(); SS_SIZE],
            heuristic: Box::new(Heuristic::new()),
            nodes: 0,
            root_moves: vec![],
            timer: Timer::new(),
        }
    }

    fn sort_root_moves(&mut self) {
        let mut best = 0;
        for i in 1..self.root_moves.len() {
            if self.root_moves[i].score > self.root_moves[best].score {
                best = i;
            }
        }

        self.root_moves.swap(0, best);
    }

    fn evaluate(&mut self, pos: &mut Chess) -> i16 {
        return pesto::evaluate(pos) as i16 + 20;
    }

    fn qsearch(&mut self, pos: Chess, alpha: i16, beta: i16, depth: i8) -> i16 {
        todo!()
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
        self.stack[ss].pv_size = 0;

        let ply = self.stack[ss].ply;
        let is_root = ply == 0;

        if depth <= 0 {
            return self.evaluate(pos);
        }

        self.nodes += 1;

        // draw checks
        if !is_root {
            if pos.has_insufficient_material(pos.turn()) {
                return VALUE_DRAW;
            }

            if pos.halfmoves() >= 50 {
                if pos.is_check() && pos.legal_moves().is_empty() {
                    return lose_in(ply);
                }

                return VALUE_DRAW;
            }

            alpha = alpha.max(lose_in(ply));
            beta = beta.min(win_in(ply + 1));
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
            unadjusted_static = self.evaluate(pos);
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
                        !cut_node,
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
                    !cut_node,
                );
            }

            if is_pv && (move_count == 1 || score > alpha) {
                score = -self.negamax(&mut new_pos, -beta, -alpha, new_depth, ss + 1, true, false);
            }

            if is_root {
                let root_move = self
                    .root_moves
                    .iter_mut()
                    .find(|rm| rm.pv_list[0] == m.m)
                    .unwrap();
                root_move.average_score = if is_valid(root_move.average_score) {
                    (root_move.average_score + score) / 2
                } else {
                    score
                };

                if move_count == 1 || score > alpha {
                    root_move.score = score;

                    for i in 0..self.stack[ss + 1].pv_size {
                        assert!(self.stack[ss + 1].pv_list[i] != NULL_MOVE);
                        root_move.pv_list[1 + i] = self.stack[ss + 1].pv_list[i];
                    }

                    root_move.pv_size = self.stack[ss + 1].pv_size + 1;
                } else {
                    // fail-low cannot be ordered
                    root_move.score = -VALUE_INF;
                }
            }

            if score > best_score {
                best_score = score;

                if score > alpha {
                    best_move = m.m;

                    if is_pv && !is_root {
                        self.stack[ss].pv_list[0] = best_move;
                        for i in 0..self.stack[ss + 1].pv_size {
                            assert!(self.stack[ss + 1].pv_list[i] != NULL_MOVE);
                            self.stack[ss].pv_list[1 + i] = self.stack[ss + 1].pv_list[i];
                        }

                        self.stack[ss].pv_size = self.stack[ss + 1].pv_size + 1;
                    }

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

    pub fn search(&mut self, pos: &mut Chess, opt_time: u128, max_time: u128) -> SearchResult {
        assert!(
            opt_time > 0 && max_time >= opt_time,
            "time must be > 0 and max_time > opt_time"
        );

        // timer
        self.timer.start(max_time);

        // root moves
        self.root_moves.clear();
        for m in pos.legal_moves() {
            self.root_moves.push(RootMove::new(m));
        }
        assert!(!self.root_moves.is_empty(), "root moves is empty");

        // search stack
        for i in 0..SS_SIZE_PRE {
            self.stack[i] = SearchStack::new();
        }
        for i in 0..(SS_SIZE - SS_SIZE_PRE) {
            self.stack[SS_SIZE_PRE + i] = SearchStack::new_ply(i as i8);
        }

        // iterative deepening
        let mut depth = 1;
        while depth < MAX_DEPTH {
            let mut alpha = -VALUE_INF;
            let mut beta = VALUE_INF;

            let average_score = self.root_moves[0].average_score;
            let mut window = if is_valid(average_score) {
                ASP_WINDOW
                    + (average_score as i64 * average_score as i64 / ASP_WINDOW_SCORE_SCALE as i64)
                        as i16
            } else {
                ASP_WINDOW
            };

            if depth >= ASP_WINDOW_MIN_DEPTH {
                alpha = (-VALUE_INF).max(self.root_moves[0].score - window);
                beta = (VALUE_INF).min(self.root_moves[0].score + window);
            }

            // asp window
            loop {
                assert!(alpha < beta, "alpha beta invariance");
                let score = self.negamax(pos, alpha, beta, depth, SS_SIZE_PRE, true, false);
                self.sort_root_moves();

                if self.timer.stopped() {
                    break;
                }

                if score <= alpha {
                    beta = (alpha + beta) / 2;
                    alpha = (-VALUE_INF).max(score - window);
                } else if score >= beta {
                    beta = (VALUE_INF).min(score + window);
                } else {
                    break;
                }

                if window < ASP_WINDOW_MAX_SIZE {
                    window += window / ASP_WINDOW_SCALE;
                } else {
                    alpha = -VALUE_INF;
                    beta = VALUE_INF;
                }
            }

            // force exit
            if self.timer.stopped() {
                break;
            }

            // opt exit
            if self.timer.test(opt_time) {
                break;
            }

            let nps = self.nodes * 1000 / self.timer.delta().max(1) as i64;
            let score = self.root_moves[0].score;
            let score_str = if is_win(score) {
                let ply = VALUE_INF - score;
                format!("mate {}", ply / 2 + ply % 2)
            } else if is_loss(score) {
                let ply = -VALUE_INF - score;
                format!("mate {}", ply / 2 + ply % 2)
            } else {
                format!("cp {}", score)
            };
            print!(
                "info depth {} nodes {} time {} score {} nps {} pv",
                depth,
                self.nodes,
                self.timer.delta(),
                score_str,
                nps,
            );
            for i in 0..self.root_moves[0].pv_size {
                print!(" {}", self.root_moves[0].pv_list[i]);
            }
            println!("");

            depth += 1;
        }

        SearchResult {
            root: self.root_moves[0],
            depth,
        }
    }
}
