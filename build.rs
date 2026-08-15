use std::{fs, path::Path};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(permute_file)");
    println!("cargo:rerun-if-changed=hash.nn");

    let hash = fs::read_to_string("hash.nn")
        .expect("Failed to read hash.nn")
        .trim()
        .to_string();
    let target_file = format!("../nets/{}.bin", hash);
    let permute_file = format!("../nets/permute_{}.bin", hash);
    if Path::new(&format!("./nets/permute_{}.bin", hash)).exists() {
        println!("cargo::rustc-cfg=permute_file");
    }

    println!("cargo:rerun-if-changed={}", target_file);
    println!("cargo:rerun-if-changed={}", permute_file);
    println!("cargo:rustc-env=EVAL_FILE={}", target_file);
    println!("cargo:rustc-env=PERMUTE_FILE={}", permute_file);
}
