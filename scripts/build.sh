#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if ! command -v cargo >/dev/null 2>&1; then
  echo "Erro: instale Rust e Cargo antes de gerar os binários." >&2
  exit 127
fi

cargo build --workspace --release
printf '\nBinários gerados em target/release:\n  vortal\n  vortald\n'
printf '  vortal-desktop\n'
printf '\nGere identidade e certifique o peer antes de conectar:\n'
printf '  target/release/vortal identity DEVICE_ID --dir "$HOME/.local/share/vortal/identity"\n'
printf '  target/release/vortal connect 100.64.0.2:24800 --identity "$HOME/.local/share/vortal/identity" --peer-cert "$HOME/.local/share/vortal/peer.der"\n'
