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
    
    // 환경 변수로 사용자 정의 아키텍처 지정 가능
    // 예: CUDA_ARCH=sm_75 cargo build --features cuda-miner
    let arch = env::var("CUDA_ARCH").unwrap_or_else(|_| "sm_61".to_string());
    
    println!("cargo:warning=Compiling CUDA kernel: {:?}", src);
    println!("cargo:warning=Target architecture: {}", arch);
    println!("cargo:warning=Output PTX: {:?}", out_ptx);
    
    let mut nvcc_args = vec![
        "-ptx",
        "-O3",                    // Maximum optimization
        "--use_fast_math",        // Fast math operations (critical for performance)
        "--maxrregcount=64",      // Optimize register usage for better occupancy
        "-Xptxas=-v",             // Verbose PTX assembly info
    ];
    
    let arch_flag = format!("-arch={}", arch);
    nvcc_args.push(&arch_flag);
    
    let src_str = src.to_str().expect("invalid .cu path");
    let out_str = out_ptx.to_str().expect("invalid .ptx path");
    
    nvcc_args.extend_from_slice(&[
        src_str,
        "-o",
        out_str,
    ]);
    
    let output = Command::new(&nvcc)
        .args(&nvcc_args)
        .output()
        .expect("failed to invoke nvcc");

    if !output.status.success() {
        eprintln!("nvcc stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("nvcc stderr: {}", String::from_utf8_lossy(&output.stderr));
        panic!("nvcc failed to compile CUDA miner kernel");
    }
    
    println!("cargo:warning=CUDA kernel compiled successfully");
}
