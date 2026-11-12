# Protocol

The protocol is a versioned message contract with no network I/O. Stage 1 serializes `Envelope` using Postcard. `Envelope.protocol_version` is checked before a message is accepted; the current version is `1`. Peers with an unknown version are rejected.

| Message | Fields and purpose | Delivery intent |
| --- | --- | --- |
| `Hello` | protocol version, stable device ID, display name, capability list | Reliable stream |
| `MouseMove` | signed relative `dx` and `dy` deltas | Unreliable datagram |
| `MouseButton` | button identifier and pressed state | Reliable stream |
| `MouseScroll` | horizontal and vertical signed wheel deltas | Reliable stream |
| `Key` | physical scancode, logical keycode, pressed state, modifier bits | Reliable stream |
| `EnterScreen` | crossed edge and normalized position (0–65,535) | Reliable stream |
| `LeaveScreen` | relinquish remote screen control | Reliable stream |
| `ClipboardOffer` | transfer ID, MIME type, total byte size, chunk count | Reliable stream |
| `ClipboardData` | transfer ID, zero-based chunk index, bytes, final-chunk flag | Reliable stream |
| `Ping` / `Pong` | nonce echoed for round-trip measurement | Reliable stream |

Clipboard limits, datagram coalescing, transport authentication, and negotiation policy are defined with the network implementation in Stage 3. No untrusted peer may inject input until authorization is established.
