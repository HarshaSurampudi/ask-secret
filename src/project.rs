//! Works out which project a directory belongs to. A project is just a name: the
//! repo's folder name inside git (the main checkout's, so worktrees share it), or
//! the folder's own name outside git. `--project NAME` overrides it.

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Detected {
    pub name: String,
    pub path: String,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn detect(dir: &Path) -> Detected {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let root = git(&dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .and_then(|common| Path::new(&common).parent().map(Path::to_path_buf))
        .or_else(|| git(&dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from))
        .unwrap_or(dir);
    let name = root.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "root".into());
    Detected { name, path: root.to_string_lossy().to_string() }
}
