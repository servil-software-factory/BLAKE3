//! Native assembly kernels assembled directly by `rustc`'s LLVM assembler via
//! `core::arch::global_asm!` (no external C compiler or `cc` crate required; #487).
//! The ordered list of `global_asm!` items is emitted directly by `build.rs`
//! into `OUT_DIR/asm_includes.rs` so staging and inclusion share one inventory.

include!(concat!(env!("OUT_DIR"), "/asm_includes.rs"));
