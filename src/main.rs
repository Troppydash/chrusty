use shakmaty::{Board, Chess, Position};

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
    // TODO: uci

    let mut engine = Engine::new();
    let mut pos = Chess::new();
    engine.search(&mut pos, 10000, 10000);
}
