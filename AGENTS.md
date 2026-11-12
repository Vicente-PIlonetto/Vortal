# Vortal contributor instructions

## Engineering rules

- Use Rust 2021 or newer in the Cargo workspace. Async code uses Tokio, logs use `tracing`, library errors use `thiserror`, and binaries use `anyhow`.
- Do not use `unwrap()` or `expect()` in production code. Keep `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` clean.
- Platform-specific behavior stays behind traits. `vortal-core` must not import Windows or Wayland implementation code.
- Fail safely: network, daemon, or backend failures must restore local input in under 500 ms. Never leave a user without local mouse or keyboard control.
- Never commit secrets, keys, certificates, tokens, or local configuration.
- Preserve the standalone VyLink repository boundary. Integrate through a dependency/API; never copy its code or use a Git submodule.

## Git workflow

- Keep `main` stable and protected; merge work through pull requests with green CI.
- Use one short-lived branch per stage: `feat/`, `fix/`, `docs/`, `chore/`, or `refactor/`.
- Use Conventional Commits. Scope names omit the `vortal-` crate prefix.
- Use SemVer, starting at `0.1.0`; derive the changelog from commits. Tags `vX.Y.Z` trigger releases.
- PR descriptions cover changes, test steps, risks, and a checklist.

## Interface design

- Use a monochrome, dark-first Windows Modern UI tile layout. Use grayscale tokens only; define colors in one token file and never hardcode colors elsewhere.
- Keep surfaces fully flat: no shadows, gradients, decorative transparency, or rounded corners.
- Keep UI copy concise, with no explanatory paragraphs. All UI text belongs in `pt-BR` and `en` localization files.
- Animate only `transform` and `opacity`; honor `prefers-reduced-motion`. Keep keyboard focus visible and support keyboard navigation.
- Keep UI rendering separate from the input daemon so rendering failures cannot affect input latency.
