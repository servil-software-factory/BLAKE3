//! Native assembly kernels assembled directly by `rustc`'s LLVM assembler via
//! `core::arch::global_asm!` (no external C compiler or `cc` crate required; #487).

use core::arch::global_asm;

#[cfg(blake3_sse41_ffi)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_sse41.s")), options(raw));

#[cfg(blake3_avx2_ffi)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_avx2.s")), options(raw));

#[cfg(blake3_sse41_ffi)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_partial.s")), options(raw));

#[cfg(blake3_sse41_ffi)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_chunk_root.s")), options(raw));

#[cfg(blake3_avx512_ffi)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_avx512.s")), options(raw));

#[cfg(all(blake3_avx512_ffi, target_os = "linux"))]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_avx512_32.s")), options(raw));

#[cfg(all(feature = "std", blake3_avx512_ffi, target_os = "linux"))]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_output16.s")), options(raw));

#[cfg(all(feature = "std", blake3_avx512_ffi, target_os = "linux"))]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_output8.s")), options(raw));

#[cfg(blake3_avx2_ffi)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_packed_x86.s")), options(raw));

#[cfg(blake3_avx2_ffi)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_counter_x86.s")), options(raw));

#[cfg(blake3_neon)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_neon_hybrid.s")), options(raw));

#[cfg(blake3_neon)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_packed_neon.s")), options(raw));

#[cfg(blake3_neon)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_counter_neon.s")), options(raw));

#[cfg(blake3_sme2)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_sme2.s")), options(raw));

#[cfg(blake3_sme2)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_packed_sme2.s")), options(raw));

#[cfg(blake3_sme2)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_counter_sme2.s")), options(raw));

#[cfg(blake3_sme2)]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_sme2_hybrid.s")), options(raw));

#[cfg(all(feature = "std", blake3_neon))]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_output_neon.s")), options(raw));

#[cfg(all(feature = "std", blake3_sme2))]
global_asm!(include_str!(concat!(env!("OUT_DIR"), "/blake3_output_sme2.s")), options(raw));
