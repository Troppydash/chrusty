use arrayvec::ArrayVec;
use cozy_chess::{
    BitBoard, Board,
    Color::{self, Black, White},
    File, Move,
    Piece::{self, King, Pawn, Queen},
    Rank, Square,
};

use crate::ext::MoveType::{CASTLE, ENPASSENT, NONE, NORMAL, PROMOTION};

// these are stack allocated

pub const MAX_MOVES: usize = 218;
pub type ScoredMoveList = ArrayVec<ScoredMove, MAX_MOVES>;
pub type MoveList = ArrayVec<Move, MAX_MOVES>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColoredPiece {
    pub color: Color,
    pub piece: Piece,
}

impl ColoredPiece {
    pub fn new(color: Color, piece: Piece) -> Self {
        Self { color, piece }
    }

    pub fn index(&self) -> usize {
        self.color as usize * 6 + self.piece as usize
    }
}

pub fn index_with_option(c: &Option<ColoredPiece>) -> usize {
    match c {
        None => 12,
        Some(c) => c.index(),
    }
}

pub trait BitBoardExt {
    fn pop(&mut self) -> Square;
}

impl BitBoardExt for BitBoard {
    fn pop(&mut self) -> Square {
        assert!(self.0 != 0);
        let index = self.0.trailing_zeros();
        self.0 ^= 1u64 << index;
        Square::ALL[index as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MoveType {
    NORMAL,
    ENPASSENT,
    CASTLE,
    PROMOTION,
    NONE,
}

impl Default for MoveType {
    fn default() -> Self {
        MoveType::NONE
    }
}

pub trait ExtMove {
    const NULL_MOVE: Move;
    const NULL_MOVE_BITS: u16;

    fn is_null(&self) -> bool;
    fn to_uci(&self, board: &Board) -> String;
    fn from_uci(uci: &str, board: &Board) -> Move;

    fn to_bits(&self) -> u16;
    fn from_bits(value: u16) -> Move;

    fn is_under_promotion(&self) -> bool;
}

pub const fn move_to_bits(m: Move) -> u16 {
    let from = m.from as u16;
    let to = m.to as u16;
    let promotion = match m.promotion {
        None => 6,
        Some(piece) => piece as u16,
    };

    // 6 bits + 6 bits + 3 bits = 15 bits
    from | (to << 6) | (promotion << 12)
}

pub const fn move_is_null(m: Move) -> bool {
    move_to_bits(m) == Move::NULL_MOVE_BITS
}

impl ExtMove for Move {
    const NULL_MOVE: Move = Move {
        from: cozy_chess::Square::A1,
        to: cozy_chess::Square::A1,
        promotion: None,
    };
    const NULL_MOVE_BITS: u16 = 6 << 12;

    fn is_null(&self) -> bool {
        *self == Self::NULL_MOVE
    }

    fn to_uci(&self, board: &Board) -> String {
        format!("{}", cozy_chess::util::display_uci_move(board, *self))
    }

    fn from_uci(uci: &str, board: &Board) -> Move {
        cozy_chess::util::parse_uci_move(board, uci).unwrap()
    }

    fn to_bits(&self) -> u16 {
        move_to_bits(*self)
    }

    fn from_bits(value: u16) -> Move {
        let from = value & (0b111111);
        let to = (value >> 6) & (0b111111);
        let promotion = value >> 12;

        Move {
            from: Square::ALL[from as usize],
            to: Square::ALL[to as usize],
            promotion: match promotion {
                6 => None,
                _ => Some(Piece::ALL[promotion as usize]),
            },
        }
    }
    
    fn is_under_promotion(&self) -> bool {
        match self.promotion {
            None => false,
            Some(Piece::Queen) => false,
            Some(Piece::Knight) => false,
            _ => true
        }
    }
}

pub trait ExtBoard {
    fn in_check(&self) -> bool;
    fn any_moves(&self) -> bool;

    fn get_captured(&self, m: Move) -> Piece;
    fn is_quiet(&self, m: Move) -> bool;
    fn ep_square(&self) -> Option<Square>;
    fn ep_capture_square(&self) -> Option<Square>;
    fn is_ep(&self, m: Move) -> bool;
    fn is_castle(&self, m: Move) -> bool;

    fn get_legal_moves(&self) -> MoveList;

    fn move_type(&self, m: Move) -> MoveType;

    /// [piece_on] but None is 6
    fn piece_on_index(&self, sq: Square) -> usize;

    fn has_insufficient_material(&self) -> bool;

    fn has_non_pawns(&self, side: Color) -> bool;

    fn correct_hash(&self) -> u64;

    fn by_color(&self) -> [BitBoard; 2];
    fn by_piece(&self) -> [BitBoard; 6];

    fn color_piece_on(&self, square: Square) -> Option<ColoredPiece>;
    fn castle_to(&self, m: Move) -> (Square, Square);

    fn opp_pinned_checkers(&self) -> (BitBoard, BitBoard);
    fn opp_pinned_pinners(&self) -> (BitBoard, BitBoard);
    fn pinners(&self) -> BitBoard;
}

impl ExtBoard for Board {
    fn in_check(&self) -> bool {
        !self.checkers().is_empty()
    }

    fn any_moves(&self) -> bool {
        self.generate_moves(|_m| true)
    }

    fn get_captured(&self, m: Move) -> Piece {
        // queen promotions treated as pawn capture
        match self.piece_on(m.to) {
            Some(piece) => piece,
            None => {
                debug_assert!(m.promotion == Some(Queen) || self.is_ep(m));
                Piece::Pawn
            }
        }
    }

    fn get_legal_moves(&self) -> MoveList {
        let mut ml = MoveList::new();
        self.generate_moves(|moves| {
            for m in moves {
                ml.push(m)
            }
            false
        });

        ml
    }

    fn is_quiet(&self, m: Move) -> bool {
        // special moves are CASTLE, PROMOTION, ENPASSENT
        if self.is_ep(m) {
            return false;
        }

        if self.is_castle(m) {
            return true;
        }

        // a quiet move is not a capture and not a queen promotion
        self.piece_on(m.to).is_none() && m.promotion != Some(Piece::Queen)
    }

    /// This is not the [ep_capture_square]
    #[inline]
    fn ep_square(&self) -> Option<Square> {
        match self.en_passant() {
            Some(file) => {
                let ep_rank = match self.side_to_move() {
                    White => Rank::Sixth,
                    Black => Rank::Third,
                };
                Some(Square::new(file, ep_rank))
            }
            None => None,
        }
    }

    fn ep_capture_square(&self) -> Option<Square> {
        match self.en_passant() {
            Some(file) => {
                let ep_rank = match self.side_to_move() {
                    White => Rank::Fifth,
                    Black => Rank::Fourth,
                };
                Some(Square::new(file, ep_rank))
            }
            None => None,
        }
    }

    #[inline]
    fn is_ep(&self, m: Move) -> bool {
        self.piece_on(m.from) == Some(Piece::Pawn) && self.ep_square() == Some(m.to)
    }

    #[inline]
    fn is_castle(&self, m: Move) -> bool {
        self.piece_on(m.to) == Some(Piece::Rook) && self.color_on(m.to) == Some(self.side_to_move())
    }

    fn move_type(&self, m: Move) -> MoveType {
        if m.is_null() {
            return NONE;
        }

        if m.promotion.is_some() {
            return PROMOTION;
        }

        if self.is_ep(m) {
            return ENPASSENT;
        }

        if self.is_castle(m) {
            return CASTLE;
        }

        return NORMAL;
    }

    fn piece_on_index(&self, sq: Square) -> usize {
        match self.piece_on(sq) {
            None => 6,
            Some(piece) => piece as usize,
        }
    }

    fn has_insufficient_material(&self) -> bool {
        let count = self.occupied().len();

        // only kings
        if count == 2 {
            return true;
        }

        // only bishop/knight
        if count == 3 {
            if !self.pieces(Piece::Bishop).is_empty() || !self.pieces(Piece::Knight).is_empty() {
                return true;
            }
        }

        if count == 4 {
            let mut white_bishops = self.colored_pieces(Color::White, Piece::Bishop);
            let mut black_bishops = self.colored_pieces(Color::Black, Piece::Bishop);
            // same colored bishops
            if !white_bishops.is_empty() && !black_bishops.is_empty() {
                let wb = white_bishops.next_square().unwrap();
                let bb = black_bishops.next_square().unwrap();
                if Square::same_color(&wb, &bb) {
                    return true;
                }
            }

            // one side with same color bishops
            if white_bishops.len() == 2 {
                let sq1 = white_bishops.next_square().unwrap();
                white_bishops ^= sq1.bitboard();
                let sq2 = white_bishops.next_square().unwrap();
                if Square::same_color(&sq1, &sq2) {
                    return true;
                }
            } else if black_bishops.len() == 2 {
                let sq1 = black_bishops.next_square().unwrap();
                black_bishops ^= sq1.bitboard();
                let sq2 = black_bishops.next_square().unwrap();
                if Square::same_color(&sq1, &sq2) {
                    return true;
                }
            }
        }

        return false;
    }

    fn has_non_pawns(&self, side: Color) -> bool {
        self.colors(side) != (self.colored_pieces(side, King) | self.colored_pieces(side, Pawn))
    }

    fn correct_hash(&self) -> u64 {
        // check if ep sq is legal
        match self.ep_square() {
            None => self.hash(),
            Some(ep_square) => {
                let origins = cozy_chess::get_pawn_attacks(ep_square, !self.side_to_move());
                if (self.colored_pieces(self.side_to_move(), Piece::Pawn) & origins).is_empty() {
                    // remove ep hash
                    self.hash_without_ep()
                } else {
                    self.hash()
                }
            }
        }
    }

    fn by_color(&self) -> [BitBoard; 2] {
        [self.colors(Color::White), self.colors(Color::Black)]
    }

    fn by_piece(&self) -> [BitBoard; 6] {
        [
            self.pieces(Piece::Pawn),
            self.pieces(Piece::Knight),
            self.pieces(Piece::Bishop),
            self.pieces(Piece::Rook),
            self.pieces(Piece::Queen),
            self.pieces(Piece::King),
        ]
    }

    fn color_piece_on(&self, square: Square) -> Option<ColoredPiece> {
        self.color_on(square)
            .zip(self.piece_on(square))
            .map(|(color, piece)| ColoredPiece::new(color, piece))
    }

    /// (king, rook)
    #[inline]
    fn castle_to(&self, m: Move) -> (Square, Square) {
        debug_assert!(self.is_castle(m));

        if m.to > m.from {
            // short
            (
                Square::new(File::G, m.from.rank()),
                Square::new(File::F, m.from.rank()),
            )
        } else {
            // long
            (
                Square::new(File::C, m.from.rank()),
                Square::new(File::D, m.from.rank()),
            )
        }
    }

    fn opp_pinned_checkers(&self) -> (BitBoard, BitBoard) {
        let mut pinned = BitBoard::EMPTY;
        let mut checkers = BitBoard::EMPTY;
        let color = !self.side_to_move();
        let our_king = self.king(color);
        let their_attackers = self.colors(!color)
            & ((cozy_chess::get_bishop_rays(our_king)
                & (self.pieces(Piece::Bishop) | self.pieces(Piece::Queen)))
                | (cozy_chess::get_rook_rays(our_king)
                    & (self.pieces(Piece::Rook) | self.pieces(Piece::Queen))));

        for square in their_attackers {
            let between = cozy_chess::get_between_rays(square, our_king) & self.occupied();
            if between.len() == 0 {
                checkers |= square.bitboard();
            } else if between.len() == 1 {
                pinned |= between;
            }
        }

        return (pinned, checkers);
    }

    fn opp_pinned_pinners(&self) -> (BitBoard, BitBoard) {
        // TODO: cache this
        let mut pinned = BitBoard::EMPTY;
        let mut pinners = BitBoard::EMPTY;
        let color = !self.side_to_move();
        let our_king = self.king(color);
        let their_attackers = self.colors(!color)
            & ((cozy_chess::get_bishop_rays(our_king)
                & (self.pieces(Piece::Bishop) | self.pieces(Piece::Queen)))
                | (cozy_chess::get_rook_rays(our_king)
                    & (self.pieces(Piece::Rook) | self.pieces(Piece::Queen))));

        for square in their_attackers {
            let between = cozy_chess::get_between_rays(square, our_king) & self.occupied();
            if between.len() == 1 {
                pinned |= between;
                pinners |= BitBoard::from(square);
            }
        }

        return (pinned, pinners);
    }

    fn pinners(&self) -> BitBoard {
        let mut pinners = BitBoard::EMPTY;
        let color = self.side_to_move();
        let our_king = self.king(color);
        let their_attackers = self.colors(!color)
            & ((cozy_chess::get_bishop_rays(our_king)
                & (self.pieces(Piece::Bishop) | self.pieces(Piece::Queen)))
                | (cozy_chess::get_rook_rays(our_king)
                    & (self.pieces(Piece::Rook) | self.pieces(Piece::Queen))));

        for square in their_attackers {
            let between = cozy_chess::get_between_rays(square, our_king) & self.occupied();
            if between.len() == 1 {
                pinners |= BitBoard::from(square);
            }
        }

        return pinners;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScoredMove {
    pub inner: Move,
    pub score: i32,
}

impl ScoredMove {
    pub const NULL_MOVE: ScoredMove = ScoredMove {
        inner: Move::NULL_MOVE,
        score: 0,
    };

    pub fn new(inner: Move, score: i32) -> Self {
        Self { inner, score }
    }

    pub fn from_move(inner: Move) -> Self {
        Self::new(inner, 0)
    }

    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    pub fn get_score(&self) -> i32 {
        return self.score;
    }
}

pub trait ExtSquare {
    fn same_color(&self, other: &Square) -> bool;
}

impl ExtSquare for Square {
    fn same_color(&self, other: &Square) -> bool {
        // magic
        ((9 * (*self as i32 ^ *other as i32)) & 8) == 0
    }
}

//- COPIED -//
#[derive(Debug)]
pub struct ColorZobristConstants {
    pub pieces: [[u64; Square::NUM]; Piece::NUM],
    pub castle_rights: [u64; File::NUM],
}

#[derive(Debug)]
pub struct ZobristConstants {
    pub color: [ColorZobristConstants; Color::NUM],
    pub en_passant: [u64; File::NUM],
    pub black_to_move: u64,
}

pub const ZOBRIST: ZobristConstants = {
    // Simple Pcg64Mcg impl
    let mut state = 0x7369787465656E2062797465206E756Du128 | 1;
    macro_rules! rand {
        () => {{
            state = state.wrapping_mul(0x2360ED051FC65DA44385DF649FCCF645);
            let rot = (state >> 122) as u32;
            let xsl = (state >> 64) as u64 ^ state as u64;
            xsl.rotate_right(rot)
        }};
    }

    macro_rules! fill_array {
        ($array:ident: $expr:expr) => {{
            let mut i = 0;
            while i < $array.len() {
                $array[i] = $expr;
                i += 1;
            }
        }};
    }

    macro_rules! color_zobrist_constant {
        () => {{
            let mut castle_rights = [0; File::NUM];
            fill_array!(castle_rights: rand!());

            let mut pieces = [[0; Square::NUM]; Piece::NUM];
            fill_array!(pieces: {
                let mut squares = [0; Square::NUM];
                fill_array!(squares: rand!());
                squares
            });

            ColorZobristConstants {
                pieces,
                castle_rights
            }
        }};
    }

    let mut en_passant = [0; File::NUM];
    fill_array!(en_passant: rand!());

    let white = color_zobrist_constant!();
    let black = color_zobrist_constant!();

    let black_to_move = rand!();

    ZobristConstants {
        color: [white, black],
        en_passant,
        black_to_move,
    }
};

pub fn zobrist_pst(color: Color, piece: Piece, sq: Square) -> u64 {
    ZOBRIST.color[color as usize].pieces[piece as usize][sq as usize]
}
