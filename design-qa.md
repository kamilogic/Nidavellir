# Design QA — Forge themes

## Scope

Literal image-to-code reconstruction of the three supplied Forge references, implemented as selectable UI themes. Command Deck is the default theme.

## Test state

- Viewport: 1487 × 1058
- Route: `http://127.0.0.1:5174/`
- Browser preview data: NVIDIA GeForce RTX 3060 Ti, forged/protected state, Brokkr's Best applied
- Product copy remains English as required by `apps/ui/AGENTS.md`; layout, hierarchy, controls, materials, typography treatment, colors, and spacing follow the references.

## Source references

- Command Deck: `C:\Users\leona\AppData\Local\Temp\codex-clipboard-8ac3202a-b7f5-4df2-aca9-0becfef759e6.png`
- Instrument Panel: `C:\Users\leona\AppData\Local\Temp\codex-clipboard-72d0af04-b761-486e-b36c-85bba5368e36.png`
- Quiet Workshop: `C:\Users\leona\AppData\Local\Temp\codex-clipboard-bdfffb5e-9b91-4cbd-babc-e04d3b250c2a.png`

## Full-screen comparison evidence

- Command Deck: `C:\Users\leona\.codex\visualizations\2026\07\13\019f5d13-7b70-7472-a4a5-9fcb9e259588\command-final-compare.jpg`
- Instrument Panel: `C:\Users\leona\.codex\visualizations\2026\07\13\019f5d13-7b70-7472-a4a5-9fcb9e259588\instrument-final-compare.jpg`
- Quiet Workshop: `C:\Users\leona\.codex\visualizations\2026\07\13\019f5d13-7b70-7472-a4a5-9fcb9e259588\workshop-final-compare.jpg`

## Functional verification

- Theme selector changes between all three themes and persists the selection.
- Forge mode selector changes between Fast, Standard, and Long.
- Primary Forge and profile actions remain connected to the existing Forge callbacks.
- Advanced diagnostics expands the existing technical diagnostics area instead of duplicating it.
- Settings is a dedicated view; its theme controls persist selection across all three themes.
- Window controls use the native application frame; no duplicate minimize, maximize or close controls are rendered inside the UI.
- Browser console: no errors after theme switching, mode selection, and advanced-panel checks.
- Production UI build: passed.

## Iteration record

- Resolved macro-layout drift in all three themes at the target viewport.
- Resolved Command Deck GPU identity, copper CTA, telemetry, profile-row, range-marker, and advanced-bar alignment.
- Resolved Instrument Panel condensed heading, gauge scale/position, side telemetry distribution, recommendation panel, action rail, and viewport overflow.
- Resolved Quiet Workshop hero rhythm, profile tradeoffs, live telemetry values, section boundaries, and footer alignment.
- Reduced generated texture contrast and tuned per-theme surface tone to match the supplied forged-metal backgrounds.

## Final assessment

- P0 issues: none
- P1 issues: none
- P2 issues: none
- Final result: passed
