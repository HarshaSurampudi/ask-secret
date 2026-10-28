# ask-secret

A local secret vault for AI coding agents. The agent asks for a key, you paste it into
a popup, and the agent only ever learns `saved`. It then runs commands with the key in
their environment, and the key is masked in their output.

```sh
ask-secret ask --field OPENAI_API_KEY --help "From platform.openai.com/api-keys"
ask-secret run -- npm run dev        # key injected, and shown as •••• in output
```

## Install

```sh
cargo build --release
(cd macos && swift build -c release)        # the popup; macOS only
install -m 755 target/release/ask-secret macos/.build/release/ask-secret-ui ~/.local/bin/
ask-secret setup                            # teach your coding agents to use it
```

On Linux, skip the popup. ask-secret then asks in the terminal, or you can send secrets
from another machine with `ask-secret push`.

## Teaching your agents: `ask-secret setup`

Agents only follow rules they actually read. Skills and docs are read when the model
decides they're relevant, which isn't reliable, so `setup` puts a short rules block into
the instructions each agent loads in **every** session:

- **Claude Code**: `~/.claude/CLAUDE.md` (or under `CLAUDE_CONFIG_DIR`)
- **Codex**: `~/.codex/AGENTS.md` (or under `CODEX_HOME`). If you use
  `AGENTS.override.md`, the rules go there, because Codex then ignores `AGENTS.md`.

It shows what it will change and asks first. The block is marked, so running `setup`
again updates it in place and `ask-secret setup --remove` takes it out, leaving the
rest of your file untouched. Other options:

- `--file AGENTS.md`: write to a specific file instead, such as a repo's `AGENTS.md`
  (this is the way to set up Cursor, which has no global rules file).
- `--print`: just print the rules.
- `--yes`: apply without asking.

## What it does

- **Projects by name.** Secrets belong to the project named after the git repo (or
  folder) you're in. Worktrees share their repo's project. Use `--project NAME` for anything else.
- **Shared keys.** Save a key once to `shared`, and other projects link to it instead of
  keeping copies. The popup offers "use shared value" whenever one exists.
- **`run` with masking.** Secrets go into the child process's environment, not into a file, and
  any secret value it prints is replaced with `••••`.
- **Leak guard.** `ask-secret hook` adds a pre-commit hook that blocks commits containing
  any stored value. Exact matches, so there are no false positives.
- **Import.** `ask-secret import ~/Projects` moves existing `.env` files into the vault.
  A value that appears in several projects becomes one shared key.
- **Servers.** `ask-secret push HOST` copies a project's secrets over ssh stdin into
  the vault on another machine.
- **Paste checks, not format rules.** The checks warn about quotes, `KEY=` prefixes,
  whitespace and placeholder text. They never block saving.

## What it protects against

Accidental leaks: secrets landing in agent transcripts, logs, terminal scrollback,
`.env` files and commits. It can't stop a determined agent: anything that can run
`ask-secret run` can read what it injects (with `--no-mask`, or by encoding the value before printing it).

## Storage

One file, `~/.local/share/ask-secret/vault.enc`, encrypted with XChaCha20-Poly1305. The
key lives in the macOS Keychain, or in `vault.key` (0600) next to the vault on Linux.
Set `ASK_SECRET_HOME` / `ASK_SECRET_KEY_FILE` to move them.

## Pieces

- `src/`: the `ask-secret` CLI (Rust). It handles the vault, projects, `run`, scanning, import and push.
- `macos/`: `ask-secret-ui`, the SwiftUI popup. It reads a JSON spec on stdin and writes
  the answers to stdout, and stores nothing itself. Without it (Linux, or `ASK_SECRET_TTY=1`) the
  CLI asks in the terminal.
