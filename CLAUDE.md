Read @AGENTS.md first: it is the single source of rules for this repo (shared with other agents).

Windows notes for Claude Code sessions:
- Rust lives in `%USERPROFILE%\.cargo\bin`; if `cargo` isn't found in PowerShell, prepend it to `$env:Path`.
- Pass JSON arguments to `dbcraft-cli` from the Bash tool, not PowerShell 5.1 (it strips the quotes).
- Use `DBCRAFT_WORKSPACE=dev/data/workspace.json` for every CLI/app run.
