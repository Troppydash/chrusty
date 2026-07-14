use std::{
    cell::RefCell,
    ops::DerefMut,
    ptr::{null, null_mut},
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use cozy_chess::{Board, Move};

use crate::{
    ext::{ExtBoard, ExtMove},
    heuristic::Heuristic,
    movepick::Movepick,
    param::*,
    pesto,
    rep::RepTable,
    timer::Timer,
    tt::{FLAG_ALPHA, FLAG_BETA, FLAG_EXACT, FLAG_NONE, Table, TablePtr, get_can_use},
};

#[derive(Clone, Copy)]
struct SearchStack {
    ply: i8,
    m: Move,
    pv_list: [Move; MAX_DEPTH as usize],
    pv_size: usize,
    in_check: bool,
    adjusted_static: i16,
    tt_hit: bool,
    tt_pv: bool,
}

impl SearchStack {
    pub fn new() -> Self {
        Self {
            ply: 0,
            m: Move::NULL_MOVE,
            pv_list: [Move::NULL_MOVE; MAX_DEPTH as usize],
            pv_size: 0,
            in_check: false,
            adjusted_static: VALUE_NONE,
            tt_hit: false,
            tt_pv: false,
        }
    }

    pub fn new_ply(ply: i8) -> Self {
        Self {
            ply,
            m: Move::NULL_MOVE,
            pv_list: [Move::NULL_MOVE; MAX_DEPTH as usize],
            pv_size: 0,
            in_check: false,
            adjusted_static: VALUE_NONE,
            tt_hit: false,
            tt_pv: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RootMove {
    pv_list: [Move; MAX_DEPTH as usize],
    pv_size: usize,
    average_score: i16,
    score: i16,
}

impl RootMove {
    fn new(m: Move) -> Self {
        let mut pv_list = [Move::NULL_MOVE; MAX_DEPTH as usize];
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

pub struct Engine {
    stack: [SearchStack; SS_SIZE],
    heuristic: Box<Heuristic>,
    nodes: i64,
    root_moves: Vec<RootMove>, // only allocated once so Vec is ok
    timer: Arc<RwLock<Timer>>,
    rep: RepTable,
    table: TablePtr,
}

impl Engine {
    pub fn new(timer: Arc<RwLock<Timer>>, table: TablePtr) -> Self {
        Self {
            stack: [SearchStack::new(); SS_SIZE],
            heuristic: Box::new(Heuristic::new()),
            nodes: 0,
            root_moves: vec![],
            timer,
            rep: RepTable::new(),
            table,
        }
    }

    pub fn newgame(&mut self) {
        self.heuristic.clear();
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

    fn make_move(&mut self, pos: &Board, m: &Move, ss: usize) -> Board {
        self.rep.add(pos.hash());
        self.stack[ss].m = m.clone();

        let mut new_pos = pos.clone();
        if m.is_null() {
            new_pos.null_move().unwrap()
        } else {
            new_pos.play_unchecked(*m);
            new_pos
        }
    }

    fn unmake_move(&mut self, pos: &Board) {
        self.rep.remove(pos.hash());
    }

    fn evaluate(&mut self, pos: &Board) -> i16 {
        let tempo = 20;
        let eval = pesto::evaluate(pos);
        return (eval + tempo).clamp(-VALUE_EVAL as i32, VALUE_EVAL as i32) as i16;
    }

    fn qsearch(&mut self, pos: &mut Board, alpha: i16, beta: i16, depth: i8) -> i16 {
        todo!()
    }

    fn negamax(
        &mut self,
        pos: &mut Board,
        mut alpha: i16,
        mut beta: i16,
        depth: i8,
        ss: usize,
        is_pv: bool,
        cut_node: bool,
    ) -> i16 {
        let ply = self.stack[ss].ply;
        let is_root = ply == 0;

        assert!(alpha < beta, "alpha beta invariance {} {}", alpha, beta);
        assert!(!(is_root && cut_node));
        assert!(!(is_pv && cut_node));

        self.stack[ss].pv_size = 0;

        if self.nodes % 8192 == 0 {
            self.timer.write().unwrap().check();
            if !self.timer.read().unwrap().stopped() {
                if self.nodes >= self.timer.read().unwrap().max_nodes {
                    self.timer.write().unwrap().force_stop();
                }
            }
        }

        if self.timer.read().unwrap().stopped() {
            return 0;
        }

        //- prevent high depths
        if ply > MAX_DEPTH - 4 {
            if pos.in_check() {
                return VALUE_DRAW;
            }

            return self.evaluate(pos);
        }

        self.nodes += 1;

        //- qsearch drop
        if depth <= 0 {
            return self.evaluate(pos);
        }

        let key = pos.hash();

        //- simple draw checks
        if !is_root {
            // if pos.has_insufficient_material(pos.turn()) {
            //     return VALUE_DRAW;
            // }

            if pos.halfmove_clock() >= 100 {
                if pos.in_check() && !pos.any_moves() {
                    return lose_in(ply);
                }

                return VALUE_DRAW;
            }

            if self.rep.check(key) {
                return VALUE_DRAW;
            }

            // mate score pruning
            alpha = alpha.max(lose_in(ply));
            beta = beta.min(win_in(ply + 1));
            if alpha >= beta {
                return alpha;
            }

            // TODO: cuckoo table
        }

        //- tt
        // this clone only clones the tt ptr
        let table = self.table.clone();
        let table = table.get();
        let tt_age = table.get_age();
        let (reader, writer) = table.get(key);
        let mut tt_data = reader.get(key, ply, depth, alpha, beta);

        //- tt parsing
        self.stack[ss].tt_hit = tt_data.hit;
        self.stack[ss].tt_pv = is_pv || (tt_data.hit && tt_data.is_pv);

        // legality
        tt_data.pv = if tt_data.hit && !tt_data.pv.is_null() && pos.is_legal(tt_data.pv) {
            tt_data.pv
        } else {
            Move::NULL_MOVE
        };

        //- always use pv of root_moves
        if is_root {
            tt_data.pv = self.root_moves[0].pv_list[0];
        }

        //- tt early return
        if !is_pv
            && tt_data.can_use
            && (cut_node == (tt_data.score >= beta))
            && tt_data.depth >= depth + (tt_data.score >= beta) as i8
        {
            if pos.halfmove_clock() < 80 {
                return tt_data.score;
            }
        }

        //- adjusted/unadjusted evals
        let mut unadjusted_static = VALUE_NONE;
        let in_check = pos.in_check();
        self.stack[ss].in_check = in_check;
        if in_check {
            self.stack[ss].adjusted_static = VALUE_NONE;
        } else if self.stack[ss].tt_hit {
            unadjusted_static = tt_data.static_score;
            if !is_valid(unadjusted_static) {
                unadjusted_static = self.evaluate(pos);
            }
            self.stack[ss].adjusted_static = unadjusted_static;

            //- use tt score to improve static score
            let can_improve_static = get_can_use(
                tt_data.score,
                tt_data.flag,
                self.stack[ss].adjusted_static,
                self.stack[ss].adjusted_static,
            );
            if is_valid(tt_data.score) && !is_decisive(tt_data.score) && can_improve_static {
                self.stack[ss].adjusted_static = tt_data.score;
            }
        } else {
            unadjusted_static = self.evaluate(pos);
            self.stack[ss].adjusted_static = unadjusted_static;

            writer.set(
                key,
                &Move::NULL_MOVE,
                ply,
                UNSEARCH_DEPTH,
                FLAG_NONE,
                VALUE_NONE,
                unadjusted_static,
                self.stack[ss].tt_pv,
                tt_age,
            );
        }

        if !in_check {
            // TODO: pruning
        }

        let mut move_count = 0;
        let mut best_score = -VALUE_INF;
        let mut best_move = Move::NULL_MOVE;

        //- negamax alphabeta search
        let mut movepick = Movepick::new_negamax(&pos, tt_data.pv);
        loop {
            let next_move = movepick.next_move();
            if next_move.is_null() {
                break;
            }

            move_count += 1;

            let mut new_pos = self.make_move(pos, &next_move.inner, ss);

            let new_depth = depth - 1;
            let mut score = 0;

            //- late move reduction
            if depth >= 2 && move_count > 1 + 2 * is_root as usize {
                let reduction = self.heuristic.get_lmr(move_count, depth);
                let reduced_depth = (new_depth - reduction).clamp(1, new_depth + 1);

                //- pv search
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

            self.unmake_move(pos);

            if self.timer.read().unwrap().stopped() {
                return 0;
            }

            //- root moves update
            if is_root {
                let root_move = self
                    .root_moves
                    .iter_mut()
                    .find(|rm| rm.pv_list[0] == next_move.inner)
                    .unwrap();
                root_move.average_score = if is_valid(root_move.average_score) {
                    ((root_move.average_score as i32 + score as i32) / 2) as i16
                } else {
                    score
                };

                if move_count == 1 || score > alpha {
                    root_move.score = score;

                    for i in 0..self.stack[ss + 1].pv_size {
                        assert!(self.stack[ss + 1].pv_list[i] != Move::NULL_MOVE);
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
                    best_move = next_move.inner;

                    if is_pv && !is_root {
                        self.stack[ss].pv_list[0] = best_move;
                        for i in 0..self.stack[ss + 1].pv_size {
                            assert!(self.stack[ss + 1].pv_list[i] != Move::NULL_MOVE);
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

            if next_move.inner != best_move {
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

        //- tt_pv propagation
        if best_score <= alpha {
            self.stack[ss].tt_pv = self.stack[ss].tt_pv || self.stack[ss - 1].tt_pv;
        }

        let flag = if best_score >= beta {
            FLAG_BETA
        } else if is_pv && !best_move.is_null() {
            FLAG_EXACT
        } else {
            FLAG_ALPHA
        };

        //- tt update
        writer.set(
            key,
            &best_move,
            ply,
            depth,
            flag,
            best_score,
            unadjusted_static,
            self.stack[ss].tt_pv,
            tt_age,
        );

        best_score
    }

    pub fn search(&mut self, startpos: Board, moves: Vec<Move>) -> SearchResult {
        self.nodes = 0;
        self.rep.clear();
        self.table.get().next_search();

        // history tracking
        let mut pos = startpos;
        for m in moves.iter() {
            let key = pos.hash();
            self.rep.add_history(key);
            pos.play_unchecked(*m);
        }

        // root moves
        self.root_moves.clear();
        for m in pos.get_legal_moves() {
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
        while depth < self.timer.read().unwrap().max_depth {
            let mut alpha = -VALUE_INF;
            let mut beta = VALUE_INF;

            let average_score = self.root_moves[0].average_score;
            let mut window = if is_valid(average_score) {
                // need to wrap to prevent overflow
                let base = ASP_WINDOW as i32
                    + (average_score as i32 * average_score as i32 / ASP_WINDOW_SCORE_SCALE as i32);
                base.min(ASP_WINDOW_MAX_SIZE as i32) as i16
            } else {
                ASP_WINDOW
            };

            assert!(window >= 0, "window must be >= 0");

            if depth >= ASP_WINDOW_MIN_DEPTH {
                alpha =
                    (-VALUE_INF as i32).max(self.root_moves[0].score as i32 - window as i32) as i16;
                beta =
                    (VALUE_INF as i32).min(self.root_moves[0].score as i32 + window as i32) as i16;
            }

            // asp window
            loop {
                assert!(alpha < beta, "alpha beta invariance {} {}", alpha, beta);
                let score = self.negamax(&mut pos, alpha, beta, depth, SS_SIZE_PRE, true, false);
                self.sort_root_moves();

                if self.timer.read().unwrap().stopped() {
                    break;
                }

                if score <= alpha {
                    beta = ((alpha as i32 + beta as i32) / 2) as i16;
                    alpha = (-VALUE_INF as i32).max(score as i32 - window as i32) as i16;
                } else if score >= beta {
                    beta = (VALUE_INF as i32).min(score as i32 + window as i32) as i16;
                } else {
                    break;
                }

                // need [ASP_WINDOW_MAX_SIZE] to be small enough to prevent overflow
                window += window / ASP_WINDOW_SCALE;
                if window > ASP_WINDOW_MAX_SIZE {
                    window = ASP_WINDOW_MAX_SIZE;
                    alpha = -VALUE_INF;
                    beta = VALUE_INF;
                }
            }

            // force exit
            if self.timer.read().unwrap().stopped() {
                break;
            }

            // opt exit
            if self
                .timer
                .read()
                .unwrap()
                .test(self.timer.read().unwrap().opt_time)
            {
                break;
            }

            let delta = self.timer.read().unwrap().delta();
            let nps = self.nodes * 1000 / delta.max(1) as i64;
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
                depth, self.nodes, delta, score_str, nps,
            );

            let mut next_pos = pos.clone();
            for i in 0..self.root_moves[0].pv_size {
                print!(" {}", self.root_moves[0].pv_list[i].to_uci(&next_pos));
                next_pos.play_unchecked(self.root_moves[0].pv_list[i]);
            }
            println!("");
            depth += 1;
        }

        let result = SearchResult {
            root: self.root_moves[0],
            depth,
        };
        println!("info time {}", self.timer.read().unwrap().delta());

        assert!(result.root.pv_size != 0);
        let best_move = result.root.pv_list[0];
        if result.root.pv_size >= 2 {
            let ponder = result.root.pv_list[1];
            let mut next_pos = pos.clone();
            next_pos.play_unchecked(best_move.clone());
            println!(
                "bestmove {} ponder {}",
                best_move.to_uci(&pos),
                ponder.to_uci(&next_pos)
            );
        } else {
            println!("bestmove {} ", best_move.to_uci(&pos));
        }

        result
    }
}
