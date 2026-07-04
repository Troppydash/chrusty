use shakmaty::{Board, Chess, Position};

use crate::engine::Engine;
mod engine;
mod ext;
mod param;
mod movepick;


fn main() {

    // TODO: uci

    let mut engine = Engine::new();
    let mut pos = Chess::new();
    engine.search(&mut pos, 10000, 10000);
}
