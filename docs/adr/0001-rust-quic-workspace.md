# ADR 0001: Rust workspace, QUIC transport, and platform traits

- Status: Accepted
- Date: 2026-11-04

## Context

Vortal needs low-latency input transport across Windows and Linux/Wayland, strict recovery of local input on failure, and a path to additional platform backends. The project must also keep protocol definitions testable without hardware or I/O.

## Decision

Use a Rust 2021 Cargo workspace. Keep the core and wire protocol platform-neutral, and put operating-system behavior behind traits and dedicated crates. Use QUIC with rustls for the network transport, subject to Stage 3 validation. Use `PeerDirectory` as the discovery and membership boundary so VyLink, manual IP, and mDNS can be separate implementations.

## Consequences

Shared types and contracts can be tested independently of the OS. Dedicated crates constrain platform dependencies. QUIC provides a common authenticated transport with reliable streams and datagrams, while transport policy and performance still require measurement. The VyLink repository is currently Go, so Stage 3 must use a supported API/client boundary rather than assuming a Rust crate exists.
