# Nidavellir

## Vision

Nidavellir is a GPU optimization and profiling system that learns the real behavior of each individual GPU and forges transparent profiles based on performance, efficiency and stability.

> Where silicon is forged to its prime.

Current scope is NVIDIA GPU tuning on Windows.

CPU, RAM and motherboard tuning are currently out of scope.

---

## Product Philosophy

Prioritize:

* transparency
* safety
* explainability
* recovery from failures
* long-term learning

Avoid:

* magic optimization
* unnecessary complexity
* generic telemetry dashboards
* hidden decision making

---

## Profiles

### Godforge

Maximum sustainable performance.

### Brokkr's Best

Best balance between performance and power.

Recommended for most users.

### Deep Calm

Maximum efficiency.

---

## Theme

Use the forge theme consistently.

Preferred terminology:

* Forge GPU
* Refine Profiles
* Forge Knowledge
* Forge Progress
* Forged
* Tempered
* Refined
* Legendary

Keep language professional and clear.

---

## Agent Ownership

### Claude Code

Backend and algorithm owner.

Responsibilities:

* GPU tuning algorithms
* Safe Loop
* Forge Knowledge
* Stability logic
* IPC implementation
* Rust backend
* Hardware interaction

Do not modify frontend architecture or UI design.

If frontend changes are required, create a request in:

docs/contracts/ui-backend.md

---

### Codex

Frontend and UI/UX owner.

Responsibilities:

* Svelte frontend
* Design system
* UX
* Information architecture
* Product copy
* Forge theme consistency

Do not modify backend algorithms or hardware logic.

If backend changes are required, create a request in:

docs/contracts/ui-backend.md
