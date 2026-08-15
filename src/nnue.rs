use std::arch::x86_64::*;
use std::io::Write;
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::ops::DerefMut;

/// Incremental NNUE
///
/// [Simd] encaps simd operations
/// [Network] for weights
/// [FinnyTable] for finny table storage
/// [NNUE] for incremental nnue wrapper that engine will use
use crate::ext::BitBoardExt;
use crate::ext::ColoredPiece;
use crate::ext::ExtBoard;
use crate::ext::MoveType;
use crate::param::MAX_DEPTH_USIZE;
use cozy_chess::Move;
use cozy_chess::{BitBoard, Board, Color, File, Piece, Square};

use std::mem;
use std::ptr;

pub const HL_NO_PST: usize = 2560;
pub const L1: usize = 32;
pub const L2: usize = 32;
pub const KINGS: usize = 10;
pub const OUTPUTS: usize = 8;
pub const HL: usize = HL_NO_PST + OUTPUTS;
pub const QA: i32 = 255;
pub const QB: i32 = 128;
pub const FT_SHIFT: usize = 9;
pub const SCALE: i32 = 400;

#[repr(C, align(64))]
#[derive(Debug)]
pub struct Aligned<T, const N: usize>([T; N]);

impl<T: Copy, const N: usize> Aligned<T, N> {
    fn uninit() -> Self {
        unsafe { Self(MaybeUninit::uninit().assume_init()) }
    }

    fn zeroed() -> Self {
        unsafe { Self(MaybeUninit::zeroed().assume_init()) }
    }
}

impl<T, const N: usize> Deref for Aligned<T, N> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const N: usize> DerefMut for Aligned<T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

struct SimdOps;
impl SimdOps {
    #[inline(always)]
    fn fused_copy(out: &mut Aligned<i16, HL>, in_vec: &Aligned<i16, HL>) {
        out.0.copy_from_slice(&in_vec.0);
    }

    #[inline(always)]
    fn fused_add(out: &mut Aligned<i16, HL>, add: &Aligned<i16, HL>) {
        for i in 0..HL {
            out[i] += add[i];
        }
    }

    #[inline(always)]
    fn fused_sub(out: &mut Aligned<i16, HL>, sub: &Aligned<i16, HL>) {
        for i in 0..HL {
            out[i] -= sub[i];
        }
    }

    #[inline(always)]
    fn fused_add_sub(out: &mut Aligned<i16, HL>, add: &Aligned<i16, HL>, sub: &Aligned<i16, HL>) {
        for i in 0..HL {
            out[i] += add[i] - sub[i];
        }
    }

    #[inline(always)]
    fn fused_add_sub_base(
        out: &mut Aligned<i16, HL>,
        base: &Aligned<i16, HL>,
        add: &Aligned<i16, HL>,
        sub: &Aligned<i16, HL>,
    ) {
        for i in 0..HL {
            out[i] = base[i] + add[i] - sub[i];
        }
    }

    #[inline(always)]
    fn fused_add_sub_sub_base(
        out: &mut Aligned<i16, HL>,
        base: &Aligned<i16, HL>,
        add: &Aligned<i16, HL>,
        sub1: &Aligned<i16, HL>,
        sub2: &Aligned<i16, HL>,
    ) {
        for i in 0..HL {
            out[i] = base[i] + add[i] - sub1[i] - sub2[i];
        }
    }

    #[inline(always)]
    fn fused_add_add_sub_sub_base(
        out: &mut Aligned<i16, HL>,
        base: &Aligned<i16, HL>,
        add1: &Aligned<i16, HL>,
        add2: &Aligned<i16, HL>,
        sub1: &Aligned<i16, HL>,
        sub2: &Aligned<i16, HL>,
    ) {
        for i in 0..HL {
            out[i] = base[i] + add1[i] + add2[i] - sub1[i] - sub2[i];
        }
    }
}

pub struct Permute {
    // mapping[i] = j means that move jth HL neuron to i
    mapping: [usize; HL_NO_PST],
}

impl Permute {
    const fn permute_name() -> &'static str {
        env!("PERMUTE_FILE")
    }

    pub fn save(&self) {
        println!("writing to {}", Self::permute_name());
        let mut file = std::fs::File::create(Self::permute_name()).unwrap();
        let bytes = unsafe {
            std::slice::from_raw_parts(
                self.mapping.as_ptr() as *const u8,
                size_of_val(&self.mapping),
            )
        };
        file.write_all(bytes).unwrap();
    }
    pub fn new(mapping: [usize; HL_NO_PST]) -> Self {
        Self { mapping }
    }

    pub fn default() -> Self {
        let mut mapping = [0; HL_NO_PST];
        for i in 0..HL_NO_PST {
            mapping[i] = i;
        }
        Self::new(mapping)
    }

    #[cfg(permute_file)]
    pub fn load() -> Self {
        let data = *include_bytes!(env!("PERMUTE_FILE"));
        let data = unsafe { std::mem::transmute(data) };

        Self::new(data)
    }

    #[cfg(not(permute_file))]
    pub fn load() -> Self {
        Self::default()
    }
}

#[repr(C, align(64))]
#[derive(Clone)]
struct RawNetwork {
    // default output sizing is (outputs, inputs)
    // not-transposed
    feature_weights: [[[i16; HL]; 768]; KINGS],
    feature_bias: [i16; HL],

    // transposed
    l1_weights: [[[i8; HL_NO_PST]; L1]; OUTPUTS],
    l1_bias: [[f32; L1]; OUTPUTS],

    // transposed
    l2_weights: [[[f32; L1]; L2]; OUTPUTS],
    l2_bias: [[f32; L2]; OUTPUTS],

    // transposed
    output_weights: [[f32; L2]; OUTPUTS],
    output_bias: [f32; OUTPUTS],
}

impl RawNetwork {
    fn load() -> Box<Self> {
        const DATA: &[u8] = include_bytes!(env!("EVAL_FILE"));
        if DATA.len() != mem::size_of::<RawNetwork>() {
            eprintln!("{} != {}", DATA.len(), mem::size_of::<RawNetwork>());
            eprintln!("failed to load include_bytes raw network");
            std::process::exit(1);
        }

        let mut net = Box::<Self>::new_uninit();
        unsafe {
            ptr::copy_nonoverlapping(DATA.as_ptr(), net.as_mut_ptr() as *mut u8, DATA.len());
            net.assume_init()
        }
    }

    fn get_boxed(&self) -> Box<Self> {
        unsafe {
            let mut uninit_box: Box<MaybeUninit<Self>> = Box::new_uninit();
            uninit_box
                .as_mut_ptr()
                .copy_from(self as *const RawNetwork, 1);
            uninit_box.assume_init()
        }
    }

    fn permute(&mut self, permute: &Permute) {
        let old = self.get_boxed();

        /*
           This part is a bit confusing but it actually ends up working.
           Using HL = HL_NO_PST

           We enforce that mapping[i+HL/2] = mapping[i]+HL/2 for symmetry.
           The network looks like
           stm           nstm
           [HL]          [HL]
           [HL/2]*[HL/2] [HL/2]*[HL/2]
           [HL/2]        [HL/2]

           Stm index i is computed by bias[j] + f(weights[j])*f(weights[j+HL/2])
           so we need to update feature bias and weights. Ntm indices will also
           be fixed by this.

           Since stm index i is actually the old stm index j, we update l1 i to j.
           For ntm that this is actually the same, ntm i is actually HL/2+i will
           get mapped to mapping[HL/2+i] = HL/2+j = j + HL/2 which is exactly correct.
        */

        for i in 0..HL_NO_PST {
            let j = permute.mapping[i];
            self.feature_bias[i] = old.feature_bias[j];

            for k in 0..KINGS {
                for p in 0..768 {
                    self.feature_weights[k][p][i] = old.feature_weights[k][p][j];
                }
            }

            for output in 0..OUTPUTS {
                for l1_idx in 0..L1 {
                    self.l1_weights[output][l1_idx][i] = old.l1_weights[output][l1_idx][j];
                }
            }
        }
    }
}

#[repr(C, align(64))]
struct Network {
    feature_weights: [[Aligned<i16, HL>; 768]; KINGS],
    feature_bias: Aligned<i16, HL>,

    l1_weights: [[[i8; 4 * L1]; HL_NO_PST / 4]; OUTPUTS],
    // l1_weights: [[[i8; HL]; L1]; OUTPUTS],
    l1_bias: [[f32; L1]; OUTPUTS],

    // [l2_weights] has inner component flipped
    l2_weights: [[[f32; L2]; L1]; OUTPUTS],
    l2_bias: [[f32; L2]; OUTPUTS],

    output_weights: [[f32; L2]; OUTPUTS],
    output_bias: [f32; OUTPUTS],
}

const KING_BUCKET: [usize; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0, //
    4, 4, 5, 5, 5, 5, 4, 4, //
    6, 6, 6, 6, 6, 6, 6, 6, //
    7, 7, 7, 7, 7, 7, 7, 7, //
    8, 8, 8, 8, 8, 8, 8, 8, //
    8, 8, 8, 8, 8, 8, 8, 8, //
    9, 9, 9, 9, 9, 9, 9, 9, //
    9, 9, 9, 9, 9, 9, 9, 9,
];

impl Network {
    fn load(raw: Box<RawNetwork>) -> Box<Self> {
        let mut net = unsafe { Box::<Self>::new_uninit().assume_init() };

        for a in 0..KINGS {
            for b in 0..768 {
                for c in 0..HL {
                    net.feature_weights[a][b][c] = raw.feature_weights[a][b][c];
                }
            }
        }

        for a in 0..HL {
            net.feature_bias[a] = raw.feature_bias[a];
        }

        for bucket in 0..OUTPUTS {
            for c in 0..(HL_NO_PST / 4) {
                for j in 0..L1 {
                    for k in 0..4 {
                        net.l1_weights[bucket][c][j * 4 + k] = raw.l1_weights[bucket][j][c * 4 + k];
                    }
                }
            }
        }

        for a in 0..OUTPUTS {
            for b in 0..L1 {
                net.l1_bias[a][b] = raw.l1_bias[a][b];
            }
        }

        // also transpose weights
        for i in 0..OUTPUTS {
            for j in 0..L2 {
                for k in 0..L1 {
                    net.l2_weights[i][k][j] = raw.l2_weights[i][j][k];
                }
            }
        }

        for a in 0..OUTPUTS {
            for b in 0..L2 {
                net.l2_bias[a][b] = raw.l2_bias[a][b];
            }
        }

        for a in 0..OUTPUTS {
            for b in 0..L2 {
                net.output_weights[a][b] = raw.output_weights[a][b];
            }
        }

        for a in 0..OUTPUTS {
            net.output_bias[a] = raw.output_bias[a];
        }

        net
    }

    #[inline(always)]
    fn get_king_bucket(square: Square) -> usize {
        KING_BUCKET[square as usize]
    }

    fn is_mirrored(square: Square) -> bool {
        square.file() >= File::E
    }

    fn needs_refresh(side: Color, old_king: Square, new_king: Square) -> bool {
        if old_king == new_king {
            return false;
        }

        // if different side, need refresh
        if Self::is_mirrored(old_king) != Self::is_mirrored(new_king) {
            return true;
        }

        return Self::get_king_bucket(old_king.relative_to(side))
            != Self::get_king_bucket(new_king.relative_to(side));
    }

    fn get_output_bucket(board: &Board) -> usize {
        ((board.occupied().len() - 2) / 4) as usize
    }

    fn feature_lookup(
        &self,
        king_sq: Square,
        side: Color,
        piece: ColoredPiece,
        mut square: Square,
    ) -> &Aligned<i16, HL> {
        if (king_sq as u16 & 0b100) != 0 {
            square = square.flip_file();
        }

        let index768 = (if piece.color == side { 0 } else { 6 } + piece.piece as usize) * 64
            + square.relative_to(side) as usize;
        &self.feature_weights[Self::get_king_bucket(king_sq.relative_to(side))][index768]
    }

    fn apply_update(
        &self,
        next: &mut Aligned<i16, HL>,
        base: &Aligned<i16, HL>,
        update: &Update,
        side: Color,
    ) {
        match update.update_type {
            UpdateType::Move => {
                SimdOps::fused_add_sub_base(
                    next,
                    base,
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add1.1,
                        update.add1.0,
                    ),
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub1.1,
                        update.sub1.0,
                    ),
                );
            }

            UpdateType::Capture => {
                SimdOps::fused_add_sub_sub_base(
                    next,
                    base,
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add1.1,
                        update.add1.0,
                    ),
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub1.1,
                        update.sub1.0,
                    ),
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub2.1,
                        update.sub2.0,
                    ),
                );
            }
            UpdateType::Castle => {
                SimdOps::fused_add_add_sub_sub_base(
                    next,
                    base,
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add1.1,
                        update.add1.0,
                    ),
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add2.1,
                        update.add2.0,
                    ),
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub1.1,
                        update.sub1.0,
                    ),
                    self.feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub2.1,
                        update.sub2.0,
                    ),
                );
            }
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum UpdateType {
    Move,
    Capture,
    Castle,
}

#[derive(Copy, Clone)]
struct Update {
    king_sq: [Square; 2],
    add1: (Square, ColoredPiece),
    add2: (Square, ColoredPiece),
    sub1: (Square, ColoredPiece),
    sub2: (Square, ColoredPiece),
    update_type: UpdateType,
}

#[repr(C, align(64))]
struct Accumulator {
    vals: [Aligned<i16, HL>; 2],
    is_clean: [bool; 2],
    up: Update,
}

impl Accumulator {
    fn new() -> Self {
        unsafe { mem::zeroed() }
    }
}

struct FinnyEntry {
    acc: Accumulator,
    // [side][color]
    bycolor: [[BitBoard; 2]; 2],
    // [side][piece]
    bypiece: [[BitBoard; 6]; 2],
}
impl FinnyEntry {
    fn acc_add_piece(
        &mut self,
        side: Color,
        king_sq: Square,
        piece: ColoredPiece,
        square: Square,
        network: &Network,
    ) {
        SimdOps::fused_add(
            &mut self.acc.vals[side as usize],
            network.feature_lookup(king_sq, side, piece, square),
        );
    }

    fn acc_sub_piece(
        &mut self,
        side: Color,
        king_sq: Square,
        piece: ColoredPiece,
        square: Square,
        network: &Network,
    ) {
        SimdOps::fused_sub(
            &mut self.acc.vals[side as usize],
            network.feature_lookup(king_sq, side, piece, square),
        );
    }

    fn acc_move_piece(
        &mut self,
        side: Color,
        king_sq: Square,
        piece: ColoredPiece,
        square_from: Square,
        square_to: Square,
        network: &Network,
    ) {
        SimdOps::fused_add_sub(
            &mut self.acc.vals[side as usize],
            network.feature_lookup(king_sq, side, piece, square_to),
            network.feature_lookup(king_sq, side, piece, square_from),
        );
    }
}

struct FinnyTable {
    entries: [[FinnyEntry; KINGS]; 2],
}

impl FinnyTable {
    fn new() -> Box<Self> {
        unsafe { Box::new_zeroed().assume_init() }
    }

    fn clear(&mut self) {
        self.entries = unsafe { *Box::new_zeroed().assume_init() };
    }
}

pub struct NNUE {
    network: Box<Network>,
    side: Box<[Accumulator]>,
    head: usize,
    finny: Box<FinnyTable>,
    nnz_table: [Aligned<u16, 8>; 256],
    // this is just a temp cache
    ft: Aligned<u8, HL_NO_PST>,
}

impl NNUE {
    pub fn build(permute: &Permute) -> Self {
        let mut raw = RawNetwork::load();
        raw.permute(permute);

        // nnz_table[bits][i] = ith bit in bits offset
        let mut nnz_table: [Aligned<u16, 8>; 256] = unsafe { MaybeUninit::zeroed().assume_init() };
        for i in 0..256 {
            let mut j = 0;
            let mut bits = i as u8;
            while bits > 0 {
                let lsb = bits.trailing_zeros();
                nnz_table[i][j] = lsb as u16;
                bits &= bits - 1;
                j += 1;
            }
        }

        // we don't have clone on [accumulator]
        let mut sides = vec![];
        for _ in 0..MAX_DEPTH_USIZE {
            sides.push(Accumulator::new());
        }
        let mut net = Self {
            network: Network::load(raw),
            side: sides.into_boxed_slice(),
            head: 0,
            finny: FinnyTable::new(),
            nnz_table,
            ft: Aligned::<u8, HL_NO_PST>::zeroed(),
        };

        // explicit clear to init finny
        net.clear();
        net
    }

    pub fn new() -> Self {
        Self::build(&Permute::default())
    }

    pub fn init(&mut self, board: &Board) {
        self.head = 0;
        self.side[self.head].up.king_sq[0] = board.king(Color::White);
        self.side[self.head].up.king_sq[1] = board.king(Color::Black);
        self.refresh(board, Color::White);
        self.refresh(board, Color::Black);
    }

    pub fn clear(&mut self) {
        self.finny.clear();
        for is_mirrored in 0..2 {
            for bucket in 0..KINGS {
                let entry = &mut self.finny.entries[is_mirrored][bucket];
                SimdOps::fused_copy(&mut entry.acc.vals[0], &self.network.feature_bias);
                SimdOps::fused_copy(&mut entry.acc.vals[1], &self.network.feature_bias);
            }
        }
    }

    pub fn make_move(&mut self, board: &Board, m: Move) {
        self.head += 1;
        debug_assert!(self.head < MAX_DEPTH_USIZE);

        self.side[self.head].is_clean[0] = false;
        self.side[self.head].is_clean[1] = false;

        let update = &mut self.side[self.head].up;

        // new king pos
        update.king_sq[0] = board.king(Color::White);
        update.king_sq[1] = board.king(Color::Black);
        if board.piece_on(m.from).unwrap() == Piece::King {
            if board.move_type(m) == MoveType::CASTLE {
                update.king_sq[board.side_to_move() as usize] = board.castle_to(m).0;
            } else {
                update.king_sq[board.side_to_move() as usize] = m.to;
            }
        }

        let piece_from = board.color_piece_on(m.from).unwrap();
        let piece_to = board.color_piece_on(m.to);
        update.update_type = UpdateType::Move;

        match board.move_type(m) {
            MoveType::NONE => {
                panic!("null move");
            }
            MoveType::NORMAL => {
                update.add1 = (m.to, piece_from);
                update.sub1 = (m.from, piece_from);

                if let Some(piece_to) = piece_to {
                    update.update_type = UpdateType::Capture;
                    update.sub2 = (m.to, piece_to);
                }
            }
            MoveType::ENPASSENT => {
                update.update_type = UpdateType::Capture;

                update.add1 = (m.to, piece_from);
                update.sub1 = (m.from, piece_from);

                let pawn = board.ep_capture_square().unwrap();
                update.sub2 = (pawn, board.color_piece_on(pawn).unwrap());
            }
            MoveType::PROMOTION => {
                update.add1 = (
                    m.to,
                    ColoredPiece::new(piece_from.color, m.promotion.unwrap()),
                );
                update.sub1 = (m.from, piece_from);

                if let Some(piece_to) = piece_to {
                    update.update_type = UpdateType::Capture;
                    update.sub2 = (m.to, piece_to);
                }
            }
            MoveType::CASTLE => {
                // king takes rook
                let (king_to, rook_to) = board.castle_to(m);

                update.update_type = UpdateType::Castle;

                // king
                update.add1 = (king_to, piece_from);
                update.sub1 = (m.from, piece_from);

                // rook
                update.add2 = (rook_to, piece_to.unwrap());
                update.sub2 = (m.to, piece_to.unwrap());
            }
        }
    }

    pub fn unmake_move(&mut self) {
        self.head -= 1;
    }

    fn refresh(&mut self, board: &Board, side: Color) {
        // finny table refresh
        let king_sq = board.king(side);
        let bucket = Network::get_king_bucket(king_sq.relative_to(side));
        let mirrored = Network::is_mirrored(king_sq);

        let entry = &mut self.finny.entries[mirrored as usize][bucket];

        // let full_count = board.occupied().len();
        // if full_count < 10 {
        //     let mut finny_count = 0;
        //     'outer: for color in 0..=1 {
        //         for piece in 0..6 {
        //             let old_bb =
        //                 entry.bycolor[side as usize][color] & entry.bypiece[side as usize][piece];
        //             let new_bb = board.colored_pieces(Color::ALL[color], Piece::ALL[piece]);

        //             let added = new_bb & !old_bb;
        //             let removed = old_bb & !new_bb;

        //             finny_count += added.len().max(removed.len());

        //             if full_count < finny_count {
        //                 SimdOps::fused_copy(
        //                     &mut entry.acc.vals[side as usize],
        //                     &self.network.feature_bias,
        //                 );
        //                 entry.bycolor[side as usize] = [BitBoard::EMPTY; 2];
        //                 entry.bypiece[side as usize] = [BitBoard::EMPTY; 6];
        //                 break 'outer;
        //             }
        //         }
        //     }
        // }

        for color in 0..=1 {
            for piece in 0..6 {
                let old_bb =
                    entry.bycolor[side as usize][color] & entry.bypiece[side as usize][piece];
                let new_bb = board.colored_pieces(Color::ALL[color], Piece::ALL[piece]);

                let mut added = new_bb & !old_bb;
                let mut removed = old_bb & !new_bb;

                while !added.is_empty() && !removed.is_empty() {
                    let square_to = added.pop();
                    let square_from = removed.pop();
                    entry.acc_move_piece(
                        side,
                        king_sq,
                        ColoredPiece::new(Color::ALL[color], Piece::ALL[piece]),
                        square_from,
                        square_to,
                        &self.network,
                    );
                }

                while !added.is_empty() {
                    let square_to = added.pop();
                    entry.acc_add_piece(
                        side,
                        king_sq,
                        ColoredPiece::new(Color::ALL[color], Piece::ALL[piece]),
                        square_to,
                        &self.network,
                    );
                }

                while !removed.is_empty() {
                    let square_from = removed.pop();
                    entry.acc_sub_piece(
                        side,
                        king_sq,
                        ColoredPiece::new(Color::ALL[color], Piece::ALL[piece]),
                        square_from,
                        &self.network,
                    );
                }
            }
        }

        SimdOps::fused_copy(
            &mut self.side[self.head].vals[side as usize],
            &entry.acc.vals[side as usize],
        );
        entry.bycolor[side as usize] = board.by_color();
        entry.bypiece[side as usize] = board.by_piece();
        self.side[self.head].is_clean[side as usize] = true;
    }

    pub fn catchup(&mut self, board: &Board) {
        for side in 0..=1 {
            if self.side[self.head].is_clean[side] {
                continue;
            }

            let mut base = self.head;
            loop {
                // full refresh check
                if Network::needs_refresh(
                    Color::ALL[side],
                    self.side[base].up.king_sq[side],
                    self.side[self.head].up.king_sq[side],
                ) {
                    self.refresh(board, Color::ALL[side]);
                    break;
                }

                // else check for incremental update
                if self.side[base].is_clean[side] {
                    for i in base + 1..=self.head {
                        let (base, next) = self.side.split_at_mut(i);

                        self.network.apply_update(
                            &mut next[0].vals[side],
                            &base[i - 1].vals[side],
                            &next[0].up,
                            Color::ALL[side],
                        );
                        self.side[i].is_clean[side] = true;
                    }

                    self.side[self.head].is_clean[side] = true;
                    break;
                }

                if base == 0 {
                    panic!("no clean base");
                }
                base -= 1;
            }
        }
    }

    pub fn evaluate(&mut self, board: &Board) -> i32 {
        self.catchup(board);
        debug_assert!(self.side[self.head].is_clean[0] && self.side[self.head].is_clean[1]);
        unsafe { self.avx512_evaluate(board) }
    }

    #[target_feature(enable = "avx512f")]
    pub unsafe fn avx512_evaluate(&mut self, board: &Board) -> i32 {
        let bucket = Network::get_output_bucket(board);
        let stm = board.side_to_move() as usize;

        unsafe {
            const DIVISOR: f32 = (1.0 / ((QA * QA * QB) >> FT_SHIFT) as f64) as f32;
            const ZERO: i16 = 0i16;
            const ONE: i16 = QA as i16;
            const ZEROF: f32 = 0.0f32;
            const ONEF: f32 = 1.0f32;

            // pst
            let pst = (self.side[self.head].vals[stm][HL_NO_PST + bucket]
                - self.side[self.head].vals[stm ^ 1][HL_NO_PST + bucket])
                as f32
                / (2.0 * QA as f32);

            //- ft cleanup
            for side in 0..=1 {
                let acc = &self.side[self.head].vals[stm ^ side];
                for i in 0..HL_NO_PST / 2 {
                    let x0 = acc[i].clamp(ZERO, ONE);
                    let x1 = acc[i + HL_NO_PST / 2].clamp(ZERO, ONE);
                    self.ft[side * HL_NO_PST / 2 + i] = ((x0 as u16 * x1 as u16) >> FT_SHIFT) as u8;
                }
            }

            let mut idx = Aligned::<u16, { HL_NO_PST / 4 }>::uninit();
            let mut base = _mm_setzero_si128();
            let lookup_inc = _mm_set1_epi16(8);
            let mut n = 0;
            for b in (0..HL_NO_PST).step_by(64) {
                let v = _mm512_load_si512(self.ft.as_ptr().add(b) as _);

                // skip if all 64 u8 are zero
                let non_zero_mask = _mm512_test_epi8_mask(v, v);
                if non_zero_mask == 0 {
                    base = _mm_add_epi16(base, _mm_set1_epi16(16));
                    continue;
                }

                let mask = _mm512_cmpgt_epi32_mask(v, _mm512_setzero_si512());
                for lookup in (0..16).step_by(8) {
                    let slice = ((mask >> lookup) & 0xff) as u8;
                    let indices =
                        _mm_load_si128(self.nnz_table[slice as usize].as_ptr() as *const __m128i);
                    _mm_storeu_si128(
                        idx.as_mut_ptr().add(n) as *mut __m128i,
                        _mm_add_epi16(base, indices),
                    );
                    n += slice.count_ones() as usize;
                    base = _mm_add_epi16(base, lookup_inc);
                }
            }

            const STEP: usize = 16;
            let mut l1_sum_acc = [_mm512_setzero_epi32(); L1 / STEP];
            let l1_weights = &self.network.l1_weights[bucket];
            for t in 0..n {
                let c = idx[t] as usize;
                let f = _mm512_set1_epi32(*(self.ft.as_ptr() as *const i32).add(c));
                let w = l1_weights[c].as_ptr() as *const __m512i;
                for q in (0..L1).step_by(STEP) {
                    l1_sum_acc[q / STEP] = _mm512_dpbusd_epi32(
                        l1_sum_acc[q / STEP],
                        f,
                        _mm512_load_si512(w.add(q / STEP)),
                    );
                }
            }

            let mut l1_sum = Aligned::<i32, L1>::uninit();
            for q in (0..L1).step_by(STEP) {
                _mm512_store_si512(l1_sum.as_mut_ptr().add(q) as _, l1_sum_acc[q / STEP]);
            }

            let mut l1 = Aligned::<f32, L1>::uninit();
            for i in 0..L1 {
                let s = (l1_sum[i] as f32 * DIVISOR + self.network.l1_bias[bucket][i])
                    .clamp(ZEROF, ONEF);
                l1[i] = s * s;
            }

            //- l1 -> l2
            let mut l2_sum = Aligned::<f32, L2>::zeroed();
            for i in 0..L1 {
                for j in 0..L2 {
                    l2_sum[j] += l1[i] * self.network.l2_weights[bucket][i][j];
                }
            }

            let mut l2 = Aligned::<f32, L2>::uninit();
            for i in 0..L2 {
                let s = (l2_sum[i] + self.network.l2_bias[bucket][i]).clamp(ZEROF, ONEF);
                l2[i] = s * s;
            }

            // TODO: this might be slow

            //- l2 -> output
            let mut output = self.network.output_bias[bucket];
            for i in 0..L2 {
                output += l2[i] * self.network.output_weights[bucket][i];
            }

            output += pst;
            (output * SCALE as f32) as i32
        }
    }

    pub fn sort_eval(&mut self, board: &Board) {
        let stm = board.side_to_move() as usize;
        const ZERO: i16 = 0i16;
        const ONE: i16 = QA as i16;
        for side in 0..=1 {
            let acc = &self.side[self.head].vals[stm ^ side];
            for i in 0..HL_NO_PST / 2 {
                let x0 = acc[i].clamp(ZERO, ONE);
                let x1 = acc[i + HL_NO_PST / 2].clamp(ZERO, ONE);
                self.ft[side * HL_NO_PST / 2 + i] = ((x0 as u16 * x1 as u16) >> FT_SHIFT) as u8;
            }
        }
    }

    pub fn sort_ft(&self) -> &Aligned<u8, HL_NO_PST> {
        &self.ft
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_unmake_test() {
        let mut net = NNUE::new();
        let board = Board::startpos();
        net.init(&board);
        let eval = net.evaluate(&board);

        let random_move = board.get_legal_moves()[12];
        net.make_move(&board, random_move);
        net.unmake_move();
        assert_eq!(net.evaluate(&board), eval);
    }

    #[test]
    fn symmetry_test() {
        let mut net = NNUE::new();

        let board = Board::startpos();
        net.init(&board);
        let eval = net.evaluate(&board);

        let new_board = board.null_move().unwrap();
        net.init(&new_board);
        let eval_null = net.evaluate(&new_board);

        assert_eq!(eval, eval_null);
    }

    #[test]
    fn make_unmake_catchup_test() {
        let mut net = NNUE::new();
        let board = Board::startpos();
        net.init(&board);
        let eval = net.evaluate(&board);

        net.catchup(&board);
        let random_move = board.get_legal_moves()[12];
        net.make_move(&board, random_move);

        let mut new_board = board.clone();
        new_board.play_unchecked(random_move);
        net.catchup(&new_board);

        net.unmake_move();
        net.catchup(&board);

        assert_eq!(net.evaluate(&board), eval);
    }

    #[test]
    fn random_make_unmake_catchup_test() {
        let mut net = NNUE::new();

        let sequence = vec![4, -2, 3, -2, -1, 10, -5, -2];

        let board = Board::startpos();
        net.init(&board);

        let mut evals = vec![None; 256];
        let mut boards = vec![None; 256];

        let mut sp = 0;
        evals[0] = Some(net.evaluate(&board));
        boards[0] = Some(board.clone());

        for op in sequence {
            if op > 0 {
                for _ in 0..op {
                    let board = boards[sp].clone().unwrap();
                    let moves = board.get_legal_moves();
                    let random_move = moves[1337 % moves.len()];

                    net.make_move(&board, random_move);

                    let mut next_board = board.clone();
                    next_board.play_unchecked(random_move);

                    sp += 1;
                    boards[sp] = Some(next_board.clone());
                    net.catchup(&next_board);
                    evals[sp] = Some(net.evaluate(&next_board));
                }
            } else {
                for _ in 0..-op {
                    net.unmake_move();

                    sp -= 1;

                    let board = boards[sp].clone().unwrap();
                    net.catchup(&board);
                    assert_eq!(net.evaluate(&board), evals[sp].unwrap());
                }
            }
        }
    }

    #[test]
    fn random_make_unmake_test() {
        let mut net = NNUE::new();

        let sequence = vec![4, -2, 3, -2, -1, 10, -5, -2, 20, -1, -2, -5, -10];

        let board = Board::startpos();
        net.init(&board);

        let mut evals = vec![None; 256];
        let mut boards = vec![None; 256];

        let mut sp = 0;
        evals[0] = Some(net.evaluate(&board));
        boards[0] = Some(board.clone());

        for op in sequence {
            if op > 0 {
                for _ in 0..op {
                    let board = boards[sp].clone().unwrap();
                    let moves = board.get_legal_moves();
                    let random_move = moves[1337 % moves.len()];

                    net.make_move(&board, random_move);

                    let mut next_board = board.clone();
                    next_board.play_unchecked(random_move);

                    sp += 1;
                    boards[sp] = Some(next_board.clone());
                    evals[sp] = Some(net.evaluate(&next_board));
                }
            } else {
                for _ in 0..-op {
                    net.unmake_move();

                    sp -= 1;

                    let board = boards[sp].clone().unwrap();
                    assert_eq!(net.evaluate(&board), evals[sp].unwrap());
                }
            }
        }
    }

    #[test]
    fn test_eval() {
        let mut net = NNUE::new();
        let board =
            Board::from_fen("6k1/p7/3q1nr1/3p3R/p3r3/8/7P/3Q1R1K w - - 2 52", false).unwrap();
        net.init(&board);
        let eval = net.evaluate(&board);
        assert_eq!(eval, -1532);
    }
}
