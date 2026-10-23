macro_rules! define {
    ($($name:ident: $type:ty = $default:expr, $min:expr, $max:expr, $step:expr),* $(,)?) => {
        #[derive(Clone)]
        pub struct Parameters {
            $(pub $name: $type,)*
        }

        impl Default for Parameters {
            fn default() -> Self {
                Self {
                    $($name: $default,)*
                }
            }
        }

        #[cfg(test)]
        #[test]
        fn parameter_ranges_and_uci_application() {
            let mut parameters = Parameters::default();
            $(
                assert!($min <= $default && $default <= $max, "{}", stringify!($name));
                assert!($step > 0, "{}", stringify!($name));
                parameters.uci_apply(stringify!($name), &($min as i64 - 1).to_string());
                assert_eq!(parameters.$name, $min);
                parameters.uci_apply(stringify!($name), &($max as i64 + 1).to_string());
                assert_eq!(parameters.$name, $max);
                parameters.uci_apply(stringify!($name), &$default.to_string());
                assert_eq!(parameters.$name, $default);
            )*
        }

        impl Parameters {
            pub fn uci_apply(&mut self, name: &str, value: &str) {
                match name {
                    $(stringify!($name) => {
                        self.$name = value.parse::<$type>().unwrap().clamp($min, $max);
                    })*
                    _ => panic!("unexpected name {}", name),
                }
            }

            pub fn uci_text() -> String {
                let mut s = String::new();
                $(
                    s.push_str(&format!("option name {} type spin default {} min {} max {}\n",
                                        stringify!($name), $default, $min, $max));
                )*
                s
            }

            pub fn uci_json() -> String {
                let mut s = String::new();
                $(
                    s.push_str(&format!("\"{}\": {{ \"value\": {}, \"min_value\": {}, \"max_value\": {}, \"step\": {} }},",
                                       stringify!($name), $default, $min, $max, $step));
                )*

                if s.len() > 0 {
                   s.pop();
                }
                s
            }
            pub fn uci_leela() -> String {
                let mut s = String::new();
                $(
                    s.push_str(&format!("\"{}\": \"Integer({}, {})\",\n",
                                       stringify!($name), $min, $max));
                )*

                if s.len() > 1 {
                   s.pop();
                   s.pop();
                }
                s
            }
        }
    };
}

define! {
    p_test: i32 = 100, 0, 200, 10,

    // lmr
    p_lmr_check: i32 = 796, 400, 1300, 300,
    p_lmr_cutnode: i32 = 1079, 1000, 1600, 300,
    p_lmr_capture: i32 = 410, 300, 700, 300,
    p_lmr_improving: i32 = 1255, 1000, 1600, 300,
    p_lmr_tt_depth: i32 = 988, 500, 1100, 300,
    p_lmr_pv: i32 = 1086, 500, 1200, 300,
    p_lmr_complexity: i32 = 932, 600, 1300, 300,
    p_lmr_history: i32 = 918, 600, 1500, 300,
    p_lmr_quiet_div: i32 = 7276, 6000, 15000, 300,
    p_lmr_capture_div: i32 = 13156, 7000, 17000, 300,

    // probcut
    p_probcut_beta : i32 = 200, 100, 300, 20,
    p_probcut_margin : i32 = 640, 500, 800, 20,

    // low depth
    p_lowdepth_see_quiet_base : i32 = 50, 25, 100, 10,
    p_lowdepth_see_quiet_depth : i32 = 50, 25, 100, 10,
    p_lowdepth_see_capture_base : i32 = 100, 50, 200, 10,
    p_lowdepth_see_capture_depth : i32 = 100, 50, 200, 10,
    p_lowdepth_fut_quiet_base : i32 = 100, 100, 300, 10,
    p_lowdepth_fut_quiet_depth : i32 = 100, 100, 300, 10,
    p_lowdepth_fut_capture_base : i32 = 100, 100, 300, 10,
    p_lowdepth_fut_capture_depth : i32 = 100, 100, 300, 10,

    // singular extension
    p_singular_double : i32 = 50, 25, 100, 10,
    p_singular_triple : i32 = 300, 200, 400, 10,

    // LMR table and re-search
    p_lmr_base: i32 = 99, 50, 150, 5,
    p_lmr_divisor: i32 = 314, 200, 500, 15,
    p_lmr_offset: i32 = 512, 0, 1024, 64,
    p_lmr_exact: i32 = 1500, 500, 2500, 100,
    p_lmr_eval_scale: i32 = 4, 0, 8, 1,
    p_research_deeper: i32 = 50, 20, 100, 5,
    p_research_shallower: i32 = 5, 0, 20, 2,

    // Razoring, reverse futility, and null move
    p_razor_margin: i32 = 300, 150, 450, 15,
    p_rfp_quad: i32 = 5, 0, 15, 1,
    p_rfp_depth: i32 = 50, 20, 100, 5,
    p_rfp_improving: i32 = 60, 0, 120, 8,
    p_rfp_complexity_div: i32 = 4, 2, 8, 1,
    p_nmp_margin: i32 = 200, 100, 350, 15,
    p_nmp_margin_depth: i32 = 20, 10, 40, 2,
    p_nmp_margin_pv: i32 = 100, 0, 200, 10,
    p_nmp_base: i32 = 6, 3, 9, 1,
    p_nmp_depth_div: i32 = 4, 2, 8, 1,
    p_nmp_eval_div: i32 = 300, 150, 600, 20,
    p_nmp_eval_max: i32 = 6, 2, 9, 1,
    p_nmp_improving: i32 = 1, 0, 3, 1,
    p_nmp_material: i32 = 600, 300, 1000, 25,
    p_tt_probcut_margin: i32 = 500, 250, 750, 25,

    // Late move/history pruning, quiescence, aspiration
    p_history_prune: i32 = 3000, 1500, 5000, 200,
    p_lmp_base: i32 = 2800, 1400, 4200, 150,
    p_lmp_improving: i32 = 1000, 0, 2000, 100,
    p_lmp_depth: i32 = 1300, 650, 2000, 80,
    p_qsee_margin: i32 = 50, 0, 150, 5,
    p_qhistory_bonus: i32 = 200, 100, 400, 10,
    p_qhistory_check: i32 = 100, 0, 100, 5,
    p_asp_window: i32 = 10, 6, 40, 2,
    p_asp_score_scale: i32 = 23000, 10000, 50000, 2000,
    p_asp_fail_low_div: i32 = 6, 2, 10, 1,
    p_asp_fail_high_div: i32 = 3, 2, 8, 1,

    // Quiet and capture history learning
    p_history_bonus_depth: i32 = 180, 90, 300, 10,
    p_history_bonus_base: i32 = 15, 0, 100, 5,
    p_history_bonus_max: i32 = 2000, 1000, 4000, 100,
    p_history_malus_depth: i32 = 190, 90, 320, 10,
    p_history_malus_base: i32 = 30, 0, 80, 5,
    p_history_malus_max: i32 = 2000, 1000, 4000, 100,
    p_capture_bonus_depth: i32 = 180, 90, 300, 10,
    p_capture_bonus_base: i32 = 15, 0, 100, 5,
    p_capture_bonus_max: i32 = 2000, 1000, 4000, 100,
    p_capture_malus_depth: i32 = 190, 90, 320, 10,
    p_capture_malus_base: i32 = 30, 0, 80, 5,
    p_capture_malus_max: i32 = 2000, 1000, 4000, 100,

    // Move ordering weights
    p_history_main_weight: i32 = 1024, 512, 1536, 64,
    p_history_pawn_weight: i32 = 1024, 512, 1536, 64,
    p_history_lowply_weight: i32 = 2048, 1024, 3072, 128,
    p_history_cont1_weight: i32 = 1024, 512, 1536, 64,
    p_history_cont2_weight: i32 = 900, 450, 1350, 64,
    p_history_cont4_weight: i32 = 700, 350, 1050, 48,
    p_history_cont6_weight: i32 = 400, 200, 800, 32,
    p_capture_lowply_weight: i32 = 1024, 512, 1536, 64,
    p_mvv_multiplier: i32 = 8, 4, 16, 1,
    p_lva_divisor: i32 = 8, 4, 16, 1,
    p_good_capture_see_div: i32 = 32, 16, 64, 2,
    p_threat_penalty: i32 = 3000, 1500, 4500, 200,
    p_escape_minor: i32 = 3000, 1500, 4500, 200,
    p_escape_rook: i32 = 5000, 2500, 7500, 300,
    p_escape_queen: i32 = 7000, 3500, 10000, 400,
    p_check_bonus: i32 = 5000, 2500, 7500, 300,

    // Correction history weights and learning
    p_corr_pawn_weight: i32 = 32, 8, 64, 2,
    p_corr_nonpawn_weight: i32 = 24, 6, 48, 2,
    p_corr_major_weight: i32 = 24, 6, 48, 2,
    p_corr_minor_weight: i32 = 24, 6, 48, 2,
    p_corr_cont1_weight: i32 = 24, 6, 48, 2,
    p_corr_cont3_weight: i32 = 12, 3, 24, 1,
    p_corr_update_div: i32 = 8, 4, 16, 1,
    p_corr_bonus_max: i32 = 256, 128, 512, 16,

    // material values used by SEE, ordering, capture futility and NMP
    p_value_pawn: i32 = 100, 70, 140, 5,
    p_value_knight: i32 = 320, 250, 390, 10,
    p_value_bishop: i32 = 340, 270, 420, 10,
    p_value_rook: i32 = 620, 480, 760, 20,
    p_value_queen: i32 = 1300, 1000, 1500, 40,
}

impl Parameters {
    #[inline]
    pub fn piece_value(&self, piece: cozy_chess::Piece) -> i32 {
        use cozy_chess::Piece;
        match piece {
            Piece::Pawn => self.p_value_pawn,
            Piece::Knight => self.p_value_knight,
            Piece::Bishop => self.p_value_bishop,
            Piece::Rook => self.p_value_rook,
            Piece::Queen => self.p_value_queen,
            Piece::King => 1500,
        }
    }
}
