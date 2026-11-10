use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for cfg_name in ["blake3_sse41_ffi", "blake3_avx2_ffi", "blake3_avx512_ffi", "blake3_neon", "blake3_sme2", "blake3_gpu", "shuttle"] {
        println!("cargo::rustc-check-cfg=cfg({cfg_name}, values(none()))");
    }
    let var = |name: &str| env::var(name).unwrap_or_default();
    let (arch, os) = (var("CARGO_CFG_TARGET_ARCH"), var("CARGO_CFG_TARGET_OS"));
    // The core's build conditions, where this crate's code reads them.
    if arch == "x86_64" {
        for name in ["blake3_sse41_ffi", "blake3_avx2_ffi", "blake3_avx512_ffi"] {
            println!("cargo::rustc-cfg={name}");
        }
    }
    if arch == "aarch64" {
        println!("cargo::rustc-cfg=blake3_neon");
        // The core's build script says whether it built the SME2 kernels.
        if env::var_os("DEP_BLAKE3_SERVIL_CORE_SME2").is_some() {
            println!("cargo::rustc-cfg=blake3_sme2");
        }
    }
    // The GPU (src/gpu.rs) on Apple Silicon macOS: the Metal bridge, and
    // the shader's source embedded for the driver to compile at run time.
    if os == "macos" && arch == "aarch64" {
        println!("cargo::rustc-cfg=blake3_gpu");
        let source = std::fs::read_to_string("blake3_gpu.metal")?;
        let out = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap());
        std::fs::write(out.join("blake3_gpu_shader.inc"), format!("static const char blake3_gpu_shader[] = R\"B3METAL({source})B3METAL\";\n"))?;
        cc::Build::new().cpp(true).file("blake3_gpu.mm").include(out).flag("-std=c++17").flag("-fobjc-arc").compile("blake3_gpu");
        println!("cargo::rustc-link-lib=framework=Metal");
        println!("cargo::rustc-link-lib=framework=Foundation");
    }
    println!("cargo::rerun-if-changed=blake3_gpu.metal");
    println!("cargo::rerun-if-changed=blake3_gpu.mm");
    Ok(())
}
