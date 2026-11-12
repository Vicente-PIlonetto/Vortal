# Architecture

The workspace separates the versioned protocol (`vortal-proto`), platform-neutral input and peer contracts (`vortal-input`), core state/layout (`vortal-core`), transport (`vortal-net`), and platform adapters. The daemon owns input and transport work; the UI remains a separate process.

Stage 1 defines contracts only. Later stages fill in the core, QUIC transport, VyLink adapter, Windows and Linux/Wayland backends, and daemon/CLI.
