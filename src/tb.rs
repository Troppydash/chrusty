use std::sync::{Mutex, OnceLock};

use cozy_chess::*;
use pyrrhic_rs::{EngineAdapter, TableBases};

use crate::ext::ExtBoard;

#[derive(Clone)]
struct CozyChessAdapter;

impl EngineAdapter for CozyChessAdapter {
    fn pawn_attacks(color: pyrrhic_rs::Color, sq: u64) -> u64 {
        let attacks = get_pawn_attacks(
            Square::index(sq as usize),
            if color == pyrrhic_rs::Color::Black {
                Color::Black
            } else {
                Color::White
            },
        );
        attacks.0
    }

    fn knight_attacks(sq: u64) -> u64 {
        get_knight_moves(Square::index(sq as usize)).0
    }

    fn bishop_attacks(sq: u64, occ: u64) -> u64 {
        get_bishop_moves(Square::index(sq as usize), BitBoard(occ)).0
    }

    fn rook_attacks(sq: u64, occ: u64) -> u64 {
        get_rook_moves(Square::index(sq as usize), BitBoard(occ)).0
    }

    fn king_attacks(sq: u64) -> u64 {
        get_king_moves(Square::index(sq as usize)).0
    }

    fn queen_attacks(sq: u64, occ: u64) -> u64 {
        (get_bishop_moves(Square::index(sq as usize), BitBoard(occ))
            | get_rook_moves(Square::index(sq as usize), BitBoard(occ)))
        .0
    }
}
pub struct TableBase {
    table: TableBases<CozyChessAdapter>,
}

impl TableBase {
    pub fn new(path: &str) -> Self {
        let table = pyrrhic_rs::TableBases::<CozyChessAdapter>::new(path).unwrap();
        Self { table }
    }

    #[inline]
    pub fn static_test(pos: &Board) -> bool {
        let cw = pos.castle_rights(Color::White);
        let cb = pos.castle_rights(Color::Black);
        pos.halfmove_clock() == 0
            && pos.occupied().len() <= 7
            && pos.en_passant().is_none()
            && cw == &CastleRights::EMPTY
            && cb == &CastleRights::EMPTY
    }
    pub fn test(&self, pos: &Board) -> bool {
        pos.occupied().len() <= self.table.max_pieces()
    }

    pub fn query(&self, pos: &Board) -> Option<i32> {
        debug_assert!(self.test(pos));
        let ep = match pos.ep_square() {
            None => 0,
            Some(square) => square as u32,
        };

        let result = self.table.probe_wdl(
            pos.colors(Color::White).0,
            pos.colors(Color::Black).0,
            pos.pieces(Piece::King).0,
            pos.pieces(Piece::Queen).0,
            pos.pieces(Piece::Rook).0,
            pos.pieces(Piece::Bishop).0,
            pos.pieces(Piece::Knight).0,
            pos.pieces(Piece::Pawn).0,
            ep,
            pos.side_to_move() == Color::White,
        );

        match result {
            Err(_) => None,
            Ok(value) => Some(match value {
                pyrrhic_rs::WdlProbeResult::Loss => -2,
                pyrrhic_rs::WdlProbeResult::BlessedLoss => -1,
                pyrrhic_rs::WdlProbeResult::Draw => 0,
                pyrrhic_rs::WdlProbeResult::CursedWin => 1,
                pyrrhic_rs::WdlProbeResult::Win => 2,
            }),
        }
    }
}
