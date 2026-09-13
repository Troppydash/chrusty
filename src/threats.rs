use cozy_chess::{
    BitBoard, Board,
    Color::{self, White},
    File, Piece, Square,
};

const NOT_A: u64 = !File::A.bitboard().0;
const NOT_H: u64 = !File::H.bitboard().0;
const NOT_AB: u64 = !(File::A.bitboard().0 | File::B.bitboard().0);
const NOT_GH: u64 = !(File::G.bitboard().0 | File::H.bitboard().0);

/// setwise ///
pub fn setwise_pawn(pawns: BitBoard, color: Color) -> BitBoard {
    let pawns = pawns.0;
    BitBoard(if color == White {
        ((pawns & NOT_A) << 7) | ((pawns & NOT_H) << 9)
    } else {
        ((pawns & NOT_H) >> 7) | ((pawns & NOT_A) >> 9)
    })
}

pub fn setwise_knight(knights: BitBoard) -> BitBoard {
    let mut out = BitBoard::EMPTY;
    for knight in knights {
        out |= cozy_chess::get_knight_moves(knight);
    }
    out
}

pub fn setwise_rook(rooks: BitBoard, occ: BitBoard) -> BitBoard {
    let mut out = BitBoard::EMPTY;
    for rook in rooks {
        out |= cozy_chess::get_rook_moves(rook, occ);
    }
    out
}

pub fn setwise_bishop(bishops: BitBoard, occ: BitBoard) -> BitBoard {
    let mut out = BitBoard::EMPTY;
    for bishop in bishops {
        out |= cozy_chess::get_bishop_moves(bishop, occ);
    }
    out
}

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
        let ntm = !pos.side_to_move();
        let mut by_opp = [BitBoard::EMPTY; 6];
        by_opp[0] = setwise_pawn(pos.colored_pieces(ntm, Piece::Pawn), ntm);
        for square in pos.colors(ntm) & !pos.pieces(Piece::Pawn) {
            let piece = pos.piece_on(square).unwrap();
            by_opp[piece as usize] |= Self::get_attacks(occ, square, piece, ntm);
        }

        let king = pos.king(!pos.side_to_move());
        let checks = [
            BitBoard::EMPTY,
            cozy_chess::get_knight_moves(king),
            cozy_chess::get_bishop_moves(king, occ),
            cozy_chess::get_rook_moves(king, occ),
            cozy_chess::get_bishop_moves(king, occ) | cozy_chess::get_rook_moves(king, occ),
            BitBoard::EMPTY,
        ];

        Self { by_opp, checks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_setwise() {
        let fens = vec![
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/N7/PPPPPPPP/R1BQKBNR b KQkq - 1 1",
            "rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b KQkq - 1 1",
            "r3r1k1/pp3pbp/1qp1b1p1/2B5/2BP4/Q1n2N2/P4PPP/3R1K1R w - - 4 18",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
            "3k4/3p4/8/K1P4r/8/8/8/8 b - - 0 1",
            "8/8/4k3/8/2p5/8/B2P2K1/8 w - - 0 1",
            "8/8/1k6/2b5/2pP4/8/5K2/8 b - d3 0 1",
            "5k2/8/8/8/8/8/8/4K2R w K - 0 1",
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/4P3/3P1N2/PPP2PPP/RNBQKB1R b KQkq - 0 4",
            "r1b1k2r/pp1p1ppp/2n1pn2/q7/1bP1P3/2N2N2/PP1B1PPP/R2QKB1R w KQkq - 4 8",
            "r2q1rk1/pbpn1ppp/1p2pn2/3p4/1bPP4/2N1PN2/PPQB1PPP/R3KB1R w KQ - 2 9",
            "r1bqkb1r/pppp1ppp/2n5/4p3/2B1n3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 5",
            "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/2N5/PPPP1PPP/R1BQKBNR b KQkq - 1 3",
            "r1bqk2r/pppp1ppp/2n2n2/4p3/1b2P3/2NP1N2/PPP2PPP/R1BQKB1R w KQkq - 1 6",
            "r2qkb1r/ppp2ppp/2np1n2/4p3/4P1b1/2NP1N2/PPP2PPP/R1BQKB1R w KQkq - 2 6",
            "r1bq1rk1/pppn1ppp/4pn2/3p4/1bPP4/2N1PN2/PP1B1PPP/R2QKB1R w KQ - 3 7",
            "r2qk2r/pppnbppp/4pn2/3p1b2/3P4/2N1PN2/PPP1BPPP/R1BQ1RK1 w kq - 4 8",
            "r1bq1rk1/pppn1ppp/4pn2/3p4/1bPP4/2N1PN2/PP1B1PPP/R2QKB1R b KQ - 3 7",
            "r1bqkb1r/pppp1ppp/2n5/4p3/4n3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 5",
            "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/2N5/PPPP1PPP/R1BQKBNR w KQkq - 2 3",
            "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
        ];

        for fen in fens {
            let pos = Board::from_fen(fen, false).unwrap();

            // regular
            let occ = pos.occupied();
            let mut by_opp = [BitBoard::EMPTY; 6];
            for square in pos.colors(!pos.side_to_move()) {
                let piece = pos.piece_on(square).unwrap();
                by_opp[piece as usize] |= get_attacks(occ, square, piece, !pos.side_to_move());
            }

            // setwise
            let stm = !pos.side_to_move();
            let setwise_by_opp = [
                setwise_pawn(pos.colored_pieces(stm, Piece::Pawn), stm),
                setwise_knight(pos.colored_pieces(stm, Piece::Knight)),
                setwise_bishop(pos.colored_pieces(stm, Piece::Bishop), occ),
                setwise_rook(pos.colored_pieces(stm, Piece::Rook), occ),
                setwise_bishop(pos.colored_pieces(stm, Piece::Queen), occ)
                    | setwise_rook(pos.colored_pieces(stm, Piece::Queen), occ),
                cozy_chess::get_king_moves(pos.king(stm)),
            ];

            assert_eq!(by_opp, setwise_by_opp, "{}, {:#?}", fen, setwise_by_opp);
        }
    }
}
