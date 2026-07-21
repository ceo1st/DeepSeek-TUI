# v0.9.1 empty Work-surface receipt

This receipt verifies the empty-surface behavior from the real TUI binary. It
is not based on a product mockup.

## Source

- Version: `codewhale-tui 0.9.1 (5c3eb8245512)`
- Commit: `5c3eb8245512cf790a933484453d3e300eb4c7af`
- Branch at capture: `codex/fix-empty-work-surface-091`
- Terminal: `106x32`
- Route: isolated local Ollama configuration using `qwen3-coder`; no API key
- Workspace: clean public fixture shown as `~/codewhale-demo`
- Theme: Blue Stage dark

## Capture method

The binary was built with:

```bash
cargo build -p codewhale-tui --bin codewhale-tui --locked
```

It was launched in a 106-column by 32-row PTY with isolated state:

```bash
CODEWHALE_HOME=/tmp/codewhale-v091-receipt/home \
CODEWHALE_CONFIG_PATH=/tmp/codewhale-v091-receipt/home/config.toml \
target/debug/codewhale-tui \
  --skip-onboarding \
  --fresh \
  --no-project-config \
  --no-mouse-capture \
  --workspace /tmp/codewhale-v091-receipt/codewhale-demo
```

The resulting 1280x720 capture has SHA-256
`69c81df8a641cdad500d985973546db0a91c138e2c82e0de9586cdea7be85170`.
It contains no username, credential, account identifier, private repository
path, error state, or unsupported product claim.

## Acceptance

The fresh active session contains the header, idle canvas, composer, and
footer. It does not contain `Work · empty`, a Work heading, or reserved Work
rows. Active, error, and disconnected Work projections remain covered by the
TUI unit suite.

```text
cargo test -p codewhale-tui --bin codewhale-tui --locked
test result: ok. 7799 passed; 0 failed; 3 ignored

cargo test -p codewhale-tui --bin codewhale-tui --locked tui::work_surface::tests::
test result: ok. 23 passed; 0 failed
```
