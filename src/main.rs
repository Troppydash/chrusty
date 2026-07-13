use std::env;

use crate::engine::Engine;
mod engine;
mod ext;
mod heuristic;
mod movepick;
mod param;
mod pesto;
mod rep;
mod timer;
mod tt;
mod uci;

fn main() {
    pesto::init();

    let args: Vec<String> = env::args().collect();
    uci::start(args);
}
