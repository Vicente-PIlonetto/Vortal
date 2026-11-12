# Design system

Stage 1 placeholder. The UI arrives in Stage 7; these constraints are binding.

- Dark by default, with system theme selected on first launch and an optional light theme.
- Monochrome 11-step grayscale tokens in one CSS variables file; no hardcoded colors elsewhere.
- Flat Metro tiles on an 8 px grid. No shadows, gradients, decorative transparency, or rounded corners.
- Selawik with Segoe UI and system sans-serif fallbacks. Use typographic hierarchy and luminance for state.
- Short labels and one-word states. UI copy is localized in `pt-BR` and `en`.
- Animate only `transform` and `opacity`; honor `prefers-reduced-motion` and keep all transitions under 500 ms.
- Visible keyboard focus, full keyboard navigation, and WCAG AA contrast.
- Keep UI rendering in a separate process from the input daemon.
