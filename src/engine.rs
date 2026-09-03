use std::{
    os::linux::raw::stat,
    sync::{Arc, RwLock},
};

use cozy_chess::{Board, Move, Piece};

use crate::{
    cuckoo,
    ext::{ColoredPiece, ExtBoard, ExtMove, MoveList},
    helpers::avg,
    heuristic::{CORR_LIMIT, Heuristic},
    movepick::Movepick,
    nnue::{NNUE, network::Permute},
    param::*,
    rep::{RepTable, is_rep},
    see::{self, see_ge},
    sort,
    spsa::Parameters,
    stack::{KeyStack, PawnKey, PvList, SearchStack},
    tb::TableBase,
    timer::Timer,
    tt::{FLAG_ALPHA, FLAG_BETA, FLAG_EXACT, FLAG_NONE, TablePtr, get_50mr_key, get_can_use},
};

#[derive(Clone, Debug)]
pub struct RootMove {
    pv_list: PvList,
    average_score: i16,
    score: i16,
    nodes: i64,
}

impl RootMove {
    fn new(m: &Move) -> Self {
        let mut pv_list = PvList::new();
        pv_list.set(m, &PvList::new());
        Self {
            pv_list,
            average_score: VALUE_NONE,
            score: 0,
            nodes: 0,
        }
    }
}

pub struct SearchResult {
    pub root: RootMove,
    pub depth: i8,
}

pub struct Engine {
    stack: Box<[SearchStack]>,
    key_stack: KeyStack,
    pawn_key: PawnKey,
    // need to Box for movepick ptr
    heuristic: Box<Heuristic>,
    nodes: i64,
    prev_score: Option<i16>,
    tb_hits: i64,
    // only allocated once so Vec is ok
    root_moves: Box<[RootMove]>,
    // TODO: accesing the entire timer via RwLock is expensive
    timer: Arc<RwLock<Timer>>,
    rep: RepTable,
    table: TablePtr,
    nnue: NNUE,
    settings: Parameters,
    tb: Option<TableBase>,
}

impl Engine {
    pub fn new(timer: Arc<RwLock<Timer>>, table: TablePtr) -> Self {
        Self {
            stack: vec![SearchStack::new(); SS_SIZE].into_boxed_slice(),
            key_stack: KeyStack::new(),
            pawn_key: PawnKey::new(),
            heuristic: Box::new(Heuristic::new()),
            nodes: 0,
            prev_score: None,
            tb_hits: 0,
            root_moves: vec![].into_boxed_slice(),
            timer,
            rep: RepTable::new(),
            table,
            nnue: NNUE::build(&Permute::load()),
            settings: Parameters::default(),
            tb: None,
        }
    }

    pub fn set_settings(&mut self, settings: &Parameters) {
        self.settings = settings.clone();
    }

    pub fn set_tb(&mut self, tb: Option<TableBase>) {
        self.tb = tb;
    }

    pub fn newgame(&mut self) {
        self.heuristic.clear();
        self.nnue.clear();
        self.prev_score = None;
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

    fn make_move(&mut self, pos: &Board, m: Move, key: u64, ss: usize) -> Board {
        self.stack[ss].m = m.clone();
        self.key_stack.push(key);
        self.pawn_key.push(pos, m);
        self.stack[ss].cont_hist = self.heuristic.get_cont_hist_index(pos, m);

        let mut new_pos = pos.clone();
        if m.is_null() {
            self.stack[ss].piece = None;
            self.stack[ss].cont_corrhist = (12, 0);
            new_pos.null_move().unwrap()
        } else {
            self.stack[ss].piece = Some(pos.color_piece_on(m.from).unwrap());
            self.stack[ss].cont_corrhist = self.heuristic.get_cont_corrhist_index(pos, m);

            new_pos.play_unchecked(m);
            self.nnue.make_move(pos, &new_pos, m);

            new_pos
        }
    }

    fn unmake_move(&mut self, pos: &Board, key: u64, ss: usize) {
        if !self.stack[ss].m.is_null() {
            self.nnue.unmake_move();
        }
        self.key_stack.pop();
        self.pawn_key.pop();
    }

    fn evaluate(&mut self, pos: &Board) -> i16 {
        let mut score = self
            .nnue
            .evaluate(pos)
            .clamp(-VALUE_EVAL as i32, VALUE_EVAL as i32);
        // score = score * (200 - pos.halfmove_clock() as i32) / 200;
        return score as i16;
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
        self.nodes += 1;

        // note that we don't check timer in qsearch
        let ply = self.stack[ss].ply;
        self.stack[ss].pv_list.clear();

        assert!(alpha < beta, "alpha beta invariance {} {}", alpha, beta);

        //- prevent high depths
        if ply > MAX_DEPTH - 4 || depth < -20 {
            if pos.in_check() {
                return VALUE_DRAW;
            }

            let score = self.evaluate(pos);
            return self.static_correction(pos, score, ss);
        }

        let key = pos.correct_hash();
        let tt_key = key ^ get_50mr_key(pos.halfmove_clock() as usize);
        if pos.has_insufficient_material() {
            return VALUE_DRAW;
        }

        if pos.halfmove_clock() >= 100 {
            if pos.in_check() && !pos.any_moves() {
                return lose_in(ply);
            }

            return VALUE_DRAW;
        }

        if is_rep(pos, ply as usize, &self.key_stack) {
            return VALUE_DRAW;
        }

        // mate score pruning
        alpha = alpha.max(lose_in(ply));
        beta = beta.min(win_in(ply + 1));
        if alpha >= beta {
            return alpha;
        }

        if alpha < 0 && cuckoo::is_upcoming_rep(pos, &self.key_stack, ply) {
            alpha = 0;
            if alpha >= beta {
                return alpha;
            }
        }

        //- tt
        // this clone only clones the tt ptr
        let table = self.table.clone();
        let table = table.get();
        let tt_age = table.get_age();
        let (reader, writer) = table.get(tt_key);
        let mut tt_data = reader.get(tt_key, ply, QDEPTH, alpha, beta);

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
        let mut futility_base = -VALUE_INF;
        let in_check = pos.in_check();
        self.stack[ss].conseq_checks = 0;
        if in_check {
            self.stack[ss].conseq_checks = self.stack[ss - 2].conseq_checks + 1;
        } else {
            if tt_data.hit {
                unadjusted_static = tt_data.static_score;
                if !is_valid(unadjusted_static) {
                    unadjusted_static = self.evaluate(pos);
                }
                best_score = self.static_correction(pos, unadjusted_static, ss);

                //- use tt score to improve static score
                let can_improve_static =
                    get_can_use(tt_data.score, tt_data.flag, best_score, best_score);
                if is_valid(tt_data.score) && !is_decisive(tt_data.score) && can_improve_static {
                    best_score = tt_data.score;
                }
            } else {
                unadjusted_static = self.evaluate(pos);
                best_score = self.static_correction(pos, unadjusted_static, ss);

                writer.set(
                    tt_key,
                    Move::NULL_MOVE,
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

            futility_base = (best_score as i32 + 300).min(VALUE_EVAL as i32) as i16;
        }

        //- negamax
        let mut move_count = 0;
        let mut best_move = Move::NULL_MOVE;
        let mut movepick = Movepick::new_qsearch(
            pos.clone(),
            tt_data.pv,
            ply,
            depth,
            &self.stack,
            ss,
            self.pawn_key.get(),
            &self.heuristic,
            in_check,
        );
        loop {
            let next_move = movepick.next_move();
            if next_move.is_null() {
                break;
            }

            move_count += 1;
            self.table.get().prefetch(pos.new_hash(next_move.inner));

            if !is_loss(best_score) && !in_check {
                //- delta pruning
                // TODO: optimize
                if !pos.is_quiet(next_move.inner)
                    && futility_base as i32
                        + pesto_value(
                            pos,
                            ColoredPiece::new(
                                !pos.side_to_move(),
                                pos.get_captured(next_move.inner),
                            ),
                            next_move.inner.to,
                        )
                        <= alpha as i32
                    && !see::see_ge(pos, next_move.inner, 0)
                {
                    let futility_best_score = (futility_base as i32
                        + pesto_value(
                            pos,
                            ColoredPiece::new(
                                !pos.side_to_move(),
                                pos.get_captured(next_move.inner),
                            ),
                            next_move.inner.to,
                        ))
                    .min(VALUE_EVAL as i32) as i16;
                    best_score = best_score.max(futility_best_score);
                    continue;
                }

                //- see pruning
                if !see::see_ge(pos, next_move.inner, -50) {
                    continue;
                }
            }

            let new_pos = self.make_move(pos, next_move.inner, key, ss);
            let score = -self.qsearch(&new_pos, -beta, -alpha, depth - 1, ss + 1, is_pv);
            self.unmake_move(pos, key, ss);

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
                if !in_check && move_count >= 4 {
                    break;
                }

                if in_check && pos.is_quiet(next_move.inner) && move_count >= 2 {
                    break;
                }
            }
        }

        //- mates
        if in_check && move_count == 0 {
            best_score = lose_in(ply);
        } else if !in_check && move_count == 0 && !pos.any_moves() {
            best_score = VALUE_DRAW;
        } else if !is_decisive(best_score) && best_score > beta {
            best_score = avg(best_score, beta);
        }

        let flag = if best_score >= beta {
            FLAG_BETA
        } else {
            FLAG_ALPHA
        };

        writer.set(
            tt_key,
            best_move,
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
        mut depth: i8,
        ss: usize,
        is_pv: bool,
        cut_node: bool,
    ) -> i16 {
        self.nodes += 1;

        let ply = self.stack[ss].ply;
        let is_root = ply == 0;

        assert!(alpha < beta, "alpha beta invariance {} {}", alpha, beta);
        assert!(!(is_root && cut_node));
        assert!(!(is_pv && cut_node));

        self.stack[ss].pv_list.clear();

        if self.nodes % 4096 == 0 {
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

            let score = self.evaluate(pos);
            return self.static_correction(pos, score, ss);
        }

        //- qsearch drop
        if depth <= 0 {
            self.nodes -= 1;
            return self.qsearch(pos, alpha, beta, 0, ss, is_pv);
        }

        let key = pos.correct_hash();
        let tt_key = key ^ get_50mr_key(pos.halfmove_clock() as usize);

        //- simple draw checks
        if !is_root {
            if pos.has_insufficient_material() {
                return VALUE_DRAW;
            }

            if pos.halfmove_clock() >= 100 {
                if pos.in_check() && !pos.any_moves() {
                    return lose_in(ply);
                }

                return VALUE_DRAW;
            }

            if is_rep(pos, ply as usize, &self.key_stack) {
                return VALUE_DRAW;
            }

            //- mate score pruning
            alpha = alpha.max(lose_in(ply));
            beta = beta.min(win_in(ply + 1));
            if alpha >= beta {
                return alpha;
            }

            //- cuckoo
            if alpha < 0 && cuckoo::is_upcoming_rep(pos, &self.key_stack, ply) {
                alpha = 0;
                if alpha >= beta {
                    return alpha;
                }
            }
        }

        //- tt
        // this clone only clones the tt ptr
        let table = self.table.clone();
        let table = table.get();
        let tt_age = table.get_age();
        let (reader, writer) = table.get(tt_key);
        let mut tt_data = reader.get(tt_key, ply, depth, alpha, beta);

        //- tt parsing
        let excluded = self.stack[ss].excluded;
        let has_excluded = !excluded.is_null();

        self.stack[ss].tt_pv = if has_excluded {
            self.stack[ss].tt_pv
        } else {
            is_pv || (tt_data.hit && tt_data.is_pv)
        };

        // legality
        tt_data.pv =
            if !has_excluded && tt_data.hit && !tt_data.pv.is_null() && pos.is_legal(tt_data.pv) {
                tt_data.pv
            } else {
                Move::NULL_MOVE
            };
        let is_tt_capture = if tt_data.pv.is_null() {
            false
        } else {
            !pos.is_quiet(tt_data.pv)
        };

        //- always use pv of root_moves
        if is_root {
            tt_data.pv = self.root_moves[0].pv_list.pv();
        }

        //- tt early return
        if !is_pv
            && !has_excluded
            && tt_data.can_use
            && ((cut_node == (tt_data.score >= beta)) || depth > 5)
            && tt_data.depth >= depth + (tt_data.score >= beta) as i8
        {
            if pos.halfmove_clock() < 90 {
                return tt_data.score;
            }
        }

        //- adjusted/unadjusted evals
        let mut unadjusted_static = VALUE_NONE;
        let mut tt_static = VALUE_NONE;
        let in_check = pos.in_check();
        let mut complexity = 0;
        self.stack[ss].conseq_checks = 0;
        if in_check {
            self.stack[ss].conseq_checks = self.stack[ss - 2].conseq_checks + 1;
            self.stack[ss].adjusted_static = VALUE_NONE;
        } else if has_excluded {
            unadjusted_static = self.stack[ss].adjusted_static;
            tt_static = self.stack[ss].adjusted_static;
            self.nnue.catchup(pos);
        } else if tt_data.hit {
            unadjusted_static = tt_data.static_score;
            if !is_valid(unadjusted_static) {
                unadjusted_static = self.evaluate(pos);
            } else if is_pv {
                self.nnue.catchup(pos);
            }
            self.stack[ss].adjusted_static = self.static_correction(pos, unadjusted_static, ss);
            tt_static = self.stack[ss].adjusted_static;
            complexity = (unadjusted_static as i32 - self.stack[ss].adjusted_static as i32).abs();

            //- use tt score to improve static score
            let can_improve_static = get_can_use(
                tt_data.score,
                tt_data.flag,
                self.stack[ss].adjusted_static,
                self.stack[ss].adjusted_static,
            );
            if is_valid(tt_data.score) && !is_decisive(tt_data.score) && can_improve_static {
                tt_static = tt_data.score;
            }
        } else {
            unadjusted_static = self.evaluate(pos);
            self.stack[ss].adjusted_static = self.static_correction(pos, unadjusted_static, ss);
            tt_static = self.stack[ss].adjusted_static;
            complexity = (unadjusted_static as i32 - self.stack[ss].adjusted_static as i32).abs();

            writer.set(
                tt_key,
                Move::NULL_MOVE,
                ply,
                UNSEARCH_DEPTH,
                FLAG_NONE,
                VALUE_NONE,
                unadjusted_static,
                self.stack[ss].tt_pv,
                tt_age,
            );
        }

        //- syzygy
        let max_score = VALUE_INF;
        let mut best_score = -VALUE_INF;
        if !has_excluded
            && !is_root
            && TableBase::static_test(pos)
            && let Some(tb) = &self.tb
            && tb.test(pos)
            && let Some(wdl) = tb.query(pos)
        {
            self.tb_hits += 1;
            let tb_score = VALUE_SYZYGY - ply as i16;
            let (score, flag) = if wdl < -1 {
                (-tb_score, FLAG_ALPHA)
            } else if wdl > 1 {
                (tb_score, FLAG_BETA)
            } else {
                (VALUE_DRAW, FLAG_EXACT)
            };

            if get_can_use(score, flag, alpha, beta) {
                writer.set(
                    tt_key,
                    Move::NULL_MOVE,
                    ply,
                    i16::min(MAX_DEPTH as i16, depth as i16 + 5) as i8,
                    flag,
                    score,
                    VALUE_NONE,
                    self.stack[ss].tt_pv,
                    tt_age,
                );

                return score;
            }

            if flag == FLAG_BETA {
                best_score = score;
                alpha = alpha.max(score);
            }
        }

        let mut improving = false;
        if in_check {
            improving = false;
        } else if is_valid(self.stack[ss - 2].adjusted_static)
            && is_valid(self.stack[ss].adjusted_static)
        {
            improving = self.stack[ss].adjusted_static > self.stack[ss - 2].adjusted_static;
        } else if is_valid(self.stack[ss - 4].adjusted_static)
            && is_valid(self.stack[ss].adjusted_static)
        {
            improving = self.stack[ss].adjusted_static > self.stack[ss - 4].adjusted_static;
        }

        if !is_root && !in_check {
            //- razoring
            if !is_pv
                && is_valid(tt_static)
                && alpha < 2000
                && (tt_static as i32) < (alpha as i32 - 300 * depth as i32 * depth as i32)
            {
                let score = self.qsearch(pos, alpha, beta, 0, ss, false);
                if score <= alpha {
                    return score;
                }
            }

            //- static null move pruning
            let margin = 0.max(70 * (depth - improving as i8) as i32);
            if !is_pv
                && is_valid(tt_static)
                && !is_loss(beta)
                && !is_win(tt_static)
                && tt_static as i32 - margin >= beta as i32
                && depth <= 14
                && (tt_data.pv.is_null() || is_tt_capture)
            {
                return avg(beta, tt_static);
            }

            //- null move pruning
            let has_non_pawns = pos.has_non_pawns(pos.side_to_move());
            if cut_node
                && !self.stack[ss].verify_null
                && !has_excluded
                && has_non_pawns
                && !self.stack[ss - 1].m.is_null()
                && is_valid(tt_static)
                && !is_loss(beta)
                && tt_static as i32 >= beta as i32 + 200 - 30 * depth as i32
                && self.stack[ss].adjusted_static >= beta
            {
                let reduction = (6 + depth as i32 / 4)
                    + ((tt_static - beta) as i32 / 500).clamp(0, 3)
                    + is_tt_capture as i32;
                let reduced_depth = i32::max(0, depth as i32 - reduction) as i8;
                self.table.get().prefetch(pos.new_hash(Move::NULL_MOVE));
                let new_pos = self.make_move(pos, Move::NULL_MOVE, key, ss);
                let score = -self.negamax(
                    &new_pos,
                    -beta,
                    -beta + 1,
                    reduced_depth,
                    ss + 1,
                    false,
                    false,
                );
                self.unmake_move(pos, key, ss);

                if self.timer.read().unwrap().stopped() {
                    return 0;
                }

                if score >= beta && !is_win(score) {
                    return score;
                }
            }

            //- iir
            if (is_pv || cut_node) && depth >= (2 + 2 * cut_node as i8) && tt_data.pv.is_null() {
                depth -= 1;
            }

            //- probcut
            let probcut_beta = (beta as i32 + self.settings.p_probcut_beta)
                .clamp(-VALUE_EVAL as i32, VALUE_EVAL as i32);
            if !is_pv
                && depth >= PROBCUT_DEPTH_MIN
                && !is_decisive(beta)
                && is_valid(tt_static)
                &&
                    // also ignore when tt score is < probcut beta
                 !(tt_data.hit
                    && is_valid(tt_data.score)
                    && ((tt_data.score as i32) < probcut_beta)
                    && (tt_data.depth >= depth - 3))
            {
                let margin = (probcut_beta - tt_static as i32).clamp(-10000, 10000)
                    * self.settings.p_probcut_margin
                    / 1024;

                let mut tt_move = Move::NULL_MOVE;
                if is_tt_capture && see_ge(pos, tt_data.pv, margin) {
                    tt_move = tt_data.pv;
                }

                let mut movepick = Movepick::new_probcut(
                    pos.clone(),
                    tt_move,
                    ply,
                    depth,
                    &self.stack,
                    ss,
                    margin,
                    &self.heuristic,
                );
                let probcut_depth = depth - PROBCUT_DEPTH_REDUCTION;
                assert!(probcut_depth > 0);

                let probcut_beta = probcut_beta as i16;
                let mut move_count = 0;
                let mut best_score = -VALUE_INF;
                loop {
                    let next_move = movepick.next_move();
                    if next_move.is_null() {
                        break;
                    }

                    if next_move.inner == excluded {
                        continue;
                    }

                    move_count += 1;
                    self.table.get().prefetch(pos.new_hash(next_move.inner));
                    let new_pos = self.make_move(pos, next_move.inner, key, ss);
                    let mut score = -self.qsearch(
                        &new_pos,
                        -probcut_beta,
                        -probcut_beta + 1,
                        QDEPTH,
                        ss + 1,
                        false,
                    );
                    if score >= probcut_beta {
                        score = -self.negamax(
                            &new_pos,
                            -probcut_beta,
                            -probcut_beta + 1,
                            probcut_depth,
                            ss + 1,
                            false,
                            !cut_node,
                        );
                    }
                    self.unmake_move(pos, key, ss);

                    if self.timer.read().unwrap().stopped() {
                        return 0;
                    }

                    if score >= probcut_beta {
                        if !has_excluded {
                            writer.set(
                                tt_key,
                                next_move.inner,
                                ply,
                                probcut_depth,
                                FLAG_BETA,
                                score,
                                unadjusted_static,
                                self.stack[ss].tt_pv,
                                tt_age,
                            );
                        }

                        return (score as i32 - probcut_beta as i32 + beta as i32)
                            .clamp(-VALUE_EVAL as i32, VALUE_EVAL as i32)
                            as i16;
                    }

                    best_score = best_score.max(score);
                }

                // fut prune
                // if move_count >= 5
                //     && !is_decisive(alpha)
                //     && (best_score as i32) < (alpha as i32 - 300 - 300 * depth as i32)
                // {
                //     return best_score;
                // }
            }
        }

        let mut move_count = 0;
        let mut best_move = Move::NULL_MOVE;

        let mut quiets = MoveList::new();
        let mut captures = MoveList::new();
        let old_alpha = alpha;

        //- negamax alphabeta search
        let mut movepick = Movepick::new_negamax(
            pos.clone(),
            tt_data.pv,
            ply,
            depth,
            &self.stack,
            ss,
            self.pawn_key.get(),
            &self.heuristic,
        );
        loop {
            let next_move = movepick.next_move();
            if next_move.is_null() {
                break;
            }

            if next_move.inner == excluded {
                continue;
            }

            let is_quiet = pos.is_quiet(next_move.inner);
            move_count += 1;
            let old_nodes = self.nodes;
            self.table.get().prefetch(pos.new_hash(next_move.inner));

            //- low depth pruning
            if !is_root && !is_loss(best_score) && !in_check {
                let lmr_depth = depth as i32;

                //- see pruning
                let see_margin = if is_quiet {
                    self.settings.p_lowdepth_see_quiet_base
                        + self.settings.p_lowdepth_see_quiet_depth * lmr_depth * lmr_depth
                } else {
                    self.settings.p_lowdepth_see_capture_base
                        + self.settings.p_lowdepth_see_capture_depth * lmr_depth
                };
                if !see::see_ge(pos, next_move.inner, -see_margin) {
                    continue;
                }

                //- history prunes
                if is_quiet && next_move.get_score() < -6000 * depth as i32 {
                    movepick.skip_quiets();
                    continue;
                }

                //- late move pruning
                if move_count as i32 >= (3 + depth as i32 * depth as i32) / (2 - improving as i32) {
                    movepick.skip_quiets();
                }

                //- futility pruning
                if is_quiet
                    && lmr_depth < 12
                    && (tt_static as i32
                        + self.settings.p_lowdepth_fut_quiet_base
                        + self.settings.p_lowdepth_fut_quiet_depth * lmr_depth)
                        < (alpha as i32)
                {
                    movepick.skip_quiets();
                    continue;
                }

                //- capture futility pruning
                if !is_quiet
                    && lmr_depth < 12
                    && (tt_static as i32
                        + self.settings.p_lowdepth_fut_capture_base
                        + self.settings.p_lowdepth_fut_capture_depth * lmr_depth
                        + pesto_value(
                            pos,
                            ColoredPiece::new(
                                !pos.side_to_move(),
                                pos.get_captured(next_move.inner),
                            ),
                            next_move.inner.to,
                        ))
                        < (alpha as i32)
                {
                    continue;
                }
            }

            //- singular extension
            let mut extension = 0;
            if !is_root
                && !has_excluded
                && tt_data.pv == next_move.inner
                && is_valid(tt_data.score)
                && !is_decisive(tt_data.score)
                && (tt_data.flag == FLAG_EXACT || tt_data.flag == FLAG_BETA)
                && tt_data.depth >= depth - 3
                && depth >= 5
            {
                let to_beat = tt_data.score - depth as i16;
                let reduced_depth = (depth - 1) / 2;
                self.stack[ss].excluded = tt_data.pv;
                let next_best_score = self.negamax(
                    pos,
                    to_beat - 1,
                    to_beat,
                    reduced_depth,
                    ss,
                    false,
                    cut_node,
                );
                self.stack[ss].excluded = Move::NULL_MOVE;

                if self.timer.read().unwrap().stopped() {
                    return 0;
                }

                if next_best_score < to_beat {
                    // extend
                    if !is_pv
                        && ((next_best_score as i32)
                            < (to_beat as i32 - self.settings.p_singular_double))
                    {
                        if is_quiet
                            && ((next_best_score as i32)
                                < (to_beat as i32 - self.settings.p_singular_triple))
                        {
                            extension = 3;
                        } else {
                            extension = 2;
                        }
                    } else {
                        extension = 1;
                    }
                } else if next_best_score >= beta {
                    // multi cut
                    return next_best_score;
                } else if tt_data.score >= beta {
                    extension = -3 + self.stack[ss].tt_pv as i8;
                } else if cut_node {
                    extension = -2;
                }
            }

            let new_pos = self.make_move(pos, next_move.inner, key, ss);
            let mut new_depth = (depth + extension - 1).max(0);
            let mut score = 0;

            //- late move reduction
            if depth >= 2 && move_count > 1 + 2 * is_root as usize {
                let mut reduction = self.heuristic.get_lmr(move_count, depth);

                // check extension
                if self.stack[ss].conseq_checks < 4 && new_pos.in_check() {
                    reduction -= self.settings.p_lmr_check;
                }

                // cutnode reduction
                if cut_node {
                    reduction += (2 - self.stack[ss].tt_pv as i32) * self.settings.p_lmr_cutnode;
                }

                // capture reduction
                if is_tt_capture && is_quiet {
                    reduction += self.settings.p_lmr_capture;
                }

                if !improving {
                    reduction += self.settings.p_lmr_improving;
                }

                if tt_data.depth >= depth {
                    reduction -= self.settings.p_lmr_tt_depth;
                }

                reduction -= (complexity * self.settings.p_lmr_complexity / 200).min(3 * 1024);

                // pv extension
                reduction -= (self.stack[ss].tt_pv as i32 + is_pv as i32) * self.settings.p_lmr_pv;

                // history adjustment
                let scaled_history_score = next_move.get_score() * self.settings.p_lmr_history
                    / if is_quiet {
                        self.settings.p_lmr_quiet_div
                    } else {
                        self.settings.p_lmr_capture_div
                    };
                reduction -= scaled_history_score as i32;

                reduction /= 1024;
                let reduced_depth =
                    (new_depth as i32 - reduction).clamp(1, new_depth as i32 + 1) as i8;

                //- pv search
                score = -self.negamax(
                    &new_pos,
                    -(alpha + 1),
                    -alpha,
                    reduced_depth,
                    ss + 1,
                    false,
                    true,
                );

                if score > alpha && reduced_depth < new_depth {
                    //- re-search adjustments
                    if (score as i32) > (best_score as i32 + 50) {
                        new_depth += 1;
                    }
                    if (score as i32) < (best_score as i32 + 5) {
                        new_depth -= 1;
                    }

                    if reduced_depth < new_depth {
                        score = -self.negamax(
                            &new_pos,
                            -(alpha + 1),
                            -alpha,
                            new_depth,
                            ss + 1,
                            false,
                            !cut_node,
                        );
                    }
                }
            } else if !is_pv || move_count > 1 {
                score = -self.negamax(
                    &new_pos,
                    -(alpha + 1),
                    -alpha,
                    new_depth,
                    ss + 1,
                    false,
                    !cut_node,
                );
            }

            if is_pv && (move_count == 1 || score > alpha) {
                score = -self.negamax(&new_pos, -beta, -alpha, new_depth, ss + 1, true, false);
            }

            self.unmake_move(pos, key, ss);

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

                root_move.nodes += self.nodes - old_nodes;
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
                if pos.is_quiet(next_move.inner) {
                    quiets.push(next_move.inner);
                } else {
                    captures.push(next_move.inner);
                }
            }
        }

        if is_pv {
            best_score = best_score.min(max_score);
        }

        if move_count == 0 {
            if has_excluded {
                best_score = alpha;
            } else if in_check {
                best_score = lose_in(ply);
            } else {
                best_score = VALUE_DRAW;
            }
        } else if best_score >= beta {
            let history_depth =
                depth + (best_score as i32 > beta as i32 + 200) as i8;
            self.heuristic.update_history(
                pos,
                history_depth,
                1,
                ply,
                best_move,
                &self.stack,
                ss,
                self.pawn_key.get(),
                &captures,
                &quiets,
            );
        }

        //- tt_pv propagation
        if best_score < old_alpha {
            self.stack[ss].tt_pv = self.stack[ss].tt_pv || self.stack[ss - 1].tt_pv;
        }

        let flag = if best_score >= beta {
            FLAG_BETA
        } else if is_pv && !best_move.is_null() {
            FLAG_EXACT
        } else {
            FLAG_ALPHA
        };

        if !has_excluded {
            //- tt update
            writer.set(
                tt_key,
                best_move,
                ply,
                depth,
                flag,
                best_score,
                unadjusted_static,
                self.stack[ss].tt_pv,
                tt_age,
            );
        }

        //- correction history
        let adjusted_static = self.stack[ss].adjusted_static;
        if is_valid(self.stack[ss].adjusted_static)
            && !in_check
            // pv not a capture
            && !(!best_move.is_null() && !pos.is_quiet(best_move))
            && (flag == FLAG_EXACT || (best_score >= adjusted_static && flag == FLAG_BETA) || (best_score < adjusted_static && flag == FLAG_ALPHA))
        {
            let bonus = ((best_score as i32 - adjusted_static as i32) * depth as i32 / 8)
                .clamp((-CORR_LIMIT / 4) as i32, (CORR_LIMIT / 4) as i32)
                as i16;
            self.heuristic
                .get_pawn_corrhist(pos, self.pawn_key.get())
                .add(bonus);

            let [white, black] = self.pawn_key.get_colored();
            self.heuristic.get_white_corrhist(pos, white).add(bonus);
            self.heuristic.get_black_corrhist(pos, black).add(bonus);
            self.heuristic
                .get_major_corrhist(pos, self.pawn_key.get_major())
                .add(bonus);
            self.heuristic
                .get_minor_corrhist(pos, self.pawn_key.get_minor())
                .add(bonus);

            // let pinners_key = self.pawn_key.get_pinners(pos);
            // if pinners_key != 0 {
            //     self.heuristic
            //         .get_pinners_corrhist(pos, pinners_key)
            //         .add(bonus);
            // }

            let prev = self.stack[ss - 1].m;
            if !prev.is_null() && !self.stack[ss - 2].m.is_null() {
                self.heuristic
                    .get_cont_corrhist(self.stack[ss - 2].cont_corrhist)
                    [self.stack[ss - 1].piece.unwrap().index()][prev.to as usize]
                    .add(bonus);
            }
        }

        best_score
    }

    fn static_correction(&mut self, pos: &Board, static_score: i16, ss: usize) -> i16 {
        let mut static_score = static_score as i32;
        static_score += 32
            * self
                .heuristic
                .get_pawn_corrhist(pos, self.pawn_key.get())
                .get() as i32
            / 512;

        let [white, black] = self.pawn_key.get_colored();
        static_score += 24 * self.heuristic.get_white_corrhist(pos, white).get() as i32 / 512;
        static_score += 24 * self.heuristic.get_black_corrhist(pos, black).get() as i32 / 512;
        static_score += 24
            * self
                .heuristic
                .get_major_corrhist(pos, self.pawn_key.get_major())
                .get() as i32
            / 512;
        static_score += 24
            * self
                .heuristic
                .get_minor_corrhist(pos, self.pawn_key.get_minor())
                .get() as i32
            / 512;

        // let pinners_key = self.pawn_key.get_pinners(pos);
        // if pinners_key != 0 {
        //     static_score +=
        //         24 * self.heuristic.get_pinners_corrhist(pos, pinners_key).get() as i32 / 512;
        // }

        let prev = self.stack[ss - 1].m;
        if !prev.is_null() {
            static_score += 24
                * self
                    .heuristic
                    .get_cont_corrhist(self.stack[ss - 2].cont_corrhist)
                    [self.stack[ss - 1].piece.unwrap().index()][prev.to as usize]
                    .get() as i32
                / 512;
        }

        static_score.clamp(-VALUE_EVAL as i32, VALUE_EVAL as i32) as i16
    }

    pub fn search(&mut self, startpos: Board, moves: Vec<Move>) -> SearchResult {
        self.nodes = 0;
        self.tb_hits = 0;
        self.key_stack.clear();
        self.table.get().next_search();
        self.heuristic.next_search();

        // history tracking
        let mut pos = startpos;
        for m in moves.iter() {
            let key = pos.correct_hash();
            if pos.halfmove_clock() == 0 {
                self.key_stack.clear();
            };
            self.key_stack.push(key);
            pos.play_unchecked(*m);
        }

        self.nnue.init(&pos);
        self.pawn_key.init(&pos);

        // root moves
        let mut root_moves = vec![];
        for m in pos.get_legal_moves().iter() {
            root_moves.push(RootMove::new(&m));
        }
        self.root_moves = root_moves.into_boxed_slice();
        assert!(!self.root_moves.is_empty(), "root moves is empty");

        // poll root
        if TableBase::static_test_root(&pos)
            && let Some(tb) = &self.tb
            && tb.test(&pos)
            && let Some((score, m)) = tb.query_root(&pos)
        {
            println!(
                "info depth {} score cp {} time {} pv {}",
                MAX_DEPTH,
                score,
                0,
                m.to_uci(&pos)
            );
            println!("bestmove {}", m.to_uci(&pos));
            return SearchResult {
                root: RootMove::new(&m),
                depth: MAX_DEPTH,
            };
        }

        // search stack
        for i in 0..SS_SIZE_PRE {
            self.stack[i] = SearchStack::new();
        }
        for i in 0..(SS_SIZE - SS_SIZE_PRE) {
            self.stack[SS_SIZE_PRE + i] = SearchStack::new_ply(i as i8);
        }

        let mut instability = 0;
        let mut last_best_move = Move::NULL_MOVE;
        let mut last_best_score = VALUE_NONE;

        // iterative deepening
        let mut depth = 1;
        while depth <= self.timer.read().unwrap().max_depth {
            let mut alpha = -VALUE_INF;
            let mut beta = VALUE_INF;

            if self.root_moves.len() == 1 && depth >= 2 {
                break;
            }

            let average_score = self.root_moves[0].average_score;
            let mut window = if is_valid(average_score) {
                ASP_WINDOW + (average_score as i32 * average_score as i32 / ASP_WINDOW_SCORE_SCALE)
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

            let mut fail_highs = 0;

            // asp window
            loop {
                assert!(alpha < beta, "alpha beta invariance {} {}", alpha, beta);
                let reduced_depth = (depth - fail_highs).max(1);
                let score =
                    self.negamax(&pos, alpha, beta, reduced_depth, SS_SIZE_PRE, true, false);
                if fail_highs <= 1 {
                    self.sort_root_moves();
                }

                if self.timer.read().unwrap().stopped() {
                    break;
                }

                if score <= alpha {
                    beta = avg(alpha, beta);
                    alpha = (-VALUE_INF as i32).max(score as i32 - window as i32) as i16;

                    fail_highs = 0;
                } else if score >= beta {
                    beta = (VALUE_INF as i32).min(score as i32 + window as i32) as i16;

                    if score < 2000 {
                        fail_highs += 1;
                    }
                } else {
                    break;
                }

                // need [ASP_WINDOW_MAX_SIZE] to be small enough to prevent overflow
                window += window / ASP_WINDOW_SCALE;
            }

            // force exit
            if self.timer.read().unwrap().stopped() {
                break;
            }

            let best = self.root_moves[0].pv_list.pv();
            let best_score = self.root_moves[0].score;
            let mut factors = 1.0;
            if depth > 1 {
                if best != last_best_move {
                    instability = (instability + 1).min(8);
                } else {
                    instability = 0;
                }

                let instability_factor = instability as f64 * 0.02;
                let score_factor =
                    ((best_score as f64 - last_best_score as f64) * -0.002).clamp(-0.3, 0.3);

                let prev_score_factor = if let Some(prev_score) = self.prev_score {
                    ((best_score as f64 - prev_score as f64) * -0.002).clamp(-0.3, 0.3)
                } else {
                    0.0
                };

                let nodes_factor =
                    (0.8 - self.root_moves[0].nodes as f64 / self.nodes as f64).clamp(-0.4, 0.4);
                factors *= (1.0 + instability_factor)
                    * (1.0 + score_factor)
                    * (1.0 + prev_score_factor)
                    * (1.0 + nodes_factor);
            }
            last_best_move = best;
            last_best_score = best_score;

            // opt exit
            if self
                .timer
                .read()
                .unwrap()
                .test((self.timer.read().unwrap().opt_time as f64 * factors) as i128)
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
                "info depth {} score {} nodes {} time {} nps {} hashfull {} tbhits {} pv",
                depth,
                score_str,
                self.nodes,
                delta,
                nps,
                self.table.clone().get().hashfull(),
                self.tb_hits
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

        self.prev_score = Some(result.root.score);

        result
    }
}
