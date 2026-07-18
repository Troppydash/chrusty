use std::sync::{Arc, RwLock};

use arrayvec::ArrayVec;
use cozy_chess::{Board, Move};

use crate::{
    ext::{ExtBoard, ExtMove, MoveList},
    helpers::avg,
    heuristic::Heuristic,
    movepick::Movepick,
    param::*,
    pesto,
    rep::RepTable,
    timer::Timer,
    tt::{FLAG_ALPHA, FLAG_BETA, FLAG_EXACT, FLAG_NONE, TablePtr, get_can_use},
};

#[derive(Clone, Debug)]
struct PvList {
    moves: ArrayVec<Move, MAX_DEPTH_USIZE>,
}

impl PvList {
    fn new() -> Self {
        Self {
            moves: ArrayVec::new(),
        }
    }

    fn clear(&mut self) {
        self.moves.clear();
    }

    fn set(&mut self, m: &Move, other: &PvList) {
        self.moves.clear();
        self.moves.push(*m);

        for m in other.moves.iter() {
            self.moves.push(*m);
        }
    }

    fn pv(&self) -> Move {
        assert!(self.moves.len() > 0);
        return self.moves[0];
    }

    fn get(&self, i: usize) -> Move {
        assert!(i < self.moves.len());
        return self.moves[i];
    }

    fn get_moves(&self) -> &ArrayVec<Move, MAX_DEPTH_USIZE> {
        &self.moves
    }

    fn len(&self) -> usize {
        self.moves.len()
    }
}

#[derive(Clone, Debug)]
struct SearchStack {
    ply: i8,
    m: Move,
    pv_list: PvList,
    adjusted_static: i16,
    tt_pv: bool,
}

impl SearchStack {
    pub fn new() -> Self {
        Self {
            ply: 0,
            m: Move::NULL_MOVE,
            pv_list: PvList::new(),
            adjusted_static: VALUE_NONE,
            tt_pv: false,
        }
    }

    pub fn new_ply(ply: i8) -> Self {
        Self {
            ply,
            m: Move::NULL_MOVE,
            pv_list: PvList::new(),
            adjusted_static: VALUE_NONE,
            tt_pv: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RootMove {
    pv_list: PvList,
    average_score: i16,
    score: i16,
}

impl RootMove {
    fn new(m: &Move) -> Self {
        let mut pv_list = PvList::new();
        pv_list.set(m, &PvList::new());
        Self {
            pv_list,
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
    stack: Box<[SearchStack]>,
    // need to Box for movepick ptr
    heuristic: Box<Heuristic>,
    nodes: i64,
    // only allocated once so Vec is ok
    root_moves: Box<[RootMove]>,
    // TODO: accesing the entire timer via RwLock is expensive
    timer: Arc<RwLock<Timer>>,
    rep: RepTable,
    table: TablePtr,
}

impl Engine {
    pub fn new(timer: Arc<RwLock<Timer>>, table: TablePtr) -> Self {
        Self {
            stack: vec![SearchStack::new(); SS_SIZE].into_boxed_slice(),
            heuristic: Box::new(Heuristic::new()),
            nodes: 0,
            root_moves: vec![].into_boxed_slice(),
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

    fn qsearch(
        &mut self,
        pos: &Board,
        mut alpha: i16,
        mut beta: i16,
        depth: i8,
        ss: usize,
        is_pv: bool,
    ) -> i16 {
        // note that we don't check timer in qsearch
        let ply = self.stack[ss].ply;

        assert!(alpha < beta, "alpha beta invariance {} {}", alpha, beta);

        //- prevent high depths
        if ply > MAX_DEPTH - 4 {
            if pos.in_check() {
                return VALUE_DRAW;
            }

            return self.evaluate(pos);
        }

        let key = pos.hash();
        //- simple draw checks
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

        //- tt
        // this clone only clones the tt ptr
        let table = self.table.clone();
        let table = table.get();
        let tt_age = table.get_age();
        let (reader, writer) = table.get(key);
        let mut tt_data = reader.get(key, ply, QDEPTH, alpha, beta);

        //- tt parsing
        tt_data.pv = if tt_data.hit && !tt_data.pv.is_null() && pos.is_legal(tt_data.pv) {
            tt_data.pv
        } else {
            Move::NULL_MOVE
        };

        //- tt cutoff
        if !is_pv && tt_data.can_use {
            return tt_data.score;
        }

        //- adjusted/unadjusted evals
        let mut unadjusted_static = VALUE_NONE;
        let mut best_score = -VALUE_INF;
        let in_check = pos.in_check();
        if in_check {
            best_score = -VALUE_INF;
        } else {
            if tt_data.hit {
                unadjusted_static = tt_data.static_score;
                if !is_valid(unadjusted_static) {
                    unadjusted_static = self.evaluate(pos);
                }
                best_score = unadjusted_static;

                //- use tt score to improve static score
                let can_improve_static =
                    get_can_use(tt_data.score, tt_data.flag, best_score, best_score);
                if is_valid(tt_data.score) && !is_decisive(tt_data.score) && can_improve_static {
                    best_score = tt_data.score;
                }
            } else {
                unadjusted_static = self.evaluate(pos);
                best_score = unadjusted_static;

                writer.set(
                    key,
                    &Move::NULL_MOVE,
                    ply,
                    UNSEARCH_DEPTH,
                    FLAG_NONE,
                    VALUE_NONE,
                    unadjusted_static,
                    false,
                    tt_age,
                );
            }

            //- standing pat
            if best_score >= beta {
                if !is_decisive(best_score) {
                    return avg(best_score, beta);
                }

                return best_score;
            }

            if best_score > alpha {
                alpha = best_score;
            }
        }

        //- negamax
        let mut move_count = 0;
        let mut best_move = Move::NULL_MOVE;
        let mut movepick =
            Movepick::new_qsearch(pos.clone(), tt_data.pv, ply, &self.heuristic, in_check);
        loop {
            let next_move = movepick.next_move();
            if next_move.is_null() {
                break;
            }

            move_count += 1;

            let new_pos = self.make_move(pos, &next_move.inner, ss);
            let score = -self.qsearch(&new_pos, -beta, -alpha, depth, ss + 1, is_pv);
            self.unmake_move(pos);

            if score > best_score {
                best_score = score;

                if score > alpha {
                    best_move = next_move.inner;

                    if score >= beta {
                        break;
                    }

                    alpha = score;
                }
            }

            //- late move prune
            if !is_loss(best_score) {
                if !in_check && move_count >= 3 {
                    break;
                }

                if in_check && pos.is_quiet(&next_move.inner) {
                    break;
                }
            }
        }

        //- mates
        if in_check && move_count == 0 {
            best_score = lose_in(ply);
        } else if !is_decisive(best_score) && best_score > beta {
            best_score = avg(best_score, beta);
        }

        let flag = if best_score >= beta {
            FLAG_BETA
        } else {
            FLAG_ALPHA
        };

        writer.set(
            key,
            &best_move,
            ply,
            QDEPTH,
            flag,
            best_score,
            unadjusted_static,
            is_pv || (tt_data.hit && tt_data.is_pv),
            tt_age,
        );

        best_score
    }

    fn negamax(
        &mut self,
        pos: &Board,
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

        self.stack[ss].pv_list.clear();

        if self.nodes % 8192 == 0 {
            self.timer.write().unwrap().check();
            if self.nodes >= self.timer.read().unwrap().max_nodes {
                self.timer.write().unwrap().force_stop();
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
            return self.qsearch(pos, alpha, beta, depth, ss, is_pv);
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
        self.stack[ss].tt_pv = is_pv || (tt_data.hit && tt_data.is_pv);

        // legality
        tt_data.pv = if tt_data.hit && !tt_data.pv.is_null() && pos.is_legal(tt_data.pv) {
            tt_data.pv
        } else {
            Move::NULL_MOVE
        };

        //- always use pv of root_moves
        if is_root {
            tt_data.pv = self.root_moves[0].pv_list.pv();
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
        if in_check {
            self.stack[ss].adjusted_static = VALUE_NONE;
        } else if tt_data.hit {
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

        let mut quiets = MoveList::new();
        let mut captures = MoveList::new();

        //- negamax alphabeta search
        let mut movepick = Movepick::new_negamax(pos.clone(), tt_data.pv, ply, &self.heuristic);
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
                    .find(|rm| rm.pv_list.pv() == next_move.inner)
                    .unwrap();
                root_move.average_score = if is_valid(root_move.average_score) {
                    avg(root_move.average_score, score)
                } else {
                    score
                };

                if move_count == 1 || score > alpha {
                    root_move.score = score;
                    root_move
                        .pv_list
                        .set(&root_move.pv_list.pv(), &self.stack[ss + 1].pv_list);
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
                        let (current, next) = self.stack.split_at_mut(ss + 1);
                        current[ss].pv_list.set(&best_move, &next[0].pv_list);
                    }

                    if score >= beta {
                        break;
                    }

                    alpha = score;
                }
            }

            if next_move.inner != best_move {
                if pos.is_quiet(&next_move.inner) {
                    quiets.push(next_move.inner);
                } else {
                    captures.push(next_move.inner);
                }
            }
        }

        if move_count == 0 {
            if in_check {
                best_score = lose_in(ply);
            } else {
                best_score = VALUE_DRAW;
            }
        } else if best_score >= beta {
            self.heuristic
                .update_history(pos, depth, ply, &best_move, &captures, &quiets);
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
        let mut root_moves = vec![];
        for m in pos.get_legal_moves().iter() {
            root_moves.push(RootMove::new(&m));
        }
        self.root_moves = root_moves.into_boxed_slice();
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
                    beta = avg(alpha, beta);
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
            for m in self.root_moves[0].pv_list.get_moves().iter() {
                print!(" {}", m.to_uci(&next_pos));
                next_pos.play_unchecked(*m);
            }
            println!("");
            depth += 1;
        }

        let result = SearchResult {
            root: self.root_moves[0].clone(),
            depth,
        };
        println!("info time {}", self.timer.read().unwrap().delta());

        let best_move = result.root.pv_list.pv();
        if result.root.pv_list.len() >= 2 {
            let ponder = result.root.pv_list.get(1);
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
