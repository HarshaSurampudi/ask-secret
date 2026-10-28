//! `run`: start a command with the project's secrets in its environment, and mask
//! those values in whatever it prints.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

const MASK: &[u8] = b"\xe2\x80\xa2\xe2\x80\xa2\xe2\x80\xa2\xe2\x80\xa2"; // ••••
/// Short values would mask ordinary words, so they're injected but not masked.
const MIN_MASKED: usize = 6;
/// How long a possible start of a secret is held back while the program is quiet.
const IDLE_FLUSH: Duration = Duration::from_millis(300);

pub fn run(env: &BTreeMap<String, String>, command: &[String], mask: bool) -> Result<i32> {
    let mut cmd = Command::new(&command[0]);
    cmd.args(&command[1..]).envs(env);
    if !mask {
        let status = cmd.status().with_context(|| format!("starting {}", command[0]))?;
        return Ok(exit_code(status));
    }

    let mut values: Vec<Vec<u8>> =
        env.values().filter(|v| v.len() >= MIN_MASKED).map(|v| v.as_bytes().to_vec()).collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    values.dedup();

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", command[0]))?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let v1 = values.clone();
    let t1 = thread::spawn(move || pump(out, std::io::stdout(), &v1));
    let t2 = thread::spawn(move || pump(err, std::io::stderr(), &values));
    let status = child.wait()?;
    let _ = t1.join();
    let _ = t2.join();
    Ok(exit_code(status))
}

fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// Copies `src` to `dst`, replacing secret values. Reads happen on their own thread so
/// a quiet program still gets its last bytes flushed.
fn pump(mut src: impl Read + Send + 'static, mut dst: impl Write, values: &[Vec<u8>]) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match src.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut pending = vec![];
    loop {
        match rx.recv_timeout(IDLE_FLUSH) {
            Ok(chunk) => {
                pending.extend_from_slice(&chunk);
                let out = mask(&mut pending, values, false);
                let _ = dst.write_all(&out);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !pending.is_empty() {
                    let out = mask(&mut pending, values, true);
                    let _ = dst.write_all(&out);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let _ = dst.flush();
    }
    let out = mask(&mut pending, values, true);
    let _ = dst.write_all(&out);
    let _ = dst.flush();
}

/// Masks every complete value in `buf` and returns what's safe to print. Unless
/// `flush`, a tail that could be the start of a value stays in `buf`.
pub fn mask(buf: &mut Vec<u8>, values: &[Vec<u8>], flush: bool) -> Vec<u8> {
    let mut out = vec![];
    // `values` is longest first, so at equal positions the longest wins.
    while let Some((at, len)) = values
        .iter()
        .filter_map(|v| find(buf, v).map(|i| (i, v.len())))
        .min_by_key(|&(i, len)| (i, std::cmp::Reverse(len)))
    {
        out.extend_from_slice(&buf[..at]);
        out.extend_from_slice(MASK);
        buf.drain(..at + len);
    }
    let keep = if flush {
        0
    } else {
        let longest = values.first().map(|v| v.len()).unwrap_or(0);
        (1..longest.min(buf.len() + 1)).rev().find(|&k| values.iter().any(|v| v.starts_with(&buf[buf.len() - k..]))).unwrap_or(0)
    };
    let cut = buf.len() - keep;
    out.extend(buf.drain(..cut));
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_chunks(chunks: &[&str], values: &[&str]) -> String {
        let values: Vec<Vec<u8>> = values.iter().map(|v| v.as_bytes().to_vec()).collect();
        let mut pending = vec![];
        let mut out = vec![];
        for c in chunks {
            pending.extend_from_slice(c.as_bytes());
            out.extend(mask(&mut pending, &values, false));
        }
        out.extend(mask(&mut pending, &values, true));
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn masks_whole_and_split_values() {
        assert_eq!(run_chunks(&["key=sk-abc123 ok"], &["sk-abc123"]), "key=•••• ok");
        assert_eq!(run_chunks(&["key=sk-a", "bc123 ok"], &["sk-abc123"]), "key=•••• ok");
        assert_eq!(run_chunks(&["sk-abc12", "X"], &["sk-abc123"]), "sk-abc12X");
        assert_eq!(run_chunks(&["a sk-abc123sk-abc123 b"], &["sk-abc123"]), "a •••••••• b");
    }

    #[test]
    fn prefers_the_longest_value() {
        assert_eq!(run_chunks(&["x secret-long y"], &["secret-long", "secret"]), "x •••• y");
    }
}
