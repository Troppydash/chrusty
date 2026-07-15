use crate::Engine;
use crate::ext::ExtMove;
use crate::param::{MAX_DEPTH, MAX_NODES, MAX_TIME};
use crate::timer::Timer;
use crate::tt::{Table, TablePtr};
use std::pin::Pin;
use std::process::exit;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

use cozy_chess::Board;
use cozy_chess::Move;
use rustyline::DefaultEditor;

pub const DEFAULT_TT_SIZE: usize = 16;

pub struct SearchLimits {
    pub max_depths: i8,
    pub max_nodes: i64,
    pub max_time: u128,
    pub opt_time: u128,
}

impl SearchLimits {
    pub fn new() -> Self {
        Self {
            max_depths: MAX_DEPTH,
            max_nodes: MAX_NODES,
            max_time: MAX_TIME,
            opt_time: MAX_TIME,
        }
    }
}

struct AsyncEngine {
    timer: Arc<RwLock<Timer>>,
    engine: Arc<Mutex<Engine>>,
    handle: Option<JoinHandle<()>>,
    table: Pin<Box<Table>>,
}

impl AsyncEngine {
    fn new() -> Self {
        let timer = Arc::new(RwLock::new(Timer::new()));
        let mut table = Pin::new(Box::new(Table::new(DEFAULT_TT_SIZE)));
        let table_ptr = TablePtr::from_table(&mut table);
        let engine = Arc::new(Mutex::new(Engine::new(timer.clone(), table_ptr)));
        Self {
            timer,
            engine,
            handle: None,
            table,
        }
    }

    fn newgame(&mut self) {
        self.engine.lock().unwrap().newgame();
        self.table.clear();
    }

    fn start_timer(&mut self, limits: &SearchLimits) {
        let mut timer = self.timer.write().unwrap();
        timer.max_depth = limits.max_depths;
        timer.max_nodes = limits.max_nodes;
        timer.opt_time = limits.opt_time;
        timer.start(limits.max_time);
    }

    fn search(&mut self, startpos: Board, moves: Vec<Move>) {
        let engine = self.engine.clone();
        self.handle = Some(thread::spawn(move || {
            engine.lock().unwrap().search(startpos, moves);
        }));
    }

    fn stop(&mut self) {
        let handle = std::mem::take(&mut self.handle); // self.handle = None
        assert!(self.handle.is_none());

        if let Some(handle) = handle {
            self.timer.write().unwrap().force_stop();
            handle.join().unwrap();
        }
    }

    fn resize_table(&mut self, size_in_mbytes: usize) {
        self.table.resize(size_in_mbytes);
    }
}

impl Drop for AsyncEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start(args: Vec<String>) {
    let mut async_engine = AsyncEngine::new();
    let mut rl = DefaultEditor::new().unwrap();
    let mut startpos = Board::startpos();
    let mut pos = Board::startpos();
    let mut moves = vec![];

    let mut process_line = |line: &String| -> () {
        let parts = line.split_whitespace().collect::<Vec<&str>>();
        if parts.len() == 0 {
            println!("warn empty input");
            return;
        }
        match parts[0] {
            "quit" => {
                exit(0);
            }
            "uci" => {
                println!("id name Chrusty");
                println!("id authors A Chinese boy and a Vietnamese man");
                println!("");
                println!("option name Threads type spin default 1 min 1 max 1");
                println!(
                    "option name Hash type spin default {} min 8 max 16000",
                    DEFAULT_TT_SIZE
                );
                println!("uciok");
            }
            "position" => {
                // position startpos moves <move1> <move2>
                // position fen <fen> moves <move1> <move2>

                let mut offset = 1;
                match parts[1] {
                    "startpos" => {
                        startpos = Board::startpos();
                        offset = 3;
                    }
                    "fen" => {
                        let fen = format!(
                            "{} {} {} {} {} {}",
                            parts[2], parts[3], parts[4], parts[5], parts[6], parts[7]
                        );
                        startpos = Board::from_fen(&fen, false).unwrap();
                        offset = 9;
                    }
                    _ => {
                        panic!("unknown position type {}", parts[1]);
                    }
                }

                moves.clear();
                pos = startpos.clone();
                for i in offset..parts.len() {
                    let m = Move::from_uci(parts[i], &pos);
                    moves.push(m);
                    pos.play_unchecked(m);
                }
            }
            "go" => {
                // go [movetime 1000 depth 10 nodes 10000] [wtime 35000 winc 100 btime 1000 binc 100]
                let mut search_limits = SearchLimits::new();
                let mut is_competitive = false;
                let mut wtime = 0;
                let mut winc = 0;
                let mut btime = 0;
                let mut binc = 0;

                let mut i = 1;
                while i < parts.len() {
                    match parts[i] {
                        "depth" => {
                            search_limits.max_depths = parts[i + 1].parse::<i8>().unwrap();
                            i += 2;
                        }
                        "nodes" => {
                            search_limits.max_nodes = parts[i + 1].parse::<i64>().unwrap();
                            i += 2;
                        }
                        "movetime" => {
                            let time = parts[i + 1].parse::<u128>().unwrap();
                            search_limits.opt_time = time;
                            search_limits.max_time = time;
                            i += 2;
                        }
                        "wtime" => {
                            is_competitive = true;
                            wtime = parts[i + 1].parse::<u128>().unwrap();
                            i += 2;
                        }
                        "winc" => {
                            is_competitive = true;
                            winc = parts[i + 1].parse::<u128>().unwrap();
                            i += 2;
                        }

                        "btime" => {
                            is_competitive = true;
                            btime = parts[i + 1].parse::<u128>().unwrap();
                            i += 2;
                        }

                        "binc" => {
                            is_competitive = true;
                            binc = parts[i + 1].parse::<u128>().unwrap();
                            i += 2;
                        }
                        _ => {
                            println!("unknown option {}, skipping", parts[i]);
                            i += 1;
                        }
                    }
                }

                if is_competitive {
                    let (time, inc) = match pos.side_to_move() {
                        cozy_chess::Color::White => (wtime, winc),
                        cozy_chess::Color::Black => (btime, binc),
                    };

                    let opt_time = (time / 20 + inc / 2).max(10);
                    let max_time = (time * 8 / 10).max(opt_time);
                    search_limits.opt_time = opt_time;
                    search_limits.max_time = max_time;
                }

                async_engine.stop();
                async_engine.start_timer(&search_limits);
                async_engine.search(startpos.clone(), moves.clone());
            }
            "stop" => {
                async_engine.stop();
            }
            "setoption" => {
                // setoption name <name> value <value>
                match parts[2] {
                    "Threads" => {}
                    "Hash" => {
                        let size = parts[4].parse::<usize>().unwrap();
                        async_engine.resize_table(size);
                    }
                    _ => {
                        println!("unknown option {}, skipping", parts[2]);
                    }
                }
            }
            "ucinewgame" => {
                async_engine.newgame();
            }
            "isready" => {
                println!("readyok");
            }
            _ => {
                println!("warn unknown command {}", parts[0]);
            }
        }
    };

    for arg in args.iter().skip(1) {
        process_line(arg);
    }

    loop {
        let line = rl.readline("");
        let line = match line {
            Err(_) => {
                break;
            }
            Ok(line) => line,
        };

        rl.add_history_entry(&line).unwrap();
        process_line(&line);
    }
}
