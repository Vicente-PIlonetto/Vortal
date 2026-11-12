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

`vortal-vylink` now implements the platform-neutral `PeerDirectory` trait using the authenticated snapshot API. It posts `{"network":"<network name>"}` to `/v1/snapshot`, reads the `network.devices` fields above, and polls for changes every 15 seconds. The response is capped at 1 MiB. This request body field should be checked against the deployed VyLink API contract; the repository documentation available to Vortal did not specify the request schema. The adapter requires HTTPS and disables redirects so a bearer token is not forwarded to another host.

The adapter accepts a `VyLinkCredentials` provider rather than loading or persisting a token itself. The CLI and daemon implement that provider using the native credential store via `keyring`; use `vortal vylink store-token` to save the bearer token. `vortal-net::serve_with_peer_directory` accepts a `PeerDirectory` and only sends `HandshakeAccepted` when the Hello device ID equals a currently online member's `publicKey`. For this mode, create the Vortal identity with that same public key as its device ID. The regular listener command remains an explicitly pinned-certificate fallback without VyLink membership lookup. Do not use a Git submodule or copy VyLink implementation code.

Manual IP and mDNS discovery remain independent fallback `PeerDirectory` implementations. They do not inherit VyLink's membership authorization and must use the explicit fallback pairing flow.
