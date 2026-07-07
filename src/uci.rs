use rustyline::DefaultEditor;

use crate::engine::Engine;

struct AsyncEngine {

}

impl AsyncEngine {
    fn new() -> Self {
        Self {}
    }

    fn newgame(&mut self) {

    }

    fn search(&mut self) {

    }

    fn stop(&mut self) {
        
    }
}

pub fn start() {
    let mut engine = Engine::new();
    let mut rl = DefaultEditor::new().unwrap();
    loop {
        let line = rl.readline("");
        let line = match line {
            Err(_) => {
                break;
            }
            Ok(line) => line,
        };

        let parts = line.split(" ").collect::<Vec<&str>>();
        if parts.len() == 0 {
            println!("warn empty input");
            continue;
        }

        match parts[0] {
            "uci" => {
                println!("id name chrusty\n\
                \n\
                option name Threads type spin default 1 min 1 max 1\n\
                option name Hash type spin default 32 min 8 max 16000\n\
                uciok");
            }
            "setoption" => {
                todo!()
            }
            "position" => {
                todo!()
            }
            "ucinewgame" => {
                todo!()
            }
            "isready" => {
                println!("readyok");
            }
            "go" => {
                todo!()
            }
            "stop" => {
                todo!()
            }
            _ => {
                println!("warn unknown command {}", parts[0]);
            }
        }
    }
}
