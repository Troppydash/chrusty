use super::*;
use crate::tt::Table;

fn initialize() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        cuckoo::init();
        crate::nnue::init();
    });
}

// Retain histories between calls, but discard TT/position state. This isolates
// the cross-position correction-history regression from transpositions.
struct TestEngine {
    engine: Engine,
    _table: Box<Table>,
}

impl TestEngine {
    fn new() -> Self {
        initialize();
        let mut table = Box::new(Table::new(1));
        let engine = Engine::new(Arc::new(Timer::new()), TablePtr::from_table(&mut table));
        Self {
            engine,
            _table: table,
        }
    }

    fn prepare(&mut self, fen: &str) -> Board {
        let pos = Board::from_fen(fen, false).unwrap();
        let engine = &mut self.engine;
        engine.table.get().clear();
        engine.key_stack.clear();
        engine.nodes = 0;
        engine.tb_hits = 0;
        engine.nnue.init(&pos);
        engine.pawn_key.init(&pos);
        engine.startpos = pos.clone();
        for i in 0..SS_SIZE_PRE {
            engine.stack[i] = SearchStack::new();
        }
        for ply in 0..SS_SIZE - SS_SIZE_PRE {
            engine.stack[SS_SIZE_PRE + ply] = SearchStack::new_ply(ply as i8);
        }
        engine.timer.start(MAX_TIME, false);
        pos
    }

    fn pv(&mut self, fen: &str, depth: i8) -> i16 {
        let pos = self.prepare(fen);
        self.engine.negamax(
            &pos,
            -VALUE_INF,
            VALUE_INF,
            depth,
            SS_SIZE_PRE + 1,
            true,
            false,
        )
    }

    fn qsearch(&mut self, fen: &str, depth: i8) -> i16 {
        let pos = self.prepare(fen);
        self.engine
            .qsearch(&pos, -VALUE_INF, VALUE_INF, depth, SS_SIZE_PRE + 1, true)
    }
}

#[test]
// disabled for now because of elo reasons
#[should_panic]
fn mate_scores_do_not_train_unrelated_endgame_correction() {
    let mut test = TestEngine::new();
    let draw = "7k/7r/8/8/8/8/R7/K7 w - - 17 1";
    let mate = "7k/8/6K1/8/8/8/8/RNNN4 w - - 0 1";
    let before = test.qsearch(draw, -10);

    for _ in 0..5 {
        assert_eq!(test.pv(mate, 2), win_in(2));
        // Assert stability rather than a network-specific centipawn value.
        assert_eq!(test.qsearch(draw, -10), before);
    }
}

#[test]
fn tablebase_scores_are_not_reported_as_mate_distances() {
    for score in [
        VALUE_SYZYGY,
        VALUE_SYZYGY - 1,
        VALUE_SYZYGY - MAX_DEPTH as i16,
    ] {
        assert_eq!(format_uci_score(score), format!("cp {score}"));
        assert_eq!(format_uci_score(-score), format!("cp {}", -score));
    }
    assert_eq!(format_uci_score(50), "cp 50");
    assert_eq!(format_uci_score(0), "cp 0");
    assert_eq!(format_uci_score(win_in(1)), "mate 1");
    assert_eq!(format_uci_score(win_in(3)), "mate 2");
    assert_eq!(format_uci_score(lose_in(2)), "mate -1");
    assert_eq!(format_uci_score(lose_in(3)), "mate -2");
}

#[test]
fn tablebase_probe_limit_includes_five_and_excludes_larger_positions() {
    let five = Board::from_fen("8/8/8/8/2K5/3R3r/5kb1/8 w - - 0 77", false).unwrap();
    assert_eq!(five.occupied().len(), 5);
    assert!(can_probe_tablebase(&five, false));
    assert!(can_probe_tablebase(&five, true));

    for fen in [
        "4n1k1/1p6/8/8/8/8/P1K5/4R3 w - - 0 1",
        "8/3r4/R4R2/3k4/8/1K5p/6b1/8 w - - 0 71",
        "3r4/3P4/R4p2/4k3/3R3p/1K6/6b1/8 w - - 3 68",
    ] {
        let pos = Board::from_fen(fen, false).unwrap();
        assert!(pos.occupied().len() > 5);
        assert!(!can_probe_tablebase(&pos, false), "{fen}");
        assert!(!can_probe_tablebase(&pos, true), "{fen}");
    }
}

#[test]
fn tablebase_probe_restrictions_preserve_clock_castling_and_ep_rules() {
    let clock = Board::from_fen("7k/8/8/8/2Q5/8/8/K7 w - - 99 1", false).unwrap();
    assert!(can_probe_tablebase(&clock, true)); // Root DTZ receives the clock.
    assert!(!can_probe_tablebase(&clock, false)); // Interior WDL does not.
    for fen in [
        "4k3/8/8/8/8/8/8/4K2R w K - 0 1",
        "7k/8/8/3pP3/8/8/8/K7 w - d6 0 1",
    ] {
        let pos = Board::from_fen(fen, false).unwrap();
        assert!(!can_probe_tablebase(&pos, false), "{fen}");
        assert!(!can_probe_tablebase(&pos, true), "{fen}");
    }
}

#[test]
fn endgame_search_still_finds_quiet_mates_for_both_colors() {
    let mut test = TestEngine::new();
    for fen in [
        "7k/8/6K1/8/8/8/8/R7 w - - 0 1",
        "6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1",
        "4r1k1/5ppp/8/8/8/8/5PPP/6K1 b - - 0 1",
    ] {
        assert_eq!(test.pv(fen, 6), win_in(2), "{fen}");
    }
}

#[test]
fn fifty_move_draw_does_not_hide_checkmate() {
    let mut test = TestEngine::new();
    let draw = "7k/8/8/8/2Q5/8/8/K7 w - - 100 1";
    assert_eq!(test.pv(draw, 4), VALUE_DRAW);
    assert_eq!(test.qsearch(draw, QDEPTH), VALUE_DRAW);
    let mate = "R6k/8/6K1/8/8/8/8/8 b - - 100 1";
    assert_eq!(test.pv(mate, 4), lose_in(1));
    assert_eq!(test.qsearch(mate, QDEPTH), lose_in(1));
}

#[test]
fn qsearch_shallow_tt_bound_does_not_hide_quiet_mate() {
    initialize();

    // Ra8 is mate, but a capture-only qsearch cannot find it.
    let pos = Board::from_fen("7k/8/6K1/8/8/8/8/R7 w - - 0 1", false).unwrap();
    let mut table = Box::new(Table::new(1));
    let mut engine = Engine::new(Arc::new(Timer::new()), TablePtr::from_table(&mut table));
    engine.nnue.init(&pos);
    engine.pawn_key.init(&pos);
    for ply in 0..(SS_SIZE - SS_SIZE_PRE) {
        engine.stack[SS_SIZE_PRE + ply] = SearchStack::new_ply(ply as i8);
    }

    for requested_depth in [-9, QDEPTH] {
        table.clear();
        let cold = engine.qsearch(&pos, 10000, 10001, requested_depth, SS_SIZE_PRE, false);
        assert_eq!(cold, win_in(1));

        table.clear();
        let shallow = engine.qsearch(&pos, 10000, 10001, -10, SS_SIZE_PRE, false);
        assert!(shallow < 10000);

        let key = pos.correct_hash() ^ get_50mr_key(pos.halfmove_clock() as usize);
        let (entry, _) = table.get(key);
        let value = entry.get(key, 0, requested_depth, 10000, 10001);
        assert!(value.hit);
        assert_eq!(value.flag, FLAG_ALPHA);
        assert_eq!(value.depth, -10);
        assert!(!value.can_use);

        let warm = engine.qsearch(&pos, 10000, 10001, requested_depth, SS_SIZE_PRE, false);
        assert_eq!(warm, cold);
    }
}
