use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for cfg_name in ["blake3_sse41_ffi", "blake3_avx2_ffi", "blake3_avx512_ffi", "blake3_neon", "blake3_sme2", "shuttle"] {
        println!("cargo::rustc-check-cfg=cfg({cfg_name}, values(none()))");
    }
    let var = |name: &str| env::var(name).unwrap_or_default();
    let arch = var("CARGO_CFG_TARGET_ARCH");
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
    Ok(())
}
