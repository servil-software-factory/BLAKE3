//! Minimal RAII temp-dir and temp-file helpers for tests, replacing `tempfile`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn resolve_base(base: &Path) -> io::Result<PathBuf> {
    if base.is_absolute() { Ok(base.to_path_buf()) } else { Ok(std::env::current_dir()?.join(base)) }
}

pub(crate) fn tempdir() -> io::Result<TempDir> {
    tempdir_in(std::env::temp_dir())
}

pub(crate) fn tempdir_in(base: impl AsRef<Path>) -> io::Result<TempDir> {
    let base = resolve_base(base.as_ref())?;
    let pid = std::process::id();
    for _ in 0..1024 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let candidate = base.join(format!(".b3sum-test-{pid}-{id}"));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(TempDir(candidate)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "failed to allocate unique temp dir"))
}

#[allow(dead_code)]
pub(crate) struct NamedTempFile {
    file: File,
    path: PathBuf,
}

#[allow(dead_code)]
impl NamedTempFile {
    pub(crate) fn new() -> io::Result<Self> {
        let base = resolve_base(&std::env::temp_dir())?;
        let pid = std::process::id();
        for _ in 0..1024 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(".b3sum-test-file-{pid}-{id}"));
            let mut opts = OpenOptions::new();
            opts.read(true).write(true).create_new(true);
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            match opts.open(&path) {
                Ok(file) => return Ok(Self { file, path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "failed to allocate unique temp file"))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn as_file(&self) -> &File {
        &self.file
    }
}

impl Write for NamedTempFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Drop for NamedTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
