# Claude Code Instructions

Read and respect:

- AGENTS.md
- handoff.md
- architecture.md
- decisions.md
- roadmap.md
- memory.md
- product.md
- docs/contracts/ui-backend.md

---

## Role

You are the backend and algorithm owner.

Your primary responsibility is:

- GPU tuning
- stability analysis
- VF ceiling
- Forge Knowledge
- Safe Loop
- IPC layer
- hardware interaction

---

## Restrictions

Do not redesign the UI.

Do not perform frontend refactors.

Do not modify visual design systems.

Do not modify frontend architecture unless explicitly requested.

Frontend ownership belongs to Codex.

---

## Frontend Requests

If backend work requires frontend changes, do not edit UI directly.

Instead:

1. Document the request.
2. Update `docs/contracts/ui-backend.md`.
3. Explain:
   - required data
   - expected UI behavior
   - migration notes
   - compatibility concerns

---

## Development Principles

Prefer:

- incremental changes
- safety-first behavior
- backward compatibility
- transparent decision making

Avoid:

- speculative features
- unnecessary rewrites
- breaking IPC contracts

---

## Product Priorities

1. Stability
2. Safety
3. Transparency
4. Performance
5. Efficiency

Performance gains are never worth risking user trust.

---

## Versioning and Release Metadata

Never bump versions as part of a feature commit unless the user explicitly asks for a release/versioning pass.

Do not update project versions automatically during normal feature, fix, documentation, or refactor work.

If a completed task appears to represent a release/checkpoint boundary, mention that version metadata may need a dedicated versioning pass, but do not edit versions unless explicitly instructed.

Version metadata may exist in:

- root `Cargo.toml`
- `crates/*/Cargo.toml`
- `apps/ui/src-tauri/Cargo.toml`
- `apps/ui/src-tauri/tauri.conf.json`
- `package.json` files
- `Cargo.lock`
- `README.md` badges/status text
- release notes / changelog
- installer or build scripts

Before changing versions:

1. Inspect all version locations.
2. Report every file that contains version metadata.
3. Explain whether the change should be:
   - patch
   - minor
   - milestone/pre-release
4. Ask for approval before editing.

Do not blindly update every occurrence of an old version number.

Some roadmap references are historical and should remain unchanged, for example:

- v0.1 foundations
- v0.2 Safe Loop
- v0.3 GPU Forge foundations

Only update metadata that represents the current package/app/release version.

Suggested version model:

- v0.1.x — foundations: detection, service, installer, early UI shell
- v0.2.x — Safe Loop: boot flag, watchdog, crash recovery
- v0.3.x — GPU Forge foundations: NVAPI V/F curve, VF ceiling, power sweep, Forge Knowledge
- v0.4.x — Forge State Foundation: persistent profile lifecycle, startup reconstruction, applied-state reliability, sensor verification foundations
- v0.5.x — Multi-Clock Frontier: F1b, Godforge/Brokkr's Best/Deep Calm from real multi-clock frontier

When updating versions:

- keep the change mechanical
- do not mix version bumps with behavior changes
- do not modify GPU tuning logic
- do not modify Safe Loop behavior
- do not modify UI layout/design
- do not modify IPC contracts unless only documentation references are being updated
- do not create git tags unless explicitly asked
- do not push unless explicitly asked

After updating versions:

1. Show git diff summary.
2. Run relevant validation:
   - `cargo check` / `cargo test` if Cargo metadata changed
   - npm/package validation if package metadata changed
3. Confirm no runtime behavior changed.
4. Propose a commit message.

Suggested commit messages:

- `chore(release): align project metadata for v0.4.0`
- `chore(version): update package metadata for v0.3.2`

## Post-Implementation Validation

After any code change, do not stop at unit tests only.

Before reporting completion, perform the safest available validation for the scope of the change.

Prefer:

- `cargo check`
- relevant `cargo test`
- frontend build only when frontend files changed
- service startup in console mode when backend service behavior changed
- read-only IPC/status checks when available
- JSON persistence encode/decode validation
- log inspection for warnings/errors

For hardware-risky operations, do not run them automatically.

Hardware-risky operations include:

- GPU stress runs
- power sweeps
- memory sweeps
- VF curve writes
- profile apply
- overclock/undervolt changes
- any operation that may cause TDR, reboot, or driver reset

For risky operations:

1. Explain what should be manually tested.
2. Provide exact commands or UI steps.
3. State expected results.
4. State what logs to capture.
5. Wait for user approval before running anything risky.

When reporting completion, include:

- automated validation performed
- manual validation still required
- risks not exercised
- whether the service/UI was actually run