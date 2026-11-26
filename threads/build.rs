fn main() {
    println!("cargo::rustc-check-cfg=cfg(shuttle, values(none()))");
    println!("cargo::rerun-if-changed=build.rs");
}
