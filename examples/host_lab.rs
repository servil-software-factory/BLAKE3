//! Native complete-cycle study: identical-library control, then candidate.
//! Run from a fresh result directory; the runner owns its process group.
fn main() {
    let root = std::env::current_dir().expect("the study has a result directory");
    for kind in ["aa", "ab"] {
        let work = root.join(kind);
        std::fs::create_dir(&work).expect("each study phase has a fresh directory");
        let status = std::process::Command::new("python3")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/studies/mac_first_chunk.py"))
            .arg(kind)
            .current_dir(work)
            .status()
            .expect("the study driver starts");
        assert!(status.success(), "{kind} failed; its logs retain the cause");
    }
}
