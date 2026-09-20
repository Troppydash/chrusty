use cozy_chess::Board;

use crate::{ext::ExtBoard, stack::KeyStack};

pub fn is_rep(pos: &Board, ply: usize, stack: &KeyStack) -> bool {
    let dist = usize::min(pos.halfmove_clock() as usize, stack.head as usize);
    let mut count = 0;
    let hash = pos.correct_hash();
    for i in (4..=dist).step_by(2) {
        let x = stack.keys[stack.head - i];
        if x == hash {
            if i <= ply {
                return true;
            }

            count += 1;
            if count > 1 {
            return true;
        }
    }
    }

    return false;
}
