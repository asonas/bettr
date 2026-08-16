# bettr Claude Code adapter

This directory contains the Claude Code adapter for the local `bettr` CLI. It shares the JSON contract and capability matrix with `skills/bettr`.

Use the repository checkout during development. In a Claude Code environment, make the skill directory available through the project's skill loading mechanism, then verify:

```sh
command -v bettr
bettr capabilities --json
```

The adapter does not start a daemon or provide a network service. It requires a local initialized bettr database and uses `BETTR_AGENT=claude` plus a caller-generated `BETTR_SESSION_ID` for lease ownership.
