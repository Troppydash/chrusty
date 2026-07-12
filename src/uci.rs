use crate::param::{MAX_DEPTH, MAX_NODES, MAX_TIME};
use crate::timer::Timer;
use crate::{Chess, Engine};
use std::process::exit;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

use rustyline::DefaultEditor;

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
}

impl AsyncEngine {
    fn new() -> Self {
        let timer = Arc::new(RwLock::new(Timer::new()));
        let engine = Arc::new(Mutex::new(Engine::new(timer.clone())));
        Self {
            timer,
            engine,
            handle: None,
        }
    }

    fn newgame(&mut self) {}

    fn search(&mut self, mut pos: Chess) {
        self.stop();
        assert!(self.handle.is_none());

        let engine = self.engine.clone();
        self.handle = Some(thread::spawn(move || {
            engine.lock().unwrap().search(&mut pos);
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
}

pub fn start() {
    let mut async_engine = AsyncEngine::new();
    let mut rl = DefaultEditor::new().unwrap();
    loop {
        let line = rl.readline(">");
        let line = match line {
            Err(_) => {
                break;
            }
            Ok(line) => line,
        };

        let parts = line.split_whitespace().collect::<Vec<&str>>();
        if parts.len() == 0 {
            println!("warn empty input");
            continue;
        }
        match parts[0] {
            "quit" => {
                break;
            },
            "uci" => {
                println!("id name Chrusty");
                println!("id authors A Chinese boy and a Vietnamese man");
                println!("");
                println!("uciok");
            }
            "go" => {
                // let mut pos = Chess::new();
                // let mut search_limits = SearchLimits::new();
                // let args: Vec<&str> = parts.into_iter().collect();
                // if args.len() == 0 {
                //     search_limits.max_time = MAX_TIME;
                //     search_limits.opt_time = OPT_TIME;
                // } else {
                //     let mut i = 0;
                //     while i < args.len() {
                //         match args[i] {
                //             "depth" => {
                //                 search_limits.max_depths = args.get(i + 1).and_then(|s| s.parse::<i8>().ok()).is_none_or(f);
                //                 i += 2;
                //             }
                //             "node" => {
                //                 search_limits.max_nodes =
                //                     args.get(i + 1).and_then(|s| s.parse::<i64>().ok());
                //                 i += 2;
                //             }
                //             // "opt_time" => {
                //             //     search_limits.max =
                //             //         args.get(i + 1).and_then(|s| s.parse::<u128>().ok());
                //             //     i += 2;
                //             // }
                //             "max_time" => {
                //                 search_limits.max_time =
                //                     args.get(i + 1).and_then(|s| s.parse::<u128>().ok());
                //                 i += 2;
                //             }
                //             _ => {
                //                 i += 1;
                //             }
                //         }
                //     }
                // }

                let pos = Chess::new();
                async_engine.search(pos);
            }
            "stop" => {
                async_engine.stop();
            }
            "setoption" => {
                // can ignore for now
                todo!()
            }
            "position" => {
                // can ignore
                todo!()
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
    }
}
