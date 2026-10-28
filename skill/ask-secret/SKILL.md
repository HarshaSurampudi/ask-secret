---
name: ask-secret
description: Get API keys, tokens and passwords from the user without ever seeing them, and use them without printing them. Use whenever a task needs a secret (an API key, a token, a DB URL with a password), when a command fails for a missing credential, or before reading any .env or credentials file.
---

# ask-secret

`ask-secret` is a local vault. You ask for a secret, the user types it into a popup,
and you only learn whether it was saved. You then run commands with the secret in
their environment, and the values are masked in their output.

## Rules

- **Never read secret files**: `.env`, `.env.*`, `credentials.json`, `secrets.*`, key files.
  No `cat`, `grep`, `head`, `Read` or anything else that would show their contents.
- **Never print environment variables**: no `env`, `printenv`, `echo $KEY`, or dumping `os.environ`.
- **Never ask the user to paste a secret into the chat.** Ask with `ask-secret` instead.
- To check whether keys are set, use `ask-secret status KEY...`. It prints OK or MISSING, never values.

## Asking for secrets

Ask for everything the task needs in **one** call. Tell the user in one line that a
popup is waiting. Give the command a long timeout (up to 10 minutes), since the user
may have to go and create the key.

```sh
ask-secret ask \
  --field OPENAI_API_KEY --label "OpenAI API key" \
    --help "Create one at [platform.openai.com/api-keys](https://platform.openai.com/api-keys)" \
  --field DATABASE_URL --label "Postgres URL" --help "Neon dashboard → Connection string" \
  --field SENTRY_DSN --optional
```

- The project is named after the git repo (or folder) you're in. Add `--project NAME` to
  pick another, or `--shared` for keys every project should use.
- If shared already has the key, the popup offers to reuse it, so the user doesn't have to paste it again.
- `--help` takes markdown, so say exactly where the value comes from. `--pattern` only warns.
- Output: `KEY: saved | saved (shared) | linked | kept | skipped`, or `cancelled`.
  Exit 0 means saved, 1 cancelled (don't ask again unless the user asks), 2 a usage error.

## Using secrets

Prefer running through the vault over writing files:

```sh
ask-secret run -- npm run dev
ask-secret run -- python scripts/backfill.py
```

The command gets this project's secrets as environment variables. Any secret value in
its output shows up as `••••`. Use `--no-mask` only when a tool needs a real terminal.

If a tool insists on a `.env` file: `ask-secret dotenv .env` (written 0600). Check that
`.env` is gitignored.

## Other commands

- `ask-secret status`: the keys in this project and whether each is set.
- `ask-secret ls`: projects and key names.
- `ask-secret push HOST [KEY...]`: copy this project's secrets to another machine over
  ssh (it needs ask-secret installed there). Use this for agents running on a server.
- `ask-secret scan --staged` / `ask-secret hook`: find, or block commits of, stored
  secret values.
- `ask-secret import DIR`: move existing .env files into the vault (dry run unless `--yes`).
  Suggest it to the user; don't run it with `--yes` or `--delete` unasked.
