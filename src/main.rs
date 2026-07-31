use std::env;

use crate::engine::Engine;
mod cuckoo;
mod engine;
mod ext;
mod helpers;
mod heuristic;
mod movepick;
mod nnue;
mod param;
mod pesto;
mod rep;
mod see;
mod stack;
mod timer;
mod tt;
mod uci;

fn main() {
    cuckoo::init();
    let args: Vec<String> = env::args().collect();
    uci::start(args);
}
