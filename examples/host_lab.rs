//! Safety cost study: identical-library control, then the named repair.
//! Run from a fresh directory with `main` or `next` as its argument.
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(args.len() == 1 && matches!(args[0].as_str(), "main" | "next"), "choose main or next");
    let root = std::env::current_dir().expect("the study has a result directory");
    for kind in ["aa", "ab"] {
        let work = root.join(kind);
        std::fs::create_dir(&work).expect("each study phase has a fresh directory");
        let status = std::process::Command::new("python3")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/studies/mac_cycles.py"))
            .arg(&args[0])
            .arg(kind)
            .current_dir(work)
            .status()
            .expect("the study driver starts");
        assert!(status.success(), "{kind} failed; its logs retain the cause");
    }
}
