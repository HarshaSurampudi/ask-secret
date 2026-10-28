//! `push`/`receive`: copy a project's secrets to another machine over SSH.
//!
//! Values travel on ssh's stdin, never in a command line or a file. The remote stores
//! them under the same project name, so a clone of the repo there finds them.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::vault::Vault;

#[derive(Serialize, Deserialize)]
struct Payload {
    project: String,
    secrets: BTreeMap<String, String>,
}

pub fn push(host: &str, project: Option<&str>, to: Option<&str>, keys: &[String]) -> Result<()> {
    let payload = {
        let mut vault = Vault::open()?;
        let id = vault.scope(project, false)?;
        let all = vault.resolved(&id);
        let secrets: BTreeMap<String, String> = if keys.is_empty() {
            all
        } else {
            let missing: Vec<&String> = keys.iter().filter(|k| !all.contains_key(*k)).collect();
            if !missing.is_empty() {
                bail!("not in this project: {}", missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
            }
            keys.iter().map(|k| (k.clone(), all[k].clone())).collect()
        };
        if secrets.is_empty() {
            bail!("project {id} has no secrets to push");
        }
        Payload { project: to.map(String::from).unwrap_or(id), secrets }
    };

    // Non-login shells often lack ~/.local/bin and ~/.cargo/bin on PATH.
    let remote = r#"PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH" ask-secret receive"#;
    let mut child = Command::new("ssh")
        .arg(host)
        .arg(remote)
        .stdin(Stdio::piped())
        .spawn()
        .context("starting ssh")?;
    {
        let mut stdin = child.stdin.take().unwrap();
        serde_json::to_writer(&mut stdin, &payload)?;
        stdin.write_all(b"\n")?;
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("the remote side failed (is ask-secret installed on {host}?)");
    }
    Ok(())
}

pub fn receive() -> Result<()> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        bail!("`receive` is what `ask-secret push` runs on the other machine; it reads from a pipe");
    }
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let p: Payload = serde_json::from_str(&text).context("unreadable payload")?;
    let mut vault = Vault::open()?;
    vault.ensure_project(&p.project, None, true);
    for (k, v) in &p.secrets {
        vault.set(&p.project, k, v);
    }
    vault.save()?;
    let host = Command::new("hostname").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let host = host.unwrap_or_default();
    for k in p.secrets.keys() {
        println!("{k}: saved on {host} (project {})", p.project);
    }
    Ok(())
}
