# VyLink integration

## Repository findings

VyLink is maintained separately at [VyktorCorner/VyLink](https://github.com/VyktorCorner/VyLink). Its current implementation is Go and its local network coordinator stores a network snapshot containing a gateway and enrolled devices. The public device record exposes `name`, `publicKey`, `address`, `connected`, `online`, and `lastSeen`; revoked devices are excluded. Online status is computed from the connected flag and a heartbeat less than 45 seconds old.

## Current integration surface

The desktop app keeps enrollment metadata locally (`server`, bearer `token`, and network name) and polls the authenticated `POST /v1/snapshot` endpoint. The endpoint responds with a `network` snapshot whose `gateway` and `devices` entries include the fields above. Vortal can use this API when it has the user's authorized VyLink enrollment credentials. Credentials must stay in the OS credential store and must never be logged or written into Vortal configuration in plaintext.

The per-device tunnel config inspector (`InspectConfig` in `internal/tunnel/config.go`) returns public metadata (`address`, public key, and peer addresses), but the current local daemon does not expose a stable public IPC API for third-party clients to query active interface state. A standalone imported WireGuard config may therefore provide a manual peer list, but does not provide presence status.

## Mapping to `PeerDirectory`

| PeerDirectory field | VyLink field | Notes |
| --- | --- | --- |
| `device_id` | `publicKey` | Use the WireGuard public key as stable identity. |
| `device_name` | `name` | Display label only; not an authorization identity. |
| `public_key` | `publicKey` | Must match the enrolled network member. |
| `address` | `address` | Private overlay IP in CIDR notation; normalize to a socket address using the Vortal transport port. |
| `online` | `online` | Heartbeat-based status, not proof that Vortal is listening. |

Vortal must independently authenticate each connection and verify the remote identity against the current member list. VyLink membership is the allowlist input; it does not replace Vortal's transport authentication.

## Planned adapter

`vortal-vylink` implements the platform-neutral `PeerDirectory` trait. Stage 1 records the observed boundary only; it does not fabricate a Rust dependency for a Go repository or copy VyLink implementation code. Stage 3 should consume either a supported VyLink library release once one exists, or the authenticated snapshot API through a small Rust HTTP client. Do not use a Git submodule.

Manual IP and mDNS discovery remain independent fallback `PeerDirectory` implementations. They do not inherit VyLink's membership authorization and must use the explicit fallback pairing flow.
