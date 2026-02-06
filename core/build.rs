use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var("CARGO_FEATURE_CUDA_MINER").is_err() {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let src = PathBuf::from("src/consensus/cuda/miner.cu");
    let out_ptx = out_dir.join("miner.ptx");

    println!("cargo:rerun-if-changed=src/consensus/cuda/miner.cu");
    println!("cargo:rerun-if-env-changed=NVCC");

    let nvcc = env::var("NVCC").unwrap_or_else(|_| "nvcc".to_string());
    let status = Command::new(nvcc)
        .args([
            "-ptx",
            "-O3",
            src.to_str().expect("invalid .cu path"),
            "-o",
            out_ptx.to_str().expect("invalid .ptx path"),
            "-lineinfo",
        ])
        .status()
        .expect("failed to invoke nvcc");

    if !status.success() {
        panic!("nvcc failed to compile CUDA miner kernel");
    }
}
