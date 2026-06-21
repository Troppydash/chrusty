use shakmaty::{Board, Chess, Position};
mod engine;
mod ext;
mod param;
mod movepick;

fn round() {
    let mut game = Chess::default();

    while !game.is_game_over() {
        println!("{:?}", game.board());
        let moves = game.legal_moves();
        game.play_unchecked(moves[0]);
    }
}

fn main() {
    round();
}
