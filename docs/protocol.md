# Protocol

The protocol is a versioned message contract with no network I/O. Stage 1 serializes `Envelope` using Postcard. `Envelope.protocol_version` is checked before a message is accepted; the current version is `1`. Peers with an unknown version are rejected.

| Message | Fields and purpose | Delivery intent |
| --- | --- | --- |
| `Hello` | protocol version, stable device ID, display name, capability list | Reliable stream |
| `MouseMove` | signed relative `dx` and `dy` deltas | Reliable stream |
| `MouseButton` | button identifier and pressed state | Reliable stream |
| `MouseScroll` | horizontal and vertical signed wheel deltas | Reliable stream |
| `Key` | physical scancode, logical keycode, pressed state, modifier bits | Reliable stream |
| `EnterScreen` | crossed edge and normalized position (0–65,535) | Reliable stream |
| `LeaveScreen` | relinquish remote screen control | Reliable stream |
| `ClipboardOffer` | transfer ID, MIME type, total byte size, chunk count | Reliable stream |
| `ClipboardData` | transfer ID, zero-based chunk index, bytes, final-chunk flag | Reliable stream |
| `Ping` / `Pong` | nonce echoed for round-trip measurement | Reliable stream |

The QUIC transport uses TLS 1.3 mutual authentication. Each installation creates a self-signed certificate; the user exchanges public certificates out of band and explicitly pins the expected peer certificate. The first bidirectional stream carries a length-prefixed Postcard `Hello` and `HandshakeAccepted` frame, limited to 64 KiB. The experimental input path opens a second reliable stream for input events and 75 ms heartbeats; either end closes an inactive session after 300 ms. `EnterScreen` moves the Hyprland cursor to the corresponding edge. On return, the Linux peer sends `LeaveScreen` to the Windows peer and drops input until another `EnterScreen`. Clipboard limits, datagram coalescing, and full input negotiation remain future work.
