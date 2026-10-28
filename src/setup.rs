//! `setup`: put the agent rules into each coding agent's always-loaded instructions.
//!
//! A skill is only read when the model decides to; a global instructions file is in
//! every session. The rules go in a marked block, so running setup again updates it in
//! place and `--remove` takes it out without touching anything else.

use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const BEGIN: &str = "<!-- ask-secret:begin (managed by `ask-secret setup`; edits here are overwritten) -->";
const END: &str = "<!-- ask-secret:end -->";
const BEGIN_PREFIX: &str = "<!-- ask-secret:begin";

pub const RULES: &str = "\
## Secrets (ask-secret)

- **Never read secret files**: `.env`, `.env.*`, `secrets.*`, `credentials.json`, key files. No `cat`, `grep`, `head`, editor reads or anything else that shows their contents.
- **Never print environment variables**: no `env`, `printenv`, `echo $KEY`, or dumping `os.environ` / `process.env`.
- **Never ask the user to paste a secret into the chat.** When a task needs an API key, token or password, ask with `ask-secret`: the user types it into a popup and you only get `KEY: saved|linked|kept|skipped` or `cancelled`.
  - Ask for every key the task needs in **one** call, and tell the user in one line that a popup is waiting. Allow up to 10 minutes; they may need to create the key first.
    `ask-secret ask --field KEY --label \"...\" --help \"where to get it (markdown link)\" [--optional] [--field KEY2 ...]`
  - Exit 1 means the user cancelled: don't ask again unless they say so.
  - Secrets belong to the project named after the git repo folder you're in. Use `--project NAME` for another, or `--shared` for keys every project uses.
- **Use secrets through `ask-secret run -- <command>`**: it puts the project's secrets in the command's environment and shows their values as `••••` in its output. Only if a tool insists on a file: `ask-secret dotenv .env`, and check `.env` is gitignored.
- To check keys are set: `ask-secret status KEY...` prints OK or MISSING, never values.
";

struct Target {
    agent: &'static str,
    file: PathBuf,
    note: Option<String>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Agents found on this machine, and the global file each one always loads.
fn detect() -> (Vec<Target>, Vec<String>) {
    let mut targets = vec![];
    let mut notes = vec![];

    // Claude Code: ~/.claude/CLAUDE.md, or under CLAUDE_CONFIG_DIR.
    let claude_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".claude"));
    let claude_bin = on_path("claude");
    if claude_bin || claude_dir.is_dir() {
        let note = (!claude_bin).then(|| "`claude` isn't on PATH, but its config folder exists".to_string());
        targets.push(Target { agent: "Claude Code", file: claude_dir.join("CLAUDE.md"), note });
    } else {
        notes.push("Claude Code: not found (no `claude` on PATH, no ~/.claude).".into());
    }

    // Codex: $CODEX_HOME/AGENTS.md. A non-empty AGENTS.override.md replaces it entirely,
    // so the rules have to go there when it exists.
    let codex_dir = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".codex"));
    let codex_bin = on_path("codex");
    if codex_bin || codex_dir.is_dir() {
        let over = codex_dir.join("AGENTS.override.md");
        let uses_override = fs::read_to_string(&over).map(|s| !s.trim().is_empty()).unwrap_or(false);
        let mut note = (!codex_bin).then(|| "`codex` isn't on PATH, but its config folder exists".to_string());
        if uses_override {
            note = Some("AGENTS.override.md is in use, so Codex ignores AGENTS.md".into());
        }
        let file = if uses_override { over } else { codex_dir.join("AGENTS.md") };
        targets.push(Target { agent: "Codex", file, note });
    } else {
        notes.push("Codex: not found (no `codex` on PATH, no ~/.codex).".into());
    }

    if on_path("cursor") || home().join(".cursor").is_dir() {
        notes.push(
            "Cursor has no global rules file. In each repo, run `ask-secret setup --file AGENTS.md` (Cursor reads AGENTS.md).".into(),
        );
    }
    (targets, notes)
}

/// The text with the managed block replaced, added, or (with `None`) removed.
fn apply(text: &str, rules: Option<&str>) -> Result<String> {
    let begin = text.find(BEGIN_PREFIX);
    let end = text.find(END);
    let block = rules.map(|r| format!("{BEGIN}\n{}{END}\n", r));
    match (begin, end) {
        (Some(b), Some(e)) if e > b => {
            let after = &text[e + END.len()..];
            let after = after.strip_prefix('\n').unwrap_or(after);
            let mut out = text[..b].to_string();
            match &block {
                Some(block) => out.push_str(block),
                // Drop the blank line that separated the block from what came before.
                None => {
                    while out.ends_with("\n\n") {
                        out.pop();
                    }
                }
            }
            out.push_str(after);
            Ok(out)
        }
        (None, None) => Ok(match block {
            Some(block) if text.trim().is_empty() => block,
            Some(block) => format!("{}\n\n{block}", text.trim_end_matches('\n')),
            None => text.to_string(),
        }),
        _ => bail!("found only one of the ask-secret markers; fix the file by hand, then run setup again"),
    }
}

enum Plan {
    Create,
    Add,
    Update,
    Current,
    Remove,
    Absent,
}

fn plan_for(path: &Path, remove: bool) -> Result<(Plan, String, String)> {
    let existing = match fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let text = existing.clone().unwrap_or_default();
    let has_block = text.contains(BEGIN_PREFIX);
    let new = apply(&text, if remove { None } else { Some(RULES) })?;
    let plan = match (remove, existing.is_some(), has_block) {
        (true, _, true) => Plan::Remove,
        (true, _, false) => Plan::Absent,
        (false, false, _) => Plan::Create,
        (false, true, false) => Plan::Add,
        (false, true, true) if new == text => Plan::Current,
        (false, true, true) => Plan::Update,
    };
    Ok((plan, text, new))
}

pub fn run(files: &[PathBuf], remove: bool, yes: bool, print: bool) -> Result<u8> {
    if print {
        print!("{RULES}");
        return Ok(0);
    }

    let (mut targets, notes) = if files.is_empty() { detect() } else { (vec![], vec![]) };
    for f in files {
        targets.push(Target { agent: "file", file: f.clone(), note: None });
    }

    println!("{}", popup_status());
    for n in &notes {
        println!("{n}");
    }
    if targets.is_empty() {
        println!(
            "\nNo coding agent found. Add the rules to your agent's instructions by hand \
             (`ask-secret setup --print`), or point at a file: `ask-secret setup --file AGENTS.md`."
        );
        return Ok(0);
    }

    // Follow symlinks so a linked instructions file is edited where it really lives.
    let mut work = vec![];
    println!();
    for t in &targets {
        let path = fs::canonicalize(&t.file).unwrap_or_else(|_| t.file.clone());
        let (plan, old, new) = plan_for(&path, remove)?;
        let what = match plan {
            Plan::Create => "will create it with the rules",
            Plan::Add => "will add the rules at the end",
            Plan::Update => "will update the rules",
            Plan::Current => "rules already up to date",
            Plan::Remove => "will remove the rules",
            Plan::Absent => "no rules to remove",
        };
        let shown = if path != t.file { format!("{} → {}", t.file.display(), path.display()) } else { t.file.display().to_string() };
        println!("{}: {shown}\n  {what}", t.agent);
        if let Some(n) = &t.note {
            println!("  note: {n}");
        }
        if new.len() > 32 * 1024 && t.agent == "Codex" {
            println!("  note: this file is over 32 KiB; Codex stops reading instructions at 32 KiB by default");
        }
        if matches!(plan, Plan::Create | Plan::Add | Plan::Update | Plan::Remove) {
            work.push((path, old, new));
        }
    }
    if work.is_empty() {
        println!("\nNothing to change.");
        return Ok(0);
    }

    if !yes {
        if !std::io::stdin().is_terminal() {
            println!("\nNot changed: run again with --yes to apply.");
            return Ok(1);
        }
        print!("\nApply? [Y/n] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer)?;
        if answer.trim().eq_ignore_ascii_case("n") {
            println!("Not changed.");
            return Ok(1);
        }
    }
    for (path, _old, new) in &work {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("ask-secret.tmp");
        fs::write(&tmp, new).with_context(|| format!("writing {}", path.display()))?;
        // Keep the original file's permissions.
        if let Ok(meta) = fs::metadata(path) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
        println!("{}: done", path.display());
    }
    if !remove {
        println!("New sessions pick this up; restart any agent that's already running.");
    }
    Ok(0)
}

fn popup_status() -> String {
    let sibling = std::env::current_exe().ok().map(|p| p.with_file_name("ask-secret-ui"));
    let found = sibling.is_some_and(|p| p.exists()) || on_path("ask-secret-ui");
    match (cfg!(target_os = "macos"), found) {
        (true, true) => "Popup: ask-secret-ui found.".into(),
        (true, false) => "Popup: ask-secret-ui not found, so ask-secret will prompt in the terminal. \
                          Install it next to ask-secret to get the popup."
            .into(),
        (false, _) => "Popup: none on this system yet; ask-secret prompts in the terminal. On a server with no \
                       terminal, send secrets from your own machine with `ask-secret push HOST`."
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_updates_and_removes_the_block() {
        let user = "# Mine\n\n- keep me\n";
        let added = apply(user, Some("RULES v1\n")).unwrap();
        assert!(added.starts_with(user.trim_end()));
        assert!(added.contains("RULES v1"));

        let updated = apply(&added, Some("RULES v2\n")).unwrap();
        assert!(updated.contains("RULES v2") && !updated.contains("RULES v1"));
        assert_eq!(updated.matches(BEGIN_PREFIX).count(), 1);

        let removed = apply(&updated, None).unwrap();
        assert_eq!(removed.trim_end(), user.trim_end());
    }

    #[test]
    fn block_in_the_middle_keeps_what_follows() {
        let text = format!("top\n\n{BEGIN}\nold\n{END}\n\n## After\nmore\n");
        let out = apply(&text, Some("new\n")).unwrap();
        assert!(out.contains("new\n") && out.contains("## After\nmore"));
        let gone = apply(&text, None).unwrap();
        assert_eq!(gone, "top\n\n## After\nmore\n");
    }

    #[test]
    fn half_a_block_is_an_error() {
        assert!(apply(&format!("x\n{BEGIN}\nold\n"), Some("new\n")).is_err());
    }

    #[test]
    fn empty_file_gets_just_the_block() {
        assert_eq!(apply("", Some("r\n")).unwrap(), format!("{BEGIN}\nr\n{END}\n"));
    }
}
