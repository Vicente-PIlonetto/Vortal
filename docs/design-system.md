# Design system

The Linux desktop window uses GTK widgets implemented in `crates/vortal-ui/src/linux_native.rs`; it shows the device ID, pending pairing request, and receiver settings. The Windows package still uses Electron and the simplified interface in `crates/vortal-ui/ui`. Run `./scripts/install.sh` to install the Linux app and icon under `~/.local`. `./scripts/serve-ui.sh` remains available for browser previews.

- Dark by default, with system theme selected on first launch and an optional light theme.
- Monochrome 11-step grayscale tokens in one CSS variables file; no hardcoded colors elsewhere.
- Flat Metro tiles on an 8 px grid. No shadows, gradients, decorative transparency, or rounded corners.
- Selawik with Segoe UI and system sans-serif fallbacks. Use typographic hierarchy and luminance for state.
- Short labels and one-word states. UI copy is localized in `pt-BR` and `en`.
- Animate only `transform` and `opacity`; honor `prefers-reduced-motion` and keep all transitions under 500 ms.
- Visible keyboard focus, full keyboard navigation, and WCAG AA contrast.
- Keep UI rendering in a separate process from the input daemon.
