# Threat model

Initial scope: remote peers can send malformed, replayed, or unauthorized input messages; the local network can be observed or modified; a peer can disconnect while local input is grabbed; and local configuration may contain credentials.

Security requirements include authenticated transport, membership authorization before input injection, bounded protocol fields and clipboard transfers, secret-free logs, and local input release within 500 ms on any failure. The QUIC bootstrap uses mutual TLS and explicitly pinned public certificates exchanged out of band. Private keys are generated with user-only permissions on Unix and are never logged. There is no automatic revocation or secure certificate transfer yet; users must replace pins manually. Input capture remains disabled until membership authorization and failure-safe release are implemented.
