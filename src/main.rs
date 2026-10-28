//! ask-secret: a local secret vault for AI agents. An agent asks for a secret, the
//! user types it into a popup, and the agent only ever learns whether it was saved.

mod dotenv;
mod guard;
mod import;
mod mask;
mod project;
mod prompt;
mod remote;
mod vault;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use prompt::{Answer, Field, Response, Spec};
use vault::{Entry, Vault, SHARED};

#[derive(Parser)]
#[command(name = "ask-secret", version, about = "A secret vault for AI agents: they ask, you type, they never see the value.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Ask the user for secrets in a popup and store them (see `ask-secret ask --usage`)
    Ask {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run a command with this project's secrets in its environment, masking them in its output
    Run {
        #[arg(long)]
        project: Option<String>,
        /// Pass output straight through (keeps colors; nothing is masked)
        #[arg(long)]
        no_mask: bool,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Show which keys are set, never their values. With keys, exit 1 if any is missing
    Status {
        #[arg(long)]
        project: Option<String>,
        keys: Vec<String>,
    },
    /// List projects and their key names
    Ls,
    /// Delete a key from this project (or from shared)
    Rm {
        key: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        shared: bool,
    },
    /// Move this project's value for KEY into shared and link to it
    Share {
        key: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Link KEY in this project to the shared value
    Link {
        key: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Rename a project (say, after renaming its folder)
    Rename { from: String, to: String },
    /// Write this project's secrets to a .env file (for tools that insist on one)
    Dotenv {
        file: PathBuf,
        #[arg(long)]
        project: Option<String>,
    },
    /// Import .env files under DIR into the vault (dry run unless --yes)
    Import {
        dir: PathBuf,
        #[arg(long)]
        yes: bool,
        /// Delete the .env files after importing
        #[arg(long, requires = "yes")]
        delete: bool,
        #[arg(long, default_value_t = 4)]
        depth: usize,
    },
    /// Look for stored secret values in files, or in staged changes. Exit 1 on a hit
    Scan {
        #[arg(long)]
        staged: bool,
        files: Vec<PathBuf>,
    },
    /// Install a git pre-commit hook in this repo that runs `scan --staged`
    Hook {
        #[arg(long)]
        force: bool,
    },
    /// Copy this project's secrets (or just KEYS) to HOST over ssh
    Push {
        host: String,
        keys: Vec<String>,
        #[arg(long)]
        project: Option<String>,
        /// Store them under a different project name on HOST
        #[arg(long)]
        to: Option<String>,
    },
    #[command(hide = true)]
    Receive,
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    // The original interface was `ask-secret --env FILE --field KEY ...`; keep it working.
    if args.len() > 1 && args[1].starts_with('-') && !["-h", "--help", "-V", "--version"].contains(&args[1].as_str()) {
        args.insert(1, "ask".into());
    }
    let cli = Cli::parse_from(args);
    match dispatch(cli.cmd) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("ask-secret: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(cmd: Cmd) -> Result<u8> {
    match cmd {
        Cmd::Ask { args } => ask(args),
        Cmd::Run { project, no_mask, command } => {
            let env = {
                let mut v = Vault::open()?;
                let id = v.scope(project.as_deref(), false)?;
                v.resolved(&id)
            };
            Ok(mask::run(&env, &command, !no_mask)?.clamp(0, 255) as u8)
        }
        Cmd::Status { project, keys } => status(project.as_deref(), &keys),
        Cmd::Ls => ls(),
        Cmd::Rm { key, project, shared } => {
            let mut v = Vault::open()?;
            let id = if shared { SHARED.to_string() } else { v.scope(project.as_deref(), false)? };
            if !v.remove(&id, &key) {
                bail!("{key} is not in {}", id);
            }
            v.save()?;
            println!("{key}: removed from {}", id);
            Ok(0)
        }
        Cmd::Share { key, project } => {
            let mut v = Vault::open()?;
            let id = v.scope(project.as_deref(), false)?;
            let Some(Entry::Value { value, .. }) = v.entry(&id, &key).cloned() else {
                bail!("{key} has no value of its own in {}", id);
            };
            if v.get(SHARED, &key).is_some_and(|s| s != value) {
                bail!("shared already has a different {key}; use `link` to switch to it, or `rm --shared` first");
            }
            v.set(SHARED, &key, &value);
            v.link(&id, &key);
            v.save()?;
            println!("{key}: now shared, linked from {}", id);
            Ok(0)
        }
        Cmd::Link { key, project } => {
            let mut v = Vault::open()?;
            let id = v.scope(project.as_deref(), true)?;
            if v.get(SHARED, &key).is_none() {
                bail!("shared has no {key}");
            }
            v.link(&id, &key);
            v.save()?;
            println!("{key}: linked to shared in {}", id);
            Ok(0)
        }
        Cmd::Rename { from, to } => {
            let mut v = Vault::open()?;
            v.rename(&from, &to)?;
            v.save()?;
            println!("renamed {from} to {to}");
            Ok(0)
        }
        Cmd::Dotenv { file, project } => {
            let mut v = Vault::open()?;
            let id = v.scope(project.as_deref(), false)?;
            let values = v.resolved(&id);
            if values.is_empty() {
                bail!("{} has no secrets", id);
            }
            dotenv::upsert(&file, &values)?;
            println!("wrote {} key(s) to {}", values.len(), file.display());
            Ok(0)
        }
        Cmd::Import { dir, yes, delete, depth } => {
            import::import(&dir, depth, yes, delete)?;
            Ok(0)
        }
        Cmd::Scan { staged, files } => {
            let files = if files.is_empty() && !staged { vec![PathBuf::from(".")] } else { files };
            let hits = guard::scan(&files, staged)?;
            Ok(if hits > 0 { 1 } else { 0 })
        }
        Cmd::Hook { force } => {
            let path = guard::install_hook(force)?;
            println!("pre-commit hook ready: {}", path.display());
            Ok(0)
        }
        Cmd::Push { host, keys, project, to } => {
            remote::push(&host, project.as_deref(), to.as_deref(), &keys)?;
            Ok(0)
        }
        Cmd::Receive => {
            remote::receive()?;
            Ok(0)
        }
    }
}

// MARK: ask

const ASK_USAGE: &str = "\
ask-secret ask: ask the user for secrets in a popup and store them in the vault,
without the values passing through the caller.

USAGE
  ask-secret ask [--project NAME | --shared] [--env FILE] [--title T] [--message M]
                 --field KEY [--label L] [--help H] [--pattern REGEX] [--hint H]
                             [--placeholder P] [--optional]
                 [--field KEY2 ...] [--timeout SECONDS]

  The project is named after the git repo (or folder) you're in.
  --project NAME   Store in this project instead (created if new).
  --shared         Store in shared, for use by any project.
  --env FILE       Also write the values to FILE as KEY=value (0600).
  --field KEY      Starts a field; the options after it describe that field.
  --help H         Markdown help under the field; links are clickable.
  --pattern REGEX  Expected format. A mismatch only warns; it never blocks saving.
  --hint H         Warning shown when the pattern doesn't match.
  --optional       The field may be left empty.

OUTPUT (stdout, never values)
  KEY: saved | linked (uses the shared value) | kept (already set) | skipped
  or `cancelled`. Exit 0 = saved, 1 = cancelled or timed out, 2 = usage error.
";

struct AskArgs {
    title: Option<String>,
    message: Option<String>,
    env_file: Option<PathBuf>,
    project: Option<String>,
    shared: bool,
    timeout: u64,
    fields: Vec<Field>,
}

fn parse_ask(args: &[String]) -> Result<AskArgs> {
    let mut a = AskArgs { title: None, message: None, env_file: None, project: None, shared: false, timeout: 900, fields: vec![] };
    let mut i = 0;
    let value = |i: &mut usize, flag: &str| -> Result<String> {
        *i += 1;
        args.get(*i).cloned().ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))
    };
    while i < args.len() {
        let flag = args[i].as_str();
        let field = a.fields.len().checked_sub(1);
        let need_field = || field.ok_or_else(|| anyhow::anyhow!("{flag} must follow --field"));
        match flag {
            "--env" => a.env_file = Some(expand_tilde(&value(&mut i, flag)?)),
            "--project" => a.project = Some(value(&mut i, flag)?),
            "--shared" => a.shared = true,
            "--title" => a.title = Some(value(&mut i, flag)?),
            "--message" => a.message = Some(value(&mut i, flag)?),
            "--timeout" => a.timeout = value(&mut i, flag)?.parse().map_err(|_| anyhow::anyhow!("--timeout needs seconds"))?,
            "--keychain" => bail!("--keychain was removed: secrets now go to the vault (use `ask-secret run` to use them)"),
            "--field" => {
                let key = value(&mut i, flag)?;
                let valid = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
                if !valid {
                    bail!("invalid key {key}");
                }
                a.fields.push(Field {
                    label: key.clone(),
                    key,
                    help: None,
                    pattern: None,
                    hint: None,
                    placeholder: None,
                    optional: false,
                    existing: false,
                    shared_available: false,
                });
            }
            "--label" => { let n = need_field()?; a.fields[n].label = value(&mut i, flag)?; }
            "--help" => { let n = need_field()?; a.fields[n].help = Some(value(&mut i, flag)?); }
            "--pattern" => { let n = need_field()?; a.fields[n].pattern = Some(value(&mut i, flag)?); }
            "--hint" => { let n = need_field()?; a.fields[n].hint = Some(value(&mut i, flag)?); }
            "--placeholder" => { let n = need_field()?; a.fields[n].placeholder = Some(value(&mut i, flag)?); }
            "--optional" => { let n = need_field()?; a.fields[n].optional = true; }
            "-h" | "--usage" => bail!("{ASK_USAGE}"),
            other => bail!("unknown option {other}\n\n{ASK_USAGE}"),
        }
        i += 1;
    }
    if a.fields.is_empty() {
        bail!("at least one --field is required\n\n{ASK_USAGE}");
    }
    if a.shared && a.project.is_some() {
        bail!("pass --shared or --project, not both");
    }
    Ok(a)
}

fn expand_tilde(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(p),
    }
}

fn ask(args: Vec<String>) -> Result<u8> {
    if args.iter().any(|a| a == "--usage" || a == "-h") {
        print!("{ASK_USAGE}");
        return Ok(0);
    }
    let mut a = parse_ask(&args)?;

    // Look up what exists, then let go of the vault while the user takes their time.
    let scope = {
        let mut v = Vault::open()?;
        // `true` so a new --project name works; nothing is saved until the answers come back.
        let scope = if a.shared { SHARED.to_string() } else { v.scope(a.project.as_deref(), true)? };
        for f in a.fields.iter_mut() {
            f.existing = v.get(&scope, &f.key).is_some();
            f.shared_available = scope != SHARED && v.get(SHARED, &f.key).is_some();
        }
        scope
    };
    let spec = Spec {
        title: a.title.clone().unwrap_or_else(|| {
            if a.fields.len() == 1 { a.fields[0].label.clone() } else { "Secrets needed".into() }
        }),
        message: a.message.clone(),
        destination: if scope == SHARED { "Vault · shared".into() } else { format!("Vault · {scope}") },
        can_share: scope != SHARED,
        timeout: a.timeout,
        fields: a.fields.clone(),
    };

    let answers = match prompt::collect(&spec)? {
        Response::Cancelled { reason } => {
            println!("{}", if reason.is_empty() { "cancelled".to_string() } else { format!("cancelled ({reason})") });
            return Ok(1);
        }
        Response::Saved { fields } => fields,
    };

    let mut v = Vault::open()?;
    let scope = if scope == SHARED { scope } else { v.scope(a.project.as_deref(), true)? };
    let mut report = vec![];
    for f in &a.fields {
        let outcome = match answers.get(&f.key) {
            Some(Answer::Value { value, to_shared }) if *to_shared && scope != SHARED => {
                v.set(SHARED, &f.key, value);
                v.link(&scope, &f.key);
                "saved (shared)"
            }
            Some(Answer::Value { value, .. }) => {
                v.set(&scope, &f.key, value);
                "saved"
            }
            Some(Answer::Link) if v.get(SHARED, &f.key).is_some() => {
                v.link(&scope, &f.key);
                "linked (uses the shared value)"
            }
            Some(Answer::Keep) | Some(Answer::Link) if v.get(&scope, &f.key).is_some() => "kept (already set)",
            _ => "skipped",
        };
        report.push((f.key.clone(), outcome));
    }
    v.save()?;

    if let Some(file) = &a.env_file {
        let resolved: BTreeMap<String, String> =
            a.fields.iter().filter_map(|f| Some((f.key.clone(), v.get(&scope, &f.key)?.to_string()))).collect();
        dotenv::upsert(file, &resolved)?;
    }
    for (key, outcome) in report {
        println!("{key}: {outcome}");
    }
    let mut stored = format!("stored in: vault · {}", scope);
    if let Some(file) = &a.env_file {
        stored += &format!(" (and {})", file.display());
    }
    println!("{stored}");
    Ok(0)
}

// MARK: status, ls

fn status(project: Option<&str>, keys: &[String]) -> Result<u8> {
    let mut v = Vault::open()?;
    let id = v.scope(project, false)?;
    let describe = |key: &str| -> &'static str {
        match v.entry(&id, key) {
            Some(Entry::Value { .. }) => "OK",
            Some(Entry::Link) if v.get(SHARED, key).is_some() => "OK (shared)",
            Some(Entry::Link) => "MISSING (linked, but shared has no value)",
            None => "MISSING",
        }
    };
    if keys.is_empty() {
        let names: Vec<String> = v.scopes().find(|(sid, _)| **sid == id).map(|(_, s)| s.secrets.keys().cloned().collect()).unwrap_or_default();
        println!("{id}");
        if names.is_empty() {
            println!("  no secrets yet");
        }
        for k in names {
            println!("  {k}: {}", describe(&k));
        }
        return Ok(0);
    }
    let mut missing = false;
    for k in keys {
        let d = describe(k);
        missing |= d.starts_with("MISSING");
        println!("{k}: {d}");
    }
    Ok(if missing { 1 } else { 0 })
}

fn ls() -> Result<u8> {
    let v = Vault::open()?;
    for (id, s) in v.scopes() {
        if id == SHARED && s.secrets.is_empty() {
            continue;
        }
        let keys: Vec<String> = s
            .secrets
            .iter()
            .map(|(k, e)| if matches!(e, Entry::Link) { format!("{k}→shared") } else { k.clone() })
            .collect();
        let place = s.path.as_deref().map(|p| format!("  {p}")).unwrap_or_default();
        println!("{id}{place}\n  {}", if keys.is_empty() { "(empty)".into() } else { keys.join(", ") });
    }
    Ok(0)
}
