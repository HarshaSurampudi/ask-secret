//! Collects values from the user: the native popup when there is one, else the terminal.
//!
//! The popup (`ask-secret-ui`) gets a JSON spec on stdin and answers with JSON on
//! stdout. Values only pass through that pipe, never through the caller.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub optional: bool,
    /// The target scope already has a value; leaving it empty keeps it.
    pub existing: bool,
    /// `shared` has this key, so the user can link to it instead of typing.
    pub shared_available: bool,
}

#[derive(Serialize)]
pub struct Spec {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Shown under the title, e.g. "Vault · rishta".
    pub destination: String,
    /// Offer "also save to shared" for new values (off when the target is shared).
    pub can_share: bool,
    pub timeout: u64,
    pub fields: Vec<Field>,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Response {
    Saved { fields: BTreeMap<String, Answer> },
    Cancelled {
        #[serde(default)]
        reason: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Answer {
    Value {
        value: String,
        #[serde(default)]
        to_shared: bool,
    },
    Link,
    Keep,
    Skip,
}

pub fn collect(spec: &Spec) -> Result<Response> {
    if std::env::var_os("ASK_SECRET_TTY").is_none() {
        if let Some(ui) = find_ui() {
            if gui_available() {
                return popup(&ui, spec);
            }
        }
    }
    terminal(spec)
}

fn find_ui() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ASK_SECRET_UI") {
        return Some(PathBuf::from(p));
    }
    let sibling = std::env::current_exe().ok()?.with_file_name("ask-secret-ui");
    if sibling.exists() {
        return Some(sibling);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).map(|d| d.join("ask-secret-ui")).find(|p| p.exists())
    })
}

fn gui_available() -> bool {
    if cfg!(target_os = "macos") {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn popup(ui: &PathBuf, spec: &Spec) -> Result<Response> {
    let mut child = Command::new(ui)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("starting {}", ui.display()))?;
    {
        let mut stdin = child.stdin.take().unwrap();
        serde_json::to_writer(&mut stdin, spec)?;
        stdin.write_all(b"\n")?;
    }
    let out = child.wait_with_output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str(text.trim()) {
        Ok(r) => Ok(r),
        Err(_) if !out.status.success() => Ok(Response::Cancelled { reason: "the popup closed".into() }),
        Err(e) => Err(e).context("the popup returned something unreadable"),
    }
}

// MARK: terminal fallback

fn terminal(spec: &Spec) -> Result<Response> {
    let tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty");
    let Ok(tty) = tty else {
        bail!(
            "no popup and no terminal to ask in. On a headless machine, send the secret from a \
             machine that has it: `ask-secret push <host> KEY`"
        );
    };
    let mut out = tty.try_clone()?;
    let mut input = BufReader::new(tty);
    let mut line = |prompt: &str, out: &mut std::fs::File| -> Result<String> {
        write!(out, "{prompt}")?;
        out.flush()?;
        let mut s = String::new();
        input.read_line(&mut s)?;
        Ok(s.trim().to_string())
    };

    writeln!(out, "\n{} ({})", spec.title, spec.destination)?;
    if let Some(m) = &spec.message {
        writeln!(out, "{m}")?;
    }
    let mut answers = BTreeMap::new();
    let mut entered: Vec<(String, String)> = vec![];
    for f in &spec.fields {
        writeln!(out, "\n{} [{}]", f.label, f.key)?;
        if let Some(h) = &f.help {
            writeln!(out, "  {h}")?;
        }
        if f.shared_available && !f.existing {
            let choice = line("  Use the shared value? [Y/n] ", &mut out)?;
            if !choice.eq_ignore_ascii_case("n") {
                answers.insert(f.key.clone(), Answer::Link);
                continue;
            }
        }
        loop {
            let hint = if f.existing { " (empty keeps the current value)" } else if f.optional { " (optional)" } else { "" };
            let value = rpassword::prompt_password(format!("  Value{hint}: "))?;
            let value = value.trim().to_string();
            if value.is_empty() {
                if f.existing {
                    answers.insert(f.key.clone(), Answer::Keep);
                    break;
                }
                if f.optional {
                    answers.insert(f.key.clone(), Answer::Skip);
                    break;
                }
                writeln!(out, "  This one is required.")?;
                continue;
            }
            let warnings = warnings(f, &value, &entered);
            if !warnings.is_empty() {
                for w in &warnings {
                    writeln!(out, "  ! {w}")?;
                }
                if !line("  Save it anyway? [y/N] ", &mut out)?.eq_ignore_ascii_case("y") {
                    continue;
                }
            }
            let to_shared = spec.can_share
                && line("  Also make it shared, for other projects? [y/N] ", &mut out)?.eq_ignore_ascii_case("y");
            entered.push((f.key.clone(), value.clone()));
            answers.insert(f.key.clone(), Answer::Value { value, to_shared });
            break;
        }
    }
    Ok(Response::Saved { fields: answers })
}

/// Paste mistakes worth a second look. None of these block saving. The popup has
/// its own copy of these checks.
pub fn warnings(field: &Field, value: &str, others: &[(String, String)]) -> Vec<String> {
    let mut w = vec![];
    let quoted = value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\'')));
    if quoted {
        w.push("It's wrapped in quotes; the quotes will be part of the value.".into());
    }
    if value.starts_with(&format!("{}=", field.key)) {
        w.push(format!("It starts with \"{}=\"; paste only the value.", field.key));
    }
    if value.chars().any(char::is_whitespace) {
        w.push("It contains spaces or line breaks.".into());
    }
    let lower = value.to_lowercase();
    if ["your-", "your_", "<", "xxxx", "changeme", "placeholder", "example"].iter().any(|p| lower.contains(p)) {
        w.push("It looks like placeholder text.".into());
    }
    if let Some((other, _)) = others.iter().find(|(_, v)| v == value) {
        w.push(format!("It's the same as {other}."));
    }
    if let Some(p) = &field.pattern {
        if !pattern_matches(p, value) {
            w.push(field.hint.clone().unwrap_or_else(|| "It doesn't look like the usual format.".into()));
        }
    }
    w
}

/// Whole-value match. A pattern that doesn't compile is ignored.
fn pattern_matches(pattern: &str, value: &str) -> bool {
    regex::Regex::new(&format!("^(?:{pattern})$")).map(|r| r.is_match(value)).unwrap_or(true)
}
