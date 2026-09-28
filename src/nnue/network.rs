use std::arch::x86_64::*;
use std::io::Write;
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::ext::ColoredPiece;
use crate::nnue::ti;
use crate::nnue::ti::FULL_THREATS;
use crate::nnue::ti::MAX_TI_INPUTS;
use crate::nnue::ti::PINNED;
use crate::nnue::ti::TI_INPUTS;
use crate::nnue::update::PinnedDelta;
use crate::nnue::update::ThreatDelta;
use crate::nnue::update::ThreatDeltaGeneral;
use crate::nnue::update::ThreatUpdate;
use crate::nnue::update::Update;
use crate::nnue::update::UpdateType;
use arrayvec::ArrayVec;
use cozy_chess::{Board, Color, File, Piece, Square};
use std::mem;
use std::ptr;

pub const HL: usize = 1024;
pub const L1: usize = 32;
pub const L2: usize = 32;
pub const OUTPUTS: usize = 8;
pub const QA: i32 = 255;
pub const QB: i32 = 128;
pub const QPST: i32 = 128 * 128;
pub const FT_SHIFT: usize = 8;
pub const SCALE: i32 = 600;
pub const CM: usize = 64;

const HALF_KING_BUCKET: [usize; 32] = [
    0, 1, 2, 3, //
    4, 5, 6, 7, //
    8, 8, 9, 9, //
    10, 10, 11, 11, //
    12, 12, 13, 13, //
    12, 12, 13, 13, //
    14, 14, 15, 15, //
    14, 14, 15, 15, //
];

const KING_BUCKET: [usize; 64] = {
    let mut table = [0; 64];
    let mut sq = 0;

    while sq < 64 {
        let rank = sq / 8;
        let file = sq % 8;

        let mirrored_file = if file < 4 { file } else { 7 - file };

        let index_32 = rank * 4 + mirrored_file;
        table[sq] = HALF_KING_BUCKET[index_32];

        sq += 1;
    }

    table
};

pub const KINGS: usize = {
    let mut i = 0;
    let mut m = 0;
    while i < 64 {
        if KING_BUCKET[i] > m {
            m = KING_BUCKET[i];
        }
        i += 1;
    }
    m + 1
};

const OUTPUT_BUCKETS: [usize; 33] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, //
    1, 1, 1, 1, //
    2, 2, 2, 2, //
    3, 3, 3, //
    4, 4, 4, //
    5, 5, 5, //
    6, 6, 6, //
    7, 7, 7, 7, //
];

#[repr(C, align(64))]
#[derive(Debug, Clone)]
pub struct Aligned<T, const N: usize>(pub [T; N]);

impl<T: Copy, const N: usize> Aligned<T, N> {
    pub fn uninit() -> Self {
        unsafe { Self(MaybeUninit::uninit().assume_init()) }
    }

    pub fn zeroed() -> Self {
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

#[repr(C, align(16))]
#[derive(Copy, Clone)]
pub struct Aligned16(pub [u16; 8]);

impl Deref for Aligned16 {
    type Target = [u16];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Aligned16 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct SimdOps;
impl SimdOps {
    #[inline(always)]
    pub fn fused_copy(out: &mut Aligned<i16, HL>, in_vec: &Aligned<i16, HL>) {
        out.0.copy_from_slice(&in_vec.0);
    }

    #[inline(always)]
    pub fn fused_copy32(out: &mut [i32; OUTPUTS], in_vec: &[i16; OUTPUTS]) {
        for i in 0..OUTPUTS {
            out[i] = in_vec[i] as i32;
        }
    }

    #[inline(always)]
    pub fn fused_add(out: &mut Aligned<i16, HL>, add: &Aligned<i16, HL>) {
        for i in 0..HL {
            out[i] += add[i];
        }
    }

    #[inline(always)]
    pub fn fused_add32(out: &mut [i32; OUTPUTS], add: &[i16; OUTPUTS]) {
        for i in 0..OUTPUTS {
            out[i] += add[i] as i32;
        }
    }

    #[inline(always)]
    pub fn fused_sub(out: &mut Aligned<i16, HL>, sub: &Aligned<i16, HL>) {
        for i in 0..HL {
            out[i] -= sub[i];
        }
    }

    #[inline(always)]
    pub fn fused_sub32(out: &mut [i32; OUTPUTS], sub: &[i16; OUTPUTS]) {
        for i in 0..OUTPUTS {
            out[i] -= sub[i] as i32;
        }
    }

    #[inline(always)]
    pub fn fused_add_sub(
        out: &mut Aligned<i16, HL>,
        add: &Aligned<i16, HL>,
        sub: &Aligned<i16, HL>,
    ) {
        for i in 0..HL {
            out[i] += add[i] - sub[i];
        }
    }

    #[inline(always)]
    pub fn fused_add_sub32(out: &mut [i32; OUTPUTS], add: &[i16; OUTPUTS], sub: &[i16; OUTPUTS]) {
        for i in 0..OUTPUTS {
            out[i] += add[i] as i32 - sub[i] as i32;
        }
    }

    #[inline(always)]
    pub fn fused_add_sub_base(
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
    pub fn fused_add_sub_base32(
        out: &mut [i32; OUTPUTS],
        base: &[i32; OUTPUTS],
        add: &[i16; OUTPUTS],
        sub: &[i16; OUTPUTS],
    ) {
        for i in 0..OUTPUTS {
            out[i] = base[i] + add[i] as i32 - sub[i] as i32;
        }
    }

    #[inline(always)]
    pub fn fused_add_sub_sub_base(
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
    pub fn fused_add_sub_sub_base32(
        out: &mut [i32; OUTPUTS],
        base: &[i32; OUTPUTS],
        add: &[i16; OUTPUTS],
        sub1: &[i16; OUTPUTS],
        sub2: &[i16; OUTPUTS],
    ) {
        for i in 0..OUTPUTS {
            out[i] = base[i] + add[i] as i32 - sub1[i] as i32 - sub2[i] as i32;
        }
    }

    #[inline(always)]
    pub fn fused_add_add_sub_sub_base(
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

    #[inline(always)]
    pub fn fused_add_add_sub_sub_base32(
        out: &mut [i32; OUTPUTS],
        base: &[i32; OUTPUTS],
        add1: &[i16; OUTPUTS],
        add2: &[i16; OUTPUTS],
        sub1: &[i16; OUTPUTS],
        sub2: &[i16; OUTPUTS],
    ) {
        for i in 0..OUTPUTS {
            out[i] = base[i] + add1[i] as i32 + add2[i] as i32 - sub1[i] as i32 - sub2[i] as i32;
        }
    }

    pub fn silu(x: __m512) -> __m512 {
        unsafe {
            let zero = _mm512_setzero_ps();
            let one = _mm512_set1_ps(1.0);
            let two = _mm512_set1_ps(2.0);

            let clamped_x = _mm512_max_ps(
                _mm512_set1_ps(-15.0),
                _mm512_min_ps(_mm512_set1_ps(15.0), x),
            );
            let neg_x = _mm512_sub_ps(zero, clamped_x);

            // goal is to compute sigmoid(x) = 1/D(x) where D(x) = 1+exp(-x)

            // compute exp2(z) = exp2(-x log2(e)), but we split -x log2(e) into dn + r
            let log2e = _mm512_set1_ps(1.44269504);
            let z = _mm512_mul_ps(neg_x, log2e);

            let dn = _mm512_roundscale_ps::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(z);
            let r = _mm512_sub_ps(z, dn);

            // compute 2^r, where abs(r) < 1/2
            let p = _mm512_fmadd_ps(_mm512_set1_ps(0.0555041), r, _mm512_set1_ps(0.2402265));
            let p = _mm512_fmadd_ps(p, r, _mm512_set1_ps(0.69314718));
            let p = _mm512_fmadd_ps(p, r, one);

            // 2^dn by exp, note the cast from i32 to f32 is a type cast
            let i = _mm512_cvtps_epi32(dn);
            let i = _mm512_add_epi32(i, _mm512_set1_epi32(127));
            let i = _mm512_slli_epi32(i, 23);
            let exp_neg_x = _mm512_mul_ps(p, _mm512_castsi512_ps(i));

            let d = _mm512_add_ps(one, exp_neg_x);
            let rcp = _mm512_rcp14_ps(d);

            // newton
            // let rcp = _mm512_mul_ps(rcp, _mm512_fnmadd_ps(d, rcp, two));

            return _mm512_mul_ps(x, rcp);
        }
    }
}

pub struct Permute {
    // mapping[i] = j means that move jth HL neuron to i
    pub mapping: [usize; HL],
}

impl Permute {
    pub fn save(&self) {
        println!("writing to {}", env!("PERMUTE_FILE_NEXT"));
        let mut file = std::fs::File::create(env!("PERMUTE_FILE_NEXT")).unwrap();
        let bytes = unsafe {
            std::slice::from_raw_parts(
                self.mapping.as_ptr() as *const u8,
                size_of_val(&self.mapping),
            )
        };
        file.write_all(bytes).unwrap();
    }

    pub fn new(mut mapping: [usize; HL]) -> Self {
        for i in 0..(HL / 2) {
            mapping[i + HL / 2] = mapping[i] + HL / 2;
        }
        Self { mapping }
    }

    pub fn default() -> Self {
        let mut mapping = [0; HL];
        for i in 0..HL {
            mapping[i] = i;
        }
        Self::new(mapping)
    }

    #[cfg(permute_file)]
    pub fn load() -> Self {
        let data = *include_bytes!(env!("PERMUTE_FILE_SRC"));
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
pub struct RawNetwork {
    // default output sizing is (outputs, inputs)
    // not-transposed
    pst_weights: [[[i16; OUTPUTS]; 768]; KINGS],
    pst_bias: [i16; OUTPUTS],

    feature_weights: [[[i16; HL]; 768]; KINGS],
    threat_weights: [[i8; HL]; TI_INPUTS],
    feature_bias: [i16; HL],

    // transposed
    l1_weights: [[[i8; HL]; L1]; OUTPUTS],
    l1_bias: [[f32; L1]; OUTPUTS],

    // transposed
    l2_weights: [[[f32; L1]; L2]; OUTPUTS],
    l2_bias: [[f32; L2]; OUTPUTS],

    // transposed
    output_weights: [[f32; L2]; OUTPUTS],
    output_bias: [f32; OUTPUTS],
}

impl RawNetwork {
    pub fn load() -> Box<Self> {
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

    pub fn permute(&mut self, permute: &Permute) {
        let old = self.get_boxed();

        /*
           This part is a bit confusing but it actually ends up working.
           Using HL = HL

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

        for i in 0..HL {
            let j = permute.mapping[i];
            self.feature_bias[i] = old.feature_bias[j];

            for k in 0..KINGS {
                for p in 0..768 {
                    self.feature_weights[k][p][i] = old.feature_weights[k][p][j];
                }
            }

            for k in 0..TI_INPUTS {
                self.threat_weights[k][i] = old.threat_weights[k][j];
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
pub struct Network {
    pub pst_weights: [[[i16; OUTPUTS]; 768]; KINGS],
    pub pst_bias: [i16; OUTPUTS],

    pub feature_weights: [[Aligned<i16, HL>; 768]; KINGS],
    pub threat_weights: [Aligned<i8, HL>; TI_INPUTS],
    pub feature_bias: Aligned<i16, HL>,

    pub l1_weights: [[Aligned<i8, { 4 * L1 }>; HL / 4]; OUTPUTS],
    pub l1_bias: [Aligned<f32, L1>; OUTPUTS],

    // [l2_weights] has inner component flipped
    pub l2_weights: [[Aligned<f32, L2>; L1]; OUTPUTS],
    pub l2_bias: [Aligned<f32, L2>; OUTPUTS],

    pub output_weights: [Aligned<f32, L2>; OUTPUTS],
    pub output_bias: [f32; OUTPUTS],
}

impl Network {
    pub fn load(raw: Box<RawNetwork>) -> Box<Self> {
        let mut net = unsafe { Box::<Self>::new_uninit().assume_init() };

        for a in 0..KINGS {
            for b in 0..768 {
                for c in 0..OUTPUTS {
                    net.pst_weights[a][b][c] = raw.pst_weights[a][b][c];
                }
            }
        }

        for a in 0..OUTPUTS {
            net.pst_bias[a] = raw.pst_bias[a];
        }

        for a in 0..KINGS {
            for b in 0..768 {
                for c in 0..HL {
                    net.feature_weights[a][b][c] = raw.feature_weights[a][b][c];
                }
            }
        }

        for a in 0..TI_INPUTS {
            for b in 0..HL {
                net.threat_weights[a][b] = raw.threat_weights[a][b];
            }
        }

        for a in 0..HL {
            net.feature_bias[a] = raw.feature_bias[a];
        }

        for bucket in 0..OUTPUTS {
            for c in 0..(HL / 4) {
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
    pub fn get_king_bucket(square: Square) -> usize {
        KING_BUCKET[square as usize]
    }

    pub fn is_mirrored(square: Square) -> bool {
        square.file() >= File::E
    }

    pub fn needs_refresh(side: Color, old_king: Square, new_king: Square) -> bool {
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

    pub fn needs_refresh_threat(old_king: Square, new_king: Square) -> bool {
        if old_king == new_king {
            return false;
        }

        // if different side, need refresh
        Self::is_mirrored(old_king) != Self::is_mirrored(new_king)
    }

    pub fn get_output_bucket(board: &Board) -> usize {
        OUTPUT_BUCKETS[board.occupied().len() as usize]
    }

    pub fn feature_lookup(
        &self,
        king_sq: Square,
        side: Color,
        piece: ColoredPiece,
        mut square: Square,
    ) -> &Aligned<i16, HL> {
        if (king_sq as u16 & 0b100) != 0 {
            square = square.flip_file();
        }

        let index768 = ((if piece.color == side { 0 } else { 6 }) + piece.piece as usize) * 64
            + square.relative_to(side) as usize;
        &self.feature_weights[Self::get_king_bucket(king_sq.relative_to(side))][index768]
    }

    pub fn pst_feature_lookup(
        &self,
        king_sq: Square,
        side: Color,
        piece: ColoredPiece,
        mut square: Square,
    ) -> &[i16; OUTPUTS] {
        if (king_sq as u16 & 0b100) != 0 {
            square = square.flip_file();
        }

        let index768 = ((if piece.color == side { 0 } else { 6 }) + piece.piece as usize) * 64
            + square.relative_to(side) as usize;
        &self.pst_weights[Self::get_king_bucket(king_sq.relative_to(side))][index768]
    }

    fn threat_feature_lookup_index_from_threat(
        &self,
        king_sq: Square,
        side: Color,
        delta: &ThreatDelta,
    ) -> i32 {
        self.threat_feature_lookup_index(
            king_sq,
            side,
            delta.attacker.color,
            delta.target.color,
            delta.attacker.piece,
            delta.attacker_sq,
            delta.target.piece,
            delta.target_sq,
        )
    }

    pub fn threat_feature_lookup_index(
        &self,
        king_sq: Square,
        side: Color,
        attacker_color: Color,
        target_color: Color,
        attacker: Piece,
        mut attacker_square: Square,
        target: Piece,
        mut target_square: Square,
    ) -> i32 {
        if !ti::is_feature(attacker, target) {
            return -1;
        }

        debug_assert!(attacker != Piece::King);
        debug_assert!(target != Piece::King);
        if (king_sq as u16 & 0b100) != 0 {
            attacker_square = attacker_square.flip_file();
            target_square = target_square.flip_file();
        }

        ti::threat_feature_index(
            side == attacker_color,
            attacker_color != target_color,
            attacker as usize,
            attacker_square.relative_to(side) as usize,
            target_square.relative_to(side) as usize,
            target as usize,
        ) as i32
    }

    pub fn threat_pinned_index(
        &self,
        king_sq: Square,
        side: Color,
        side_pinned: bool,
        piece: ColoredPiece,
        mut square: Square,
    ) -> usize {
        if (king_sq as u16 & 0b100) != 0 {
            square = square.flip_file();
        }

        let index768 = ((if piece.color == side { 0 } else { 6 }) + piece.piece as usize) * 64
            + square.relative_to(side) as usize;

        FULL_THREATS
            + (if side_pinned {
                index768
            } else {
                index768 + PINNED / 2
            })
    }

    pub fn threat_pinned_index_from_threat(
        &self,
        king_sq: Square,
        side: Color,
        delta: &PinnedDelta,
    ) -> usize {
        self.threat_pinned_index(
            king_sq,
            side,
            side == delta.pinned_king_color,
            delta.piece,
            delta.square,
        )
    }

    pub fn apply_update(
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

    pub fn pst_apply_update(
        &self,
        next: &mut [i32; OUTPUTS],
        base: &[i32; OUTPUTS],
        update: &Update,
        side: Color,
    ) {
        match update.update_type {
            UpdateType::Move => {
                SimdOps::fused_add_sub_base32(
                    next,
                    base,
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add1.1,
                        update.add1.0,
                    ),
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub1.1,
                        update.sub1.0,
                    ),
                );
            }

            UpdateType::Capture => {
                SimdOps::fused_add_sub_sub_base32(
                    next,
                    base,
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add1.1,
                        update.add1.0,
                    ),
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub1.1,
                        update.sub1.0,
                    ),
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub2.1,
                        update.sub2.0,
                    ),
                );
            }
            UpdateType::Castle => {
                SimdOps::fused_add_add_sub_sub_base32(
                    next,
                    base,
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add1.1,
                        update.add1.0,
                    ),
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.add2.1,
                        update.add2.0,
                    ),
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub1.1,
                        update.sub1.0,
                    ),
                    self.pst_feature_lookup(
                        update.king_sq[side as usize],
                        side,
                        update.sub2.1,
                        update.sub2.0,
                    ),
                );
            }
        }
    }

    pub fn threat_apply_update(
        &self,
        next: &mut Aligned<i16, HL>,
        base: &Aligned<i16, HL>,
        update: &ThreatUpdate,
        side: Color,
        king_sq: Square,
    ) {
        let mut adds: ArrayVec<usize, MAX_TI_INPUTS> = ArrayVec::new();
        let mut subs: ArrayVec<usize, MAX_TI_INPUTS> = ArrayVec::new();
        for add in update.adds.iter() {
            match add {
                ThreatDeltaGeneral::ThreatDelta(add) => {
                    let i = self.threat_feature_lookup_index_from_threat(king_sq, side, add);
                    if i >= 0 {
                        adds.push(i as usize);
                    }
                }
                ThreatDeltaGeneral::PinnedDelta(add) => {
                    adds.push(self.threat_pinned_index_from_threat(king_sq, side, add) as usize);
                }
            }
        }
        for sub in update.subs.iter() {
            match sub {
                ThreatDeltaGeneral::ThreatDelta(sub) => {
                    let i = self.threat_feature_lookup_index_from_threat(king_sq, side, sub);
                    if i >= 0 {
                        subs.push(i as usize);
                    }
                }
                ThreatDeltaGeneral::PinnedDelta(sub) => {
                    subs.push(self.threat_pinned_index_from_threat(king_sq, side, sub) as usize);
                }
            }
        }

        unsafe {
            let mut acc = [_mm512_setzero_si512(); 8];
            for i in (0..HL).step_by(32 * 8) {
                for k in 0..8 {
                    acc[k] = *(base.as_ptr().add(i + k * 32) as *const __m512i);
                }

                let mut add_idx = 0;
                let mut sub_idx = 0;
                while add_idx < adds.len() && sub_idx < subs.len() {
                    let add = self.threat_weights[adds[add_idx]].as_ptr().add(i);
                    let sub = self.threat_weights[subs[sub_idx]].as_ptr().add(i);
                    for k in 0..8 {
                        acc[k] = _mm512_add_epi16(
                            acc[k],
                            _mm512_cvtepi8_epi16(*(add.add(k * 32) as *const __m256i)),
                        );
                        acc[k] = _mm512_sub_epi16(
                            acc[k],
                            _mm512_cvtepi8_epi16(*(sub.add(k * 32) as *const __m256i)),
                        );
                    }
                    add_idx += 1;
                    sub_idx += 1;
                }

                while add_idx < adds.len() {
                    let add = self.threat_weights[adds[add_idx]].as_ptr().add(i);
                    for k in 0..8 {
                        acc[k] = _mm512_add_epi16(
                            acc[k],
                            _mm512_cvtepi8_epi16(*(add.add(k * 32) as *const __m256i)),
                        );
                    }
                    add_idx += 1;
                }

                while sub_idx < subs.len() {
                    let sub = self.threat_weights[subs[sub_idx]].as_ptr().add(i);
                    for k in 0..8 {
                        acc[k] = _mm512_sub_epi16(
                            acc[k],
                            _mm512_cvtepi8_epi16(*(sub.add(k * 32) as *const __m256i)),
                        );
                    }
                    sub_idx += 1;
                }

                for k in 0..8 {
                    *(next.as_mut_ptr().add(i + k * 32) as *mut __m512i) = acc[k];
                }
            }
        }
    }
}
