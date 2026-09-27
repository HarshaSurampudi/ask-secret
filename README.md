# ask-secret

A native macOS popup that asks the user for one or more secrets and writes
them straight into a `.env` file (upsert, chmod 600) or the Keychain. The
calling process (e.g. Claude Code) only ever sees `KEY: saved` / `cancelled`.

```sh
swift build -c release && install -m 755 .build/release/ask-secret ~/.local/bin/
ask-secret --usage
```
