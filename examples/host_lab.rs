//! Safety cost study: identical-library control, then the named repair.
//! Run from a fresh directory with `main` or `next`; add `preflight` for
//! correctness checks with timing and quiet waits disabled.
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        (args.len() == 1 || (args.len() == 2 && args[1] == "preflight")) && matches!(args[0].as_str(), "main" | "next"),
        "choose main or next, with optional preflight"
    );
    let root = std::env::current_dir().expect("the study has a result directory");
    for kind in ["aa", "ab"] {
        let work = root.join(kind);
        std::fs::create_dir(&work).expect("each study phase has a fresh directory");
        let mut command = std::process::Command::new("python3");
        if args.len() == 2 {
            command.env("PINNED_RUN_PREFLIGHT", "1");
        }
        let status = command
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/studies/mac_cycles.py"))
            .arg(&args[0])
            .arg(kind)
            .current_dir(work)
            .status()
            .expect("the study driver starts");
        assert!(status.success(), "{kind} failed; its logs retain the cause");
    }
}
