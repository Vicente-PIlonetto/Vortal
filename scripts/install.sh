#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${HOME}/.local/bin"
data_dir="${XDG_DATA_HOME:-${HOME}/.local/share}"
icon_dir="${data_dir}/icons/hicolor/128x128/apps"
applications_dir="${data_dir}/applications"
service_dir="${HOME}/.config/systemd/user"

"${repo_root}/scripts/build.sh"
install -Dm755 "${repo_root}/target/release/vortal" "${bin_dir}/vortal"
install -Dm755 "${repo_root}/target/release/vortald" "${bin_dir}/vortald"
install -Dm755 "${repo_root}/target/release/vortal-desktop" "${bin_dir}/vortal-desktop"
install -Dm644 "${repo_root}/crates/vortal-desktop/assets/vortal.png" \
  "${icon_dir}/com.vortal.desktop.png"

desktop_file="${applications_dir}/vortal.desktop"
install -Dm644 "${repo_root}/scripts/vortal.desktop" "${desktop_file}"
install -Dm644 "${repo_root}/scripts/vortal-input.service" \
  "${service_dir}/vortal-input.service"
sed -i \
  -e "s|^Exec=.*|Exec=${bin_dir}/vortal-desktop|" \
  -e "s|^Icon=.*|Icon=${icon_dir}/com.vortal.desktop.png|" \
  "${desktop_file}"

if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
  systemctl --user daemon-reload
  if [[ ! -f "${HOME}/.config/vortal/startup-enabled" ]] || [[ "$(cat "${HOME}/.config/vortal/startup-enabled")" == "1" ]]; then
    systemctl --user enable vortal-input.service
  fi
  if [[ -f "${data_dir}/vortal/identity/key.der" && -f "${data_dir}/vortal/peer.der" ]]; then
    systemctl --user restart vortal-input.service
  fi
fi

printf '\nVortal instalado. Abra pelo menu de aplicativos ou execute:\n  %s\n' \
  "${bin_dir}/vortal-desktop"
