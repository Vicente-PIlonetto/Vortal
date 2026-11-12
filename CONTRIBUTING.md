# Contributing

## Requirements

- Rust 1.85.0 with `rustfmt` and `clippy` (installed from `rust-toolchain.toml`).
- Git.

## Local checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

On Windows use PowerShell or Command Prompt; the Cargo commands are the same. CI builds and tests on Windows and Ubuntu.

An optional pre-commit hook is available at `scripts/pre-commit.sh`. Install it with `ln -s ../../scripts/pre-commit.sh .git/hooks/pre-commit` from the repository root on Unix-like systems.

Use one branch per stage and Conventional Commits. Pull requests should explain changes, test steps, risks, and complete the checklist. Never commit keys, certificates, tokens, or local configuration.
