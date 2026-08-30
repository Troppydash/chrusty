use cozy_chess::{BitBoard, Board, Color, Piece, Square};

// TODO: incr update this
pub struct Threats {
    pub by_opp: [BitBoard; 6],
    pub checks: [BitBoard; 6],
}

impl Threats {
    fn get_attacks(occ: BitBoard, square: Square, piece: Piece, color: Color) -> BitBoard {
        match piece {
            Piece::Pawn => cozy_chess::get_pawn_attacks(square, color),
            Piece::Knight => cozy_chess::get_knight_moves(square),
            Piece::Bishop => cozy_chess::get_bishop_moves(square, occ),
            Piece::Rook => cozy_chess::get_rook_moves(square, occ),
            Piece::Queen => {
                cozy_chess::get_bishop_moves(square, occ) | cozy_chess::get_rook_moves(square, occ)
            }
            Piece::King => cozy_chess::get_king_moves(square),
        }
    }

    pub fn build(pos: &Board) -> Self {
        let occ = pos.occupied();
        let mut by_opp = [BitBoard::EMPTY; 6];
        for square in pos.colors(!pos.side_to_move()) {
            let piece = pos.piece_on(square).unwrap();
            by_opp[piece as usize] |= Self::get_attacks(occ, square, piece, !pos.side_to_move());
        }

        let king = pos.king(!pos.side_to_move());
        let mut checks = [
            cozy_chess::get_pawn_attacks(king, !pos.side_to_move()),
            cozy_chess::get_knight_moves(king),
            cozy_chess::get_bishop_moves(king, occ),
            cozy_chess::get_rook_moves(king, occ),
            cozy_chess::get_bishop_moves(king, occ) | cozy_chess::get_rook_moves(king, occ),
            BitBoard::EMPTY,
        ];

        Self { by_opp, checks }
    }
}
