#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
port="${1:-4173}"
exec python3 -m http.server "$port" --directory "$repo_root/crates/vortal-ui/ui"
