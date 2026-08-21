use cozy_chess::{BitBoard, Color, Piece, Square};
use std::sync::OnceLock;

pub const NUM_ATTACKER_PIECES: usize = 5;
pub const NUM_TARGET_PIECES: usize = 5;

const ATTACKER_PIECES: [Piece; NUM_ATTACKER_PIECES] = [
    Piece::Pawn,
    Piece::Knight,
    Piece::Bishop,
    Piece::Rook,
    Piece::Queen,
];

fn geometric_attacks(piece: Piece, sq: Square) -> BitBoard {
    match piece {
        Piece::Pawn => {
            cozy_chess::get_pawn_attacks(sq, Color::White)
                | cozy_chess::get_pawn_attacks(sq, Color::Black)
        }
        Piece::Knight => cozy_chess::get_knight_moves(sq),
        Piece::Bishop => cozy_chess::get_bishop_moves(sq, BitBoard::EMPTY),
        Piece::Rook => cozy_chess::get_rook_moves(sq, BitBoard::EMPTY),
        Piece::Queen => {
            cozy_chess::get_bishop_moves(sq, BitBoard::EMPTY)
                | cozy_chess::get_rook_moves(sq, BitBoard::EMPTY)
        }
        Piece::King => panic!(),
    }
}

#[derive(Debug)]
pub struct ThreatLut {
    /// [piece_idx][from*64 + to] -> local index within that piece's block, or u16::MAX
    index: Box<[[u16; 4096]; NUM_ATTACKER_PIECES]>,
    offset: [usize; NUM_ATTACKER_PIECES],
    pub geo_total: usize,
}

impl ThreatLut {
    fn build() -> Self {
        let mut index = Box::new([[u16::MAX; 4096]; NUM_ATTACKER_PIECES]);
        let mut offset = [0; NUM_ATTACKER_PIECES];
        let mut running = 0;

        for (pi, &piece) in ATTACKER_PIECES.iter().enumerate() {
            offset[pi] = running;
            let mut local: u16 = 0;
            for from in Square::ALL {
                for to in geometric_attacks(piece, from) {
                    index[pi][from as usize * 64 + to as usize] = local;
                    local += 1;
                }
            }
            running += local as usize;
        }

        println!("info threat_lut {}", running * 5 * 2);

        Self {
            index,
            offset,
            geo_total: running,
        }
    }

    #[inline]
    pub fn lookup(&self, attacker_pi: usize, from: Square, to: Square) -> usize {
        let local = self.index[attacker_pi][from as usize * 64 + to as usize];
        debug_assert!(local != u16::MAX, "{} {} {}", attacker_pi, from, to);
        self.offset[attacker_pi] + local as usize
    }
}

static THREAT_LUT: OnceLock<ThreatLut> = OnceLock::new();

fn threat_lut() -> &'static ThreatLut {
    THREAT_LUT.get_or_init(ThreatLut::build)
}

#[inline]
pub fn threat_feature_index(
    attacker_pi: usize,
    from: usize,
    to: usize,
    target_piece: usize,
) -> usize {
    let geo = threat_lut().lookup(attacker_pi, Square::ALL[from], Square::ALL[to]);
    geo * NUM_TARGET_PIECES + target_piece
}
