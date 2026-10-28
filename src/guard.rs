//! Leak guard: find exact copies of stored secrets in files or staged commits.
//!
//! Pattern scanners guess what looks like a key; this knows the actual values, so a
//! hit is a real leak.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::vault::Vault;

/// Very short values match too much ordinary text to be worth reporting.
const MIN_SCANNED: usize = 8;
const MAX_FILE: u64 = 5 * 1024 * 1024;

/// Prints `path:line: contains scope/KEY` for each hit. Returns the number of hits.
pub fn scan(files: &[PathBuf], staged: bool) -> Result<usize> {
    let values: Vec<(String, String)> = {
        let vault = Vault::open()?;
        vault.all_values().into_iter().filter(|(_, v)| v.len() >= MIN_SCANNED).collect()
    };
    let mut hits = 0;
    let mut check = |label: &str, text: &str| {
        for (n, line) in text.lines().enumerate() {
            for (name, value) in &values {
                if line.contains(value.as_str()) {
                    println!("{label}:{}: contains {name}", n + 1);
                    hits += 1;
                }
            }
        }
    };
    if staged {
        let out = Command::new("git")
            .args(["diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z"])
            .output()
            .context("running git")?;
        if !out.status.success() {
            bail!("not in a git repository");
        }
        for path in out.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
            let path = String::from_utf8_lossy(path);
            let blob = Command::new("git").args(["show", &format!(":{path}")]).output()?;
            if blob.stdout.len() as u64 <= MAX_FILE && !blob.stdout.contains(&0) {
                check(&path, &String::from_utf8_lossy(&blob.stdout));
            }
        }
    } else {
        for path in files {
            for file in walkdir::WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                if !file.file_type().is_file() || file.metadata().map(|m| m.len() > MAX_FILE).unwrap_or(true) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(file.path()) else { continue };
                if !bytes.contains(&0) {
                    check(&file.path().display().to_string(), &String::from_utf8_lossy(&bytes));
                }
            }
        }
    }
    Ok(hits)
}

const HOOK_MARK: &str = "ask-secret scan --staged";

/// Installs a pre-commit hook in the current repo that blocks commits containing a
/// stored secret.
pub fn install_hook(force: bool) -> Result<PathBuf> {
    let out = Command::new("git").args(["rev-parse", "--git-path", "hooks"]).output().context("running git")?;
    if !out.status.success() {
        bail!("not in a git repository");
    }
    let hooks = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    std::fs::create_dir_all(&hooks)?;
    let hook = hooks.join("pre-commit");
    let block = format!(
        "# ask-secret: block commits that contain a stored secret value.\n\
         # Only a hit (exit 1) blocks; if the check itself can't run, the commit goes ahead.\n\
         if command -v ask-secret >/dev/null 2>&1; then\n  \
         {HOOK_MARK}\n  \
         case $? in\n    \
         0) ;;\n    \
         1) echo 'ask-secret: commit blocked, a stored secret is in the staged files.' >&2; exit 1 ;;\n    \
         *) echo 'ask-secret: could not check for secrets, committing anyway.' >&2 ;;\n  \
         esac\n\
         fi\n"
    );
    let existing = std::fs::read_to_string(&hook).unwrap_or_default();
    if existing.contains(HOOK_MARK) {
        return Ok(hook);
    }
    let text = if existing.is_empty() {
        format!("#!/bin/sh\n{block}")
    } else if force {
        // Prepend after the shebang, so the check runs even if the old hook ends with exec.
        let (first, rest) = existing.split_once('\n').unwrap_or((&existing, ""));
        if first.starts_with("#!") { format!("{first}\n{block}{rest}") } else { format!("#!/bin/sh\n{block}{existing}") }
    } else {
        bail!("{} already exists; pass --force to add the check to it", hook.display());
    };
    std::fs::write(&hook, text)?;
    set_executable(&hook)?;
    Ok(hook)
}

fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}
