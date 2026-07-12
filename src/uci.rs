use std::io::{Write, stdin, stdout};
use crate::engine::SearchLimits;
use crate::param::{MAX_TIME, OPT_TIME};
use crate::{Engine, Chess};

pub fn start() {
    println!("Welcome to the chess engine Chrusty");
    let mut engine = Engine::new();
    loop {
        print!(">> ");
        let mut s = String::new();
        let _ = stdout().flush();
        stdin().read_line(&mut s).expect("Did not enter a correct string");
        if let Some('\n') = s.chars().next_back() {
            s.pop();
        }
        if let Some('\r') = s.chars().next_back() {
            s.pop();
        }

        let mut parts = s.split_whitespace();
        let command = parts.next().unwrap_or("");

        match command {
            "quit" => break,
            "uci" => {
                println!("id name: Chess engine Chrusty");
                println!("id authors: A Chinese boy and a Vietnamese man")
            },
            "go" => {
                let mut pos = Chess::new();
                let mut search_limits = SearchLimits::new();
                let args: Vec<&str> = parts.collect();
                if args.len() == 0 {
                    search_limits.max_time = Some(MAX_TIME);
                    search_limits.opt_time = Some(OPT_TIME);
                } else {
                    let mut i = 0;
                    while i < args.len() {
                        match args[i] {
                            "depth" => {
                                search_limits.depths = args.get(i+1).and_then(|s| s.parse::<i8>().ok());
                                i += 2;
                            },
                            "node" => {
                                search_limits.nodes = args.get(i+1).and_then(|s| s.parse::<i64>().ok());
                                i += 2;
                            },
                            "opt_time" => {
                                search_limits.opt_time = args.get(i+1).and_then(|s| s.parse::<u128>().ok());
                                i += 2;
                            },
                            "max_time" => {
                                search_limits.max_time = args.get(i+1).and_then(|s| s.parse::<u128>().ok());
                                i += 2;
                            },
                            _=> {
                                i += 1;
                            }
                        }
                    }
                }
                engine.search(&mut pos, &search_limits);
            }
            _ => continue
        }
    }
}
