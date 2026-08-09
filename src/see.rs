use cozy_chess::{
    BitBoard, Board,
    Color::{Black, White},
    Move, Piece, Square, get_bishop_moves, get_rook_moves,
};

use crate::{
    ext::{
        BitBoardExt, ColoredPiece, ExtBoard,
        MoveType::{NORMAL, PROMOTION},
    },
    param::{pesto_value, pesto_value_opt},
};

fn to_bool(result: i32) -> bool {
    debug_assert!(result == 0 || result == 1);
    return result == 1;
}

fn get_attackers(pos: &Board, occ: BitBoard, to: Square) -> BitBoard {
    let queens = pos.pieces(Piece::Queen);

    let mut attackers = BitBoard::EMPTY;
    attackers |=
        cozy_chess::get_bishop_moves(to, occ) & (occ & (pos.pieces(Piece::Bishop) | queens));
    attackers |= cozy_chess::get_rook_moves(to, occ) & (occ & (pos.pieces(Piece::Rook) | queens));
    attackers |= cozy_chess::get_knight_moves(to) & occ & pos.pieces(Piece::Knight);
    attackers |= cozy_chess::get_king_moves(to) & occ & pos.pieces(Piece::King);

    attackers |=
        cozy_chess::get_pawn_attacks(to, Black) & occ & pos.colored_pieces(White, Piece::Pawn);
    attackers |=
        cozy_chess::get_pawn_attacks(to, White) & occ & pos.colored_pieces(Black, Piece::Pawn);

    attackers
}

pub fn see_ge(pos: &Board, m: Move, beta: i32) -> bool {
    let mt = pos.move_type(m);
    if mt != NORMAL {
        return true;
    }

    // initial capture check
    let mut swap = pesto_value_opt(&pos, pos.color_piece_on(m.to), m.to) - beta;
    if swap < 0 {
        return false;
    }

    // recapture test
    swap = pesto_value(&pos, pos.color_piece_on(m.from).unwrap(), m.from) - swap;
    if swap <= 0 {
        return true;
    }

    let queens = pos.pieces(Piece::Queen);
    let rooks = pos.pieces(Piece::Rook);
    let knights = pos.pieces(Piece::Knight);
    let bishops = pos.pieces(Piece::Bishop);
    let pawns = pos.pieces(Piece::Pawn);

    let mut stm = pos.side_to_move();
    // this always removes [to] which is potentially an enemy pinner
    let mut occ = pos.occupied() ^ m.from.bitboard() ^ m.to.bitboard();
    let mut attackers = get_attackers(pos, occ, m.to);

    let mut pinned = [BitBoard::EMPTY, BitBoard::EMPTY];
    let mut pinners = [BitBoard::EMPTY, BitBoard::EMPTY];
    let (opp_pinned, opp_pinners) = pos.opp_pinned_pinners();
    pinned[stm as usize] = pos.pinned() & pos.colors(stm);
    pinned[!stm as usize] = opp_pinned & pos.colors(!stm);
    pinners[stm as usize] = pos.pinners();
    pinners[!stm as usize] = opp_pinners;

    let mut result = 1;

    loop {
        stm = !stm;
        attackers &= occ;

        let mut stm_attackers = attackers & pos.colors(stm);
        if stm_attackers.is_empty() {
            break;
        }

        // remove pinned stm attackers
        if !(pinners[stm as usize] & occ).is_empty() {
            stm_attackers &= !pinned[stm as usize];
            if stm_attackers.is_empty() {
                break;
            }
        }

        result ^= 1;

        if let mut bb = stm_attackers & pawns
            && !bb.is_empty()
        {
            // TODO: we can also do occ masking
            swap = pesto_value(&pos, ColoredPiece::new(stm, Piece::Pawn), m.to) - swap;
            if swap < result {
                break;
            }

            occ ^= bb.pop().bitboard();
            attackers |= get_bishop_moves(m.to, occ) & (bishops | queens);
        } else if let mut bb = stm_attackers & knights
            && !bb.is_empty()
        {
            swap = pesto_value(&pos, ColoredPiece::new(stm, Piece::Knight), m.to) - swap;
            if swap < result {
                break;
            }

            occ ^= bb.pop().bitboard();
        } else if let mut bb = stm_attackers & bishops
            && !bb.is_empty()
        {
            swap = pesto_value(&pos, ColoredPiece::new(stm, Piece::Bishop), m.to) - swap;
            if swap < result {
                break;
            }

            occ ^= bb.pop().bitboard();
            attackers |= get_bishop_moves(m.to, occ) & (bishops | queens);
        } else if let mut bb = stm_attackers & rooks
            && !bb.is_empty()
        {
            swap = pesto_value(&pos, ColoredPiece::new(stm, Piece::Rook), m.to) - swap;
            if swap < result {
                break;
            }

            occ ^= bb.pop().bitboard();
            attackers |= get_rook_moves(m.to, occ) & (rooks | queens);
        } else if let bb = stm_attackers & queens
            && !bb.is_empty()
        {
            swap = pesto_value(&pos, ColoredPiece::new(stm, Piece::Queen), m.to) - swap;
            if swap < result {
                break;
            }

            occ ^= bb.next_square().unwrap().bitboard();
            attackers |= get_bishop_moves(m.to, occ) & (bishops | queens);
            attackers |= get_rook_moves(m.to, occ) & (rooks | queens);
        } else {
            // king
            if (attackers & pos.colors(!stm)).is_empty() {
                return to_bool(result);
            } else {
                return !to_bool(result);
            }
        }
    }

    return to_bool(result);
}
