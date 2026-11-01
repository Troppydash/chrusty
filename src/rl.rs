use std::fs::File as FsFile;
use std::io::{self, Read};
use std::path::Path;

use cozy_chess::{Board, Color, File, Move, Piece, Rank, Square};

use crate::spsa::Parameters;

pub const FEATURES: usize = 971;
pub const ACTIONS: usize = 8;
pub const DEFAULT_ACTION_SCALE: f32 = 0.5;

/// Action coordinate order from rlchess.param.TUNES.
pub const TUNES: [&str; ACTIONS] = [
    "p_lmr_offset",
    "p_lmr_divisor",
    "p_lmr_pv",
    "p_lmr_history",
    "p_lmp_depth",
    "p_rfp_depth",
    "p_nmp_margin",
    "p_probcut_beta",
];

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Linear {
    weights: Vec<f32>,
    biases: Vec<f32>,
    inputs: usize,
}

impl Linear {
    fn read(reader: &mut impl Read, inputs: usize, outputs: usize) -> io::Result<Self> {
        let count = inputs * outputs + outputs;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes)?;
            let value = f32::from_le_bytes(bytes);
            if !value.is_finite() {
                return Err(invalid("Non-finite policy weight"));
            }
            values.push(value);
        }
        let biases = values.split_off(inputs * outputs);
        Ok(Self {
            weights: values,
            biases,
            inputs,
        })
    }

    fn forward(&self, input: &[f32], output: &mut [f32]) {
        for ((row, bias), result) in self
            .weights
            .chunks_exact(self.inputs)
            .zip(&self.biases)
            .zip(output)
        {
            *result = row.iter().zip(input).fold(*bias, |sum, (w, x)| sum + w * x);
        }
    }
}

pub struct Policy {
    layers: [Linear; 5],
}

impl Policy {
    /// Load the versioned little-endian float32 format from scripts/export_rl.py.
    /// SB3 .zip checkpoints must first be exported.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_reader(io::BufReader::new(FsFile::open(path)?))
    }

    fn from_reader(mut reader: impl Read) -> io::Result<Self> {
        fn u32_le(reader: &mut impl Read) -> io::Result<u32> {
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes)?;
            Ok(u32::from_le_bytes(bytes))
        }
        let mut magic = [0; 8];
        reader.read_exact(&mut magic)?;
        if &magic != b"RLCHESS1"
            || u32_le(&mut reader)? != FEATURES as u32
            || u32_le(&mut reader)? != ACTIONS as u32
        {
            return Err(invalid("Unsupported RL policy format or dimensions"));
        }
        for name in TUNES {
            if u32_le(&mut reader)? as usize != name.len() {
                return Err(invalid("Incompatible RL action order"));
            }
            let mut bytes = vec![0; name.len()];
            reader.read_exact(&mut bytes)?;
            if bytes != name.as_bytes() {
                return Err(invalid("Incompatible RL action order"));
            }
        }
        let layers = [
            Linear::read(&mut reader, FEATURES, 128)?,
            Linear::read(&mut reader, 128, 64)?,
            Linear::read(&mut reader, 64, 64)?,
            Linear::read(&mut reader, 64, 64)?,
            Linear::read(&mut reader, 64, ACTIONS)?,
        ];
        if reader.read(&mut [0])? != 0 {
            return Err(invalid("Unexpected trailing policy data"));
        }
        Ok(Self { layers })
    }

    /// Match PPO.predict(..., deterministic=True), including action clipping.
    fn actions(&self, features: &[f32; FEATURES]) -> io::Result<[f32; ACTIONS]> {
        if features
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return Err(invalid("Expected 971 finite features in [0, 1]"));
        }
        let mut first = [0.0; 128];
        let mut second = [0.0; 64];
        let mut third = [0.0; 64];
        let mut actor = [0.0; 64];
        let mut actions = [0.0; ACTIONS];
        self.layers[0].forward(features, &mut first);
        first.iter_mut().for_each(|x| *x /= 1.0 + (-*x).exp());
        self.layers[1].forward(&first, &mut second);
        second.iter_mut().for_each(|x| *x /= 1.0 + (-*x).exp());
        self.layers[2].forward(&second, &mut third);
        self.layers[3].forward(&third, &mut actor);
        actor.iter_mut().for_each(|x| *x = x.tanh());
        self.layers[4].forward(&actor, &mut actions);
        if actions.iter().any(|x| !x.is_finite()) {
            return Err(invalid("Non-finite inference output"));
        }
        actions.iter_mut().for_each(|x| *x = x.clamp(-1.0, 1.0));
        Ok(actions)
    }

    /// Infer settings from a starting board and the moves played from it, just
    /// like Engine::search. `nnue_eval` is the raw static evaluation of the final
    /// position from its side to move's perspective. Moves use cozy's castling
    /// representation (king captures own rook). Uses the default action scale.
    pub fn infer(
        &self,
        startpos: &Board,
        nnue_eval: i32,
        moves: &[Move],
    ) -> io::Result<Parameters> {
        self.infer_with_scale(startpos, nnue_eval, moves, DEFAULT_ACTION_SCALE)
    }

    /// Use the action scale selected during training if it differs from 0.5.
    fn infer_with_scale(
        &self,
        startpos: &Board,
        nnue_eval: i32,
        moves: &[Move],
        action_scale: f32,
    ) -> io::Result<Parameters> {
        let features = Self::features(startpos, nnue_eval, moves)?;
        Self::settings(&self.actions(&features)?, action_scale)
    }

    /// Build the training observation with the final side to move as learner.
    /// Repetition considers only the supplied history, starting at `startpos`.
    pub fn features(
        startpos: &Board,
        nnue_eval: i32,
        moves: &[Move],
    ) -> io::Result<[f32; FEATURES]> {
        let mut board = startpos.clone();
        let mut last_move = None;
        for &mv in moves {
            let mut target = mv.to;
            // python-chess uses the king's destination for standard castling.
            if board.piece_on(mv.from) == Some(Piece::King)
                && board.color_on(mv.to) == Some(board.side_to_move())
            {
                target = Square::new(
                    if mv.to.file() > mv.from.file() {
                        File::G
                    } else {
                        File::C
                    },
                    mv.from.rank(),
                );
            }
            board
                .try_play(mv)
                .map_err(|_| invalid("Illegal move in RL position history"))?;
            last_move = Some((mv.from, target));
        }

        let color = board.side_to_move();
        let flip = if color == Color::Black { 56 } else { 0 };
        let mut features = [0.0; FEATURES];
        for side in [color, !color] {
            let offset = if side == color { 0 } else { 6 };
            for piece in Piece::ALL {
                for square in board.colored_pieces(side, piece) {
                    features[(offset + piece as usize) * 64 + (square as usize ^ flip)] = 1.0;
                }
            }
        }
        let (source, target) = last_move.map_or((64, 64), |(from, to)| {
            (from as usize ^ flip, to as usize ^ flip)
        });
        features[768 + source] = 1.0;
        features[833 + target] = 1.0;
        let ply = 2 * (board.fullmove_number() as u32 - 1) + u32::from(color == Color::Black);
        features[898] = (ply as f64 / (ply as f64 + 100.0)) as f32;
        for (i, side) in [color, !color].into_iter().enumerate() {
            features[899 + 2 * i] = u8::from(board.castle_rights(side).short.is_some()) as f32;
            features[900 + 2 * i] = u8::from(board.castle_rights(side).long.is_some()) as f32;
        }
        let ep = board.en_passant().map_or(64, |file| {
            Square::new(file, Rank::Sixth.relative_to(color)) as usize ^ flip
        });
        features[903 + ep] = 1.0;
        features[968] = (board.halfmove_clock() as f64 / 150.0).min(1.0) as f32;

        // same_position applies FIDE's legal-en-passant rule, as python-chess
        // does. Comparing raw hashes would misclassify uncapturable EP targets.
        let mut previous = startpos.clone();
        for &mv in moves {
            if board.same_position(&previous) {
                features[969] = 1.0;
                break;
            }
            previous.play_unchecked(mv); // Validated in the first pass.
        }
        let score = nnue_eval as f64 / 400.0;
        features[970] = if score >= 0.0 {
            1.0 / (1.0 + (-score).exp())
        } else {
            let exp = score.exp();
            exp / (1.0 + exp)
        } as f32;
        Ok(features)
    }

    fn settings(actions: &[f32; ACTIONS], action_scale: f32) -> io::Result<Parameters> {
        if !action_scale.is_finite()
            || action_scale <= 0.0
            || action_scale > 1.0
            || actions.iter().any(|x| !x.is_finite())
        {
            return Err(invalid(
                "Expected finite actions and action_scale in (0, 1]",
            ));
        }
        let mut settings = Parameters::default();
        for (name, action) in TUNES.into_iter().zip(actions) {
            let (default, min, max) =
                Parameters::option_range(name).ok_or_else(|| invalid("Unknown policy option"))?;
            let value = action.clamp(-1.0, 1.0) * action_scale;
            let span = if value >= 0.0 {
                max - default
            } else {
                default - min
            };
            // NumPy scales in f32; Python then multiplies and rounds in f64.
            let value = (default as f64 + value as f64 * span as f64).round_ties_even() as i32;
            settings.uci_apply(name, &value.to_string());
        }
        Ok(settings)
    }
}
