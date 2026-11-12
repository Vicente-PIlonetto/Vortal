# Threat model

Initial scope: remote peers can send malformed, replayed, or unauthorized input messages; the local network can be observed or modified; a peer can disconnect while local input is grabbed; and local configuration may contain credentials.

Security requirements include authenticated transport, membership authorization before input injection, bounded protocol fields and clipboard transfers, secret-free logs, and local input release within 500 ms on any failure. Detailed assets, trust boundaries, and mitigations will be expanded alongside the network implementation.
