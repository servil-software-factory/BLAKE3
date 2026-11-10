//! The build version (version/build_version.rs), against a repository made
//! here: each expected name built from Git's own answers (the commit, the
//! tree that committing the dirty tree makes), never from the code under
//! test.

#[path = "../version/build_version.rs"]
mod build_version;

use std::path::Path;
use std::process::Command;

/// `what` and its time, on stderr: nextest prints a failed or timed-out
/// test's output, so a test that hits the clock shows each step's time and
/// ends on the step it was in (BLAKE3 #347: 6 to 60 seconds a test once
/// in a VM, milliseconds elsewhere, with nothing to say where).
fn timed<T>(what: std::fmt::Arguments, f: impl FnOnce() -> T) -> T {
    eprint!("{what}: ");
    let start = std::time::Instant::now();
    let result = f();
    eprintln!("{} ms", start.elapsed().as_millis());
    result
}

fn git(dir: &Path, args: &[&str]) -> String {
    timed(format_args!("git {args:?}"), || git_untimed(dir, args))
}

fn git_untimed(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

struct Repo(std::path::PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A directory of this test's own, in memory where the system has the
/// standard place for it (Linux's /dev/shm), else under Cargo's scratch
/// directory for integration tests. These tests' time is Git's work on
/// directories and small files, which waits behind whatever else is
/// writing to the disk: in a VM whose builds' disk was busy (a mutation
/// run's shards, check-builds.sh), creating one directory took up to
/// 0.9 s and `git init` up to 6 s, where each takes milliseconds on an
/// idle disk, and a test passed its clock (BLAKE3 #347: the steps' times
/// showed Git's, and writing 20 GiB beside them reproduced it). What the
/// tests check is the name, whatever holds the files.
fn scratch(name: &str) -> std::path::PathBuf {
    let memory = Path::new("/dev/shm");
    let base = if memory.is_dir() { memory } else { Path::new(env!("CARGO_TARGET_TMPDIR")) };
    let dir = base.join(format!("blake3-servil-build-version-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo(name: &str) -> Repo {
    let dir = scratch(name);
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("code.rs"), "one").unwrap();
    git(&dir, &["add", "code.rs"]);
    git(&dir, &["commit", "-q", "-m", "code"]);
    Repo(dir)
}

/// The name, its scratch index beside the repository (each test's own, as
/// build.rs's is its OUT_DIR).
fn name(dir: &Path, version: &str) -> String {
    let scratch = dir.with_extension("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    // The version crate's directory: inside the repository, `version/`
    // (ignored by its Git, as an empty directory is); for a package, the
    // package itself.
    let krate = if dir.join(".git").exists() { dir.join("version") } else { dir.to_owned() };
    std::fs::create_dir_all(&krate).unwrap();
    let name = timed(format_args!("build_version"), || build_version::build_version(&krate, version, &scratch).0);
    std::fs::remove_dir_all(&scratch).unwrap();
    name
}

#[test]
fn a_release_commit_is_named_by_its_version_alone() {
    let r = repo("release");
    let code = git(&r.0, &["rev-parse", "HEAD"]);
    std::fs::write(r.0.join("Cargo.toml"), "version").unwrap();
    git(&r.0, &["add", "Cargo.toml"]);
    git(&r.0, &["commit", "-q", "-m", "release"]);
    let version = format!("0.8.0+{code}");
    assert_eq!(name(&r.0, &version), version);
    // Its code commit carries the same version and is not the release.
    git(&r.0, &["checkout", "-q", "--detach", &code]);
    assert_eq!(name(&r.0, &version), format!("{version}.{code}"));
}

#[test]
fn a_later_commit_is_named_by_its_commit() {
    let r = repo("dev");
    let commit = git(&r.0, &["rev-parse", "HEAD"]);
    assert_eq!(name(&r.0, "0.8.0+dev"), format!("0.8.0+dev.{commit}"));
}

#[test]
fn a_dirty_tree_is_named_by_its_tree_id_and_cargos_marker_is_no_change() {
    let r = repo("dirty");
    let commit = git(&r.0, &["rev-parse", "HEAD"]);
    std::fs::write(r.0.join(".cargo-ok"), "{\"v\":1}").unwrap();
    assert_eq!(name(&r.0, "0.8.0+dev"), format!("0.8.0+dev.{commit}"), "Cargo's marker alone");
    std::fs::write(r.0.join("code.rs"), "two").unwrap();
    std::fs::write(r.0.join("new.rs"), "three").unwrap();
    let got = name(&r.0, "0.8.0+dev");
    // The tree that committing exactly this working tree makes.
    git(&r.0, &["add", "-A", "--", ".", ":(exclude).cargo-ok"]);
    git(&r.0, &["commit", "-q", "-m", "as it was"]);
    let tree = git(&r.0, &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(got, format!("0.8.0+dev.{commit}.dirty.{tree}"));
    assert_ne!(tree, git(&r.0, &["rev-parse", "HEAD^^{tree}"]));
}

/// An edit Git's own index catches is in the name. Git reads a file's
/// content again, whatever its recorded size and time say, when the file's
/// time is no earlier than the index's own: the file may have changed
/// after the index was written, unseen at Git's whole-second times. The
/// name is made through a copy of the index, and a copy written later
/// than the edit would trust the record. Here an edit keeps its file's
/// size and time, in the second the index was written, ten seconds ago:
/// `git add` in the checkout sees it, and so must the name. (Found as one
/// run in about a hundred of the dirty-tree test above, when a second
/// began between its edit and the copy.)
#[test]
fn an_edit_in_the_second_its_index_was_written_is_in_the_name() {
    let r = repo("racy");
    let commit = git(&r.0, &["rev-parse", "HEAD"]);
    // Git's file times alone decide, as in a build of Git with whole seconds.
    git(&r.0, &["config", "core.trustctime", "false"]);
    let then = std::time::SystemTime::now() - std::time::Duration::from_secs(10);
    let set = |path: &Path| std::fs::File::options().write(true).open(path).unwrap().set_modified(then).unwrap();
    let code = r.0.join("code.rs");
    // The record: code.rs as committed, at `then`; the index written at `then`.
    set(&code);
    git(&r.0, &["add", "code.rs"]);
    assert_eq!(name(&r.0, "0.8.0+dev"), format!("0.8.0+dev.{commit}"), "unchanged so far");
    set(&r.0.join(".git/index"));
    // The edit: the same size, the same time.
    std::fs::write(&code, "two").unwrap();
    set(&code);
    let got = name(&r.0, "0.8.0+dev");
    git(&r.0, &["add", "-A"]);
    git(&r.0, &["commit", "-q", "-m", "the edit"]);
    assert_eq!(git(&r.0, &["show", "HEAD:code.rs"]), "two", "Git's own index saw the edit");
    let tree = git(&r.0, &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(got, format!("0.8.0+dev.{commit}.dirty.{tree}"));
}

#[test]
fn a_package_is_named_from_its_vcs_info() {
    let r = Repo(scratch("package"));
    let sha = "0123456789abcdef0123456789abcdef01234567";
    std::fs::write(r.0.join(".cargo_vcs_info.json"), format!("{{\n  \"git\": {{\n    \"sha1\": \"{sha}\",\n    \"dirty\": true\n  }}\n}}")).unwrap();
    assert_eq!(name(&r.0, "0.8.0+dev"), format!("0.8.0+dev.{sha}.dirty"));
    std::fs::write(r.0.join(".cargo_vcs_info.json"), format!("{{\"git\":{{\"sha1\":\"{sha}\"}}}}")).unwrap();
    assert_eq!(name(&r.0, "0.8.0+dev"), format!("0.8.0+dev.{sha}"));
    assert_eq!(name(&r.0, &format!("0.8.0+{sha}")), format!("0.8.0+{sha}"), "a clean release package");
}

#[test]
fn this_build_starts_with_the_crates_version() {
    assert!(blake3_servil::BUILD_VERSION.starts_with(env!("CARGO_PKG_VERSION")), "{}", blake3_servil::BUILD_VERSION);
}
