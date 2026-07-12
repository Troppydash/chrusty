use shakmaty::{Board, Chess, Position};

use crate::{engine::Engine};
mod engine;
mod ext;
mod heuristic;
mod movepick;
mod param;
mod timer;
mod tt;
mod uci;
mod pesto;

fn main() {
    pesto::init();
    uci::start();
}
