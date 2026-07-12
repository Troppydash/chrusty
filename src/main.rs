use crate::engine::Engine;
mod engine;
mod ext;
mod heuristic;
mod movepick;
mod param;
mod pesto;
mod timer;
mod tt;
mod uci;

fn main() {
    pesto::init();
    uci::start();
}
