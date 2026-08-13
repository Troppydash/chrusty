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

        impl Parameters {
            pub fn uci_apply(&mut self, name: &str, value: &str) {
                match name {
                    $(stringify!($name) => {
                        self.$name = value.parse::<$type>().unwrap();
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
    p_lmr_check: i32 = 522, 400, 1000, 300,
    p_lmr_cutnode: i32 = 1124, 1000, 1600, 300,
    p_lmr_capture: i32 = 525, 300, 700, 300,
    p_lmr_improving: i32 = 1372, 1000, 1600, 300,
    p_lmr_tt_depth: i32 = 815, 500, 1100, 300,
    p_lmr_pv: i32 = 903, 500, 1200, 300,
    p_lmr_complexity: i32 = 925, 600, 1300, 300,
    p_lmr_history: i32 = 1243, 600, 1300, 300,
    p_lmr_quiet_div: i32 = 10971, 7000, 12000, 300,
    p_lmr_capture_div: i32 = 14365, 11000, 17000, 300,
}
