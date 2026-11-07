use std::mem;

use cozy_chess::{BitBoard, Board, Color, Piece, Square};

use crate::{
    ext::{BitBoardExt, ColoredPiece, ExtBoard},
    nnue::{
        network::{KINGS, Network, OUTPUTS, SimdOps},
        update::Update,
    },
    param::MAX_DEPTH_USIZE,
};

#[repr(C, align(64))]
pub struct Accumulator {
    pub vals: [[i32; OUTPUTS]; 2],
    is_clean: [bool; 2],
    up: Update,
}

impl Accumulator {
    fn new() -> Self {
        unsafe { mem::zeroed() }
    }
}

pub struct FinnyEntry {
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
        SimdOps::fused_add32(
            &mut self.acc.vals[side as usize],
            network.pst_feature_lookup(king_sq, side, piece, square),
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
        SimdOps::fused_sub32(
            &mut self.acc.vals[side as usize],
            network.pst_feature_lookup(king_sq, side, piece, square),
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
        SimdOps::fused_add_sub32(
            &mut self.acc.vals[side as usize],
            network.pst_feature_lookup(king_sq, side, piece, square_to),
            network.pst_feature_lookup(king_sq, side, piece, square_from),
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
        for entry in self.entries.iter_mut().flatten() {
            entry.bycolor = [[BitBoard::EMPTY; 2]; 2];
            entry.bypiece = [[BitBoard::EMPTY; 6]; 2];
        }
    }
}

pub struct Pst {
    pub side: Box<[Accumulator]>,
    pub head: usize,
    finny: Box<FinnyTable>,
}

impl Pst {
    pub fn new() -> Self {
        // we don't have clone on [accumulator]
        let mut sides = vec![];
        for _ in 0..MAX_DEPTH_USIZE {
            sides.push(Accumulator::new());
        }

        Self {
            side: sides.into_boxed_slice(),
            head: 0,
            finny: FinnyTable::new(),
        }
    }

    pub fn init(&mut self, board: &Board, network: &Box<Network>) {
        self.head = 0;
        self.side[self.head].up.king_sq[0] = board.king(Color::White);
        self.side[self.head].up.king_sq[1] = board.king(Color::Black);
        self.refresh(board, self.head, Color::White, network);
        self.refresh(board, self.head, Color::Black, network);
    }

    pub fn clear(&mut self, network: &Box<Network>) {
        self.finny.clear();
        for is_mirrored in 0..2 {
            for bucket in 0..KINGS {
                let entry = &mut self.finny.entries[is_mirrored][bucket];
                SimdOps::fused_copy32(&mut entry.acc.vals[0], &network.pst_bias);
                SimdOps::fused_copy32(&mut entry.acc.vals[1], &network.pst_bias);
            }
        }
    }

    pub fn make_move(&mut self, update: &Update) {
        self.head += 1;
        debug_assert!(self.head < MAX_DEPTH_USIZE);
        self.side[self.head].is_clean = [false; 2];
        self.side[self.head].up = update.clone();
    }

    pub fn unmake_move(&mut self) {
        self.head -= 1;
    }

    fn refresh(&mut self, board: &Board, head: usize, side: Color, network: &Box<Network>) {
        // finny table refresh
        let king_sq = board.king(side);
        let bucket = Network::get_king_bucket(king_sq, side);
        let mirrored = Network::is_mirrored(king_sq);

        let entry = &mut self.finny.entries[mirrored as usize][bucket];

        for color in 0..=1 {
            for piece in 0..6 {
                // TODO: we can also improve this
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
                        &network,
                    );
                }

                while !added.is_empty() {
                    let square_to = added.pop();
                    entry.acc_add_piece(
                        side,
                        king_sq,
                        ColoredPiece::new(Color::ALL[color], Piece::ALL[piece]),
                        square_to,
                        &network,
                    );
                }

                while !removed.is_empty() {
                    let square_from = removed.pop();
                    entry.acc_sub_piece(
                        side,
                        king_sq,
                        ColoredPiece::new(Color::ALL[color], Piece::ALL[piece]),
                        square_from,
                        &network,
                    );
                }
            }
        }

        self.side[head].vals[side as usize] = entry.acc.vals[side as usize];
        entry.bycolor[side as usize] = board.by_color();
        entry.bypiece[side as usize] = board.by_piece();
        self.side[head].is_clean[side as usize] = true;
    }

    pub fn catchup(&mut self, head: usize, board: &Board, network: &Box<Network>) {
        for side in 0..=1 {
            if self.side[head].is_clean[side] {
                continue;
            }

            let mut base = head;
            loop {
                // full refresh check
                if Network::needs_refresh(
                    Color::ALL[side],
                    self.side[base].up.king_sq[side],
                    self.side[head].up.king_sq[side],
                ) {
                    self.refresh(board, head, Color::ALL[side], network);
                    break;
                }

                // else check for incremental update
                if self.side[base].is_clean[side] {
                    for i in base + 1..=head {
                        let (base, next) = self.side.split_at_mut(i);

                        network.pst_apply_update(
                            &mut next[0].vals[side],
                            &base[i - 1].vals[side],
                            &next[0].up,
                            Color::ALL[side],
                        );
                        self.side[i].is_clean[side] = true;
                    }

                    self.side[head].is_clean[side] = true;
                    break;
                }

                if base == 0 {
                    panic!("no clean base");
                }
                base -= 1;
            }
        }
    }
}
