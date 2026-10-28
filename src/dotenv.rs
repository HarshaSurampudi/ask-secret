//! Just enough dotenv: reading KEY=value files and upserting into them.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::Result;

/// (key, raw value) for an assignment line, or None for comments and blanks.
fn split_line(line: &str) -> Option<(String, String)> {
    let mut s = line.trim();
    if s.is_empty() || s.starts_with('#') {
        return None;
    }
    if let Some(rest) = s.strip_prefix("export ") {
        s = rest;
    }
    let (key, value) = s.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) {
        return None;
    }
    Some((key.to_string(), value.trim().to_string()))
}

fn unquote(raw: &str) -> String {
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return raw[1..raw.len() - 1].to_string();
    }
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let inner = &raw[1..raw.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some(o) => out.push(o),
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    // Unquoted: drop a trailing " # comment".
    raw.split(" #").next().unwrap_or("").trim().to_string()
}

pub fn parse(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(split_line)
        .map(|(k, v)| (k, unquote(&v)))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// Quote only when needed. Single quotes are literal in dotenv (no ${} interpolation).
fn encode(value: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_-./:@+=,".contains(c);
    if !value.is_empty() && value.chars().all(plain) {
        return value.to_string();
    }
    if !value.contains('\'') && !value.contains('\n') {
        return format!("'{value}'");
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    format!("\"{escaped}\"")
}

/// Replaces existing keys in place, appends new ones, keeps everything else, and
/// leaves the file at 0600.
pub fn upsert(path: &Path, values: &BTreeMap<String, String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(String::from).collect();
    let mut pending = values.clone();
    for line in lines.iter_mut() {
        let Some((key, _)) = split_line(line) else { continue };
        let Some(value) = pending.remove(&key) else { continue };
        let prefix = if line.trim_start().starts_with("export ") { "export " } else { "" };
        *line = format!("{prefix}{key}={}", encode(&value));
    }
    for (key, value) in pending {
        lines.push(format!("{key}={}", encode(&value)));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.ask-secret.tmp"));
    let mut f = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all((lines.join("\n") + "\n").as_bytes())?;
    fs::rename(&tmp, path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
