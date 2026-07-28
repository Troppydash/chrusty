use std::fs;

fn main() {
    println!("cargo:rerun-if-changed=hash.nn");

    let hash = fs::read_to_string("hash.nn")
        .expect("Failed to read hash.nn")
        .trim()
        .to_string();
    let target_file = format!("../nets/{}.bin", hash);

    println!("cargo:rerun-if-changed={}", target_file);
    println!("cargo:rustc-env=EVAL_FILE={}", target_file);
}
