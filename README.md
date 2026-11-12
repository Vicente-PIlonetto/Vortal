# Vortal

Software KVM for sharing one mouse and keyboard across computers on a VyLink network.

## Status

Stage 1 foundation and protocol contracts. Platform input capture, network transport, and user interface are not implemented yet.

## Development

Install the pinned Rust toolchain with rustfmt and clippy, then run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [CONTRIBUTING.md](CONTRIBUTING.md), [architecture](docs/architecture.md), [protocol](docs/protocol.md), and [VyLink integration](docs/vylink-integration.md).
