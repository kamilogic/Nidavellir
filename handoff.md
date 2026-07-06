# Nidavellir — Session Handoff

How to pick this up cold. State as of 2026-07-04: the F2 live Forge keeps homogeneous PowerRender
discovery/p99 calibration, but deployability now requires qualification contract v8: High-FPS,
Texture and Transitions — each including the new FrameCadence phase (game-frame-scale droop
transients, own stock golden) — at the boundary and exact Apply. P5 remains the performance floor;
p95 owns the electrical support regime with zero bin tolerance. Stop is cooperative inside GPU loops
and the UI avoids overlapping polling. Apply still requests 12 mV above the learned boundary, snaps
to a valid physical VF bin and exposes both values. `finished` means current v8 profiles are
qualified. **NEXT = supervised v8 hardware gate; Leva 2 remains blocked.** Direction roadmap:
`docs/qualification-v8-plan.md`.
Deep NvAPI struct details live in `~/.claude/.../memory/gpu-forge-real-v031.md`.

## Backend fix (2026-07-06, post-commit) — console shutdown handler (committed 6551997)
- Console mode had no Ctrl+C/close handler: the main thread blocks in `ConnectNamedPipe` and
  worker threads keep the GPU saturated, so teardown stalled on driver DLL detach (Ctrl+C and
  "End task" looked dead for a long time). New `console_shutdown` module in `main.rs`:
  `SetConsoleCtrlHandler` signals every motor's cooperative stop (same path as IPC Stop → dwell
  cancels within a band, resets to stock, clears boot flag), waits a bounded 30 s grace polling
  the forge's `running`, then exits; second Ctrl+C forces immediate exit. Grace expiry exits
  anyway — an armed boot flag is the Safe Loop recovery's designed input. Added
  `Win32_System_Console` feature. Tests 357/0.

## Latest backend checkpoint (2026-07-06, late) — v12: regime LIFT + exact-Apply 4-pattern fix (code-complete, NOT HW-tested)
- **Run finding 1 (bug)**: exact-Apply ran only 3 patterns (`gate_anchored_candidate_fsgl3` was
  called with a hardcoded `final_gate_passes = 3`) while the p95/p99 publish gates require the
  complete 4-pattern set — 1875@925 passed 15 min of soak and was refused as "sem p95 sustentado
  mensurável". Fixed: the call now passes `REQUIRED_QUALIFICATION_PATTERNS.len()`.
- **Run finding 2 (design)**: the strict p95 reconciliation excluded 11/13 candidates again — but
  the log showed the excluded-with-reason voltages were exactly right: lifting 1800's Apply to the
  1830-regime requirement gives 875 mV = the user's hand-validated daily driver. **v12 replaces
  exclude with LIFT**: `apply_f2_margin_policy` now (a) records `base_apply_mv` (boundary+margin,
  new additive `PowerSweepPoint` field), (b) lifts `vf_table_voltage_mv` to the sustained regime's
  required voltage computed FROM BASE APPLIES — lifted values never feed requirements, so lifts
  cannot cascade (the lifted extra on target S covers S's own overshoot, which a lower target's
  hardware never reaches). `f2_regime_support` also reads `base_apply_mv` (fallback to
  `vf_table_voltage_mv` for legacy), so the strict reconciliation stays as a fail-closed net that
  lifted points satisfy by construction. Top-of-frontier points with NO measured regime above are
  still excluded (future: direct regime confirmation dwell). Lift runs BEFORE the p99 backfill, so
  lifted pairs get their own calibrated power like any pair.
- **Expected next run**: lifted Apply ladder (e.g. 1800→875, 1830→893, mid/top pairs at the
  sustained-regime voltages), 3 profiles synthesized, exact-Apply 4×5 min per selected pair.
  1935-at-cap remains excluded by target-residency Inconclusive (power-bound; known, conservative).
  1875 boundary 912 was a non-monotonic outlier vs 1890@906 — watch it; isotonic outlier handling
  is a possible follow-up.
- **Validation**: workspace 488/0 (new lift + no-cascade unit test). No hardware touched. Safety
  audit still pending for v11 A2/A3/C1 + this v12 gate change before commit.

## Backend checkpoint (2026-07-06) — v11 qualification engine hardening (code-complete, NOT HW-tested)
- **Why**: the v10 run traded graceful silent errors for straight TDRs (2 in one run, e.g.
  1920@906 boot-flag crash) and froze Discord voice at each burst — replacing the L2-resident
  TextureRop with the memory-latency version removed the wrong-pixel detector AND made frames
  giant non-preemptible draws.
- **B1**: TextureRop reverted to the v9 L2-resident form (the proven graceful detector).
- **B2/A1**: the scattered VRAM sampling moved to a NEW `TextureStream` workload/phase (code 11,
  own golden `RenderGoldens.stream`), rendered in 16 scissor BANDS with one submit each — the
  driver can preempt between bands (desktop/audio responsive) and each band is timed.
- **A2 (two layers)**: (1) a band exceeding `STREAM_PREHANG_BAND_MS = 500` fails the dwell as the
  new `StabilityResult::Unstable` BEFORE the ~2 s driver watchdog (partial frames are never
  checksummed); (2) the existing `prehang_stall_detected` NVML-starvation signal (recorded since
  v6, log-only) is now a failing verdict in `classify_f2_stress_dwell` for qualification dwells.
- **A3**: golden capture now also returns avg frame time; `RenderGoldens.stream_frame_reference_ms`
  is the stock reference — sustained stream frame time beyond 2× reference fails the bin as
  Unstable (marginal silicon slows before it hangs). `capture_one_golden` returns
  `(checksum, avg_frame_ms)`.
- **B3**: severity ladder — hang-prone detectors (VramPressure, TextureStream) run LAST in
  V8Texture/V8Memory; graceful detectors kill bad bins cheaply first (unit-tested ordering).
- **C1**: crash-proximity margin in core synthesis: `frontier_entry_for_target` refuses a boundary
  closer than `F2_CRASH_PROXIMITY_MIN_MV = 12` (~2 physical bins) above the target's highest
  crash/TDR anchor (`crash_floor_for_target`, pure + tested) — a TDR at V taints V+1 even if it
  passed, per the observed reality that the silent-error threshold above a crash went undetected.
- **Contract v11**; `StabilityResult::Unstable` added (legacy F1 matches map it as non-crash
  failure). Validation: workspace 487/0; clippy clean on new code. No hardware touched.
- **Safety note**: A2/A3/C1 change verdict/gate semantics — run `nidavellir-safety-auditor`
  before merge/commit.
- **HW gate (manual)**: rerun Standard. Expect: TextureRop silent errors return as the primary
  killer (graceful, early); Discord/desktop responsive during TextureStream; log shows Unstable
  verdicts (pre-hang/degradation) instead of TDRs; any TDR that still happens must push the
  boundary ≥2 bins above it in synthesis. Regression table gate unchanged (1815@856 & 1860@875
  must be rejected; 1800 boundary near the user's known ~875).

## Backend checkpoint (2026-07-05, late) — 1.7 texture-stream + 1.8 upward recovery, contract v10 (code-complete, NOT HW-tested)
- **Why**: the 2026-07-05 HW run + user ground truth (1800@875 stable daily; 1800@868 and 1830@875
  UNSTABLE in game) showed v9 still ~4-5 bins optimistic (approved 1815@843, 1860@875) — and 1800
  was discarded whole because the warm-start entered below the real boundary. Ground-truth table
  lives in `docs/qualification-v8-plan.md` ONLY as a validation gate — never hardcoded.
- **1.7 (gpu-stress)**: TextureRop (the empirically sensitive detector — every v9 failure fired in
  `texture-rop`) now samples a large VRAM-resident source: fixed 8192² RGBA8 (256 MB, size fixed —
  NOT runtime-probed — so golden capture and qualifier can never diverge on size), GPU-filled via
  hash shader (`TEXTURE_STREAM_FILL_SHADER`), and the tap chain start is SCATTERED per pixel
  (sin-hash of frag coords) so neighbouring fragments hit far-apart texels → bilinear taps pay DRAM
  latency: TMU + memory controller together, the game texturing path. Same source in
  `capture_one_golden`. `F2_QUALIFICATION_CONTRACT_VERSION = 10` (v9 positives are the proven false
  negatives; they cannot unlock Apply).
- **1.8 (service)**: two fixes. (a) `warm_start_rejected` now requires a bin with the FULL required
  pattern set passed — a single-pattern pass (High-FPS ok, Texture failed) no longer suppresses the
  conservative fallback; `sustainable` uses the same full-set check. (b) New bounded upward
  recovery in the outer ladder: a `QualificationRejected` end (NOT ClockDrop) re-runs the clock one
  physical bin above the last attempted start, up to `F2_START_RECOVERY_MAX_CLIMBS = 4` (generic
  search parameter). Pure helper `f2_next_bin_above` + test.
- Earlier same day: IPC freeze fixed (`ERROR_PIPE_CONNECTED` treated as success; handle closed on
  connect error) — `ipc_server.rs`.
- **Validation**: workspace 486/0; clippy clean on new code. No hardware touched.
- **HW gate (manual)**: clear forge state → Forge Standard → the run must now (a) REJECT 1815@856
  and 1860@875-class points (expect `texture-rop` failures at higher voltages than v9), (b) land
  the 1800 MHz boundary near the user's known ~875 (climb log lines "recuperação para cima" prove
  1.8 fired), (c) TextureRop frame times will be longer (memory-latency bound) — watch for TDR
  margin; per-frame work is bounded but untested on HW.

## Backend checkpoint (2026-07-05) — Phase 1 complete: v8 workloads full set, contract v9 (code-complete, NOT HW-tested)
- **Why**: v8 with FrameCadence alone still passed points the user knows crash in-game. Completed
  the full Phase 1 workload set from `docs/qualification-v8-plan.md`.
- **New workloads** (`gpu-stress`): `VramPressure` — up to 8×256 MB VRAM-resident tables (OOM-guarded
  via error scopes, degrades gracefully), cache-defeating gathers cycling tables per dispatch,
  known-answer verified (any mismatch = silent error). `GeometryDepth` — 49 152 procedural triangles
  × 8 instances under a depth test (unique per-triangle depth ⇒ deterministic image), loads vertex
  fetch/raster/depth-ROP; golden-verified (`RenderGoldens.geometry`, 5 goldens captured now).
- **Patterns renamed V7→V8** (labels `v8-*`) and extended: HighFps/Transitions +GeometryDepth,
  Texture +VramPressure, and NEW `V8Memory` pattern (VRAM-dominant, all 11 phases). Qualification
  set is now **4 patterns** (HighFps/Texture/Transitions/Memory): boundary 4×60 s, exact-Apply
  4×5 min (upper estimates updated: target 275 s, apply pair 1 220 s).
- **Core**: `F2QualificationPattern::Memory`; canonical `REQUIRED_QUALIFICATION_PATTERNS: [_; 4]` —
  all completeness gates (p99/p95 at anchor, frontier qualification count) now index into it, so
  extending the array tightens every gate automatically. `F2_QUALIFICATION_CONTRACT_VERSION = 9`.
  Item 1.5: pure `qualification_failure_histogram()` aggregates failed dwells by
  (clock, mV, pattern, failure_phase) — data source for future pattern weighting/adaptive margin;
  log/UI wiring deliberately deferred.
- **Item 1.6 verified, no change needed**: golden-mode MixedGame decomposes into
  BoostEdge/TextureRop/PowerRender (each golden-checked); ComputeBurst is known-answer.
- **Validation**: workspace 485/0 tests; clippy baseline only (0 warnings in new code). No hardware
  touched.
- **HW next (manual)**: clear forge state → Forge Standard → confirm 5 golden captures + 4 patterns
  run (look for `vram-pressure`/`geometry-depth` phases in JSONL) → re-test the known-unstable
  points that v8-cadence-only still passed; they must now be rejected (likely in `vram-pressure` or
  `frame-cadence` phases). VramPressure on cards < 4 GB and the geometry golden determinism across
  driver versions are the untested risks.

## Backend checkpoint (2026-07-04) — qualification v8: FrameCadence phase (code-complete, NOT HW-tested)
- **Why**: v7-passing points still crash/TDR in real games. Root-cause analysis (see
  `docs/qualification-v8-plan.md`, item 1.1): the v7 patterns never exercise VRM droop-release
  transients at game frame cadence — idle pulses fire every 750 ms while games oscillate load every
  6–16 ms, which is where undervolt Vmin actually fails.
- **What**: new `VfWorkload::FrameCadence` + `VfQualifierPhase::FrameCadence` (code 8, label
  `frame-cadence`) in `gpu-stress`: one heavy RENDER_SHADER frame at 1 instance (~10-20 ms of work =
  a game frame) → poll(Wait) → sleep gap cycling 2/4/6/8 ms, repeating. Each frame is a heavy burst;
  the gap sweep crosses different VRM response periods. Golden-verified with its OWN stock checksum
  (`RenderGoldens.cadence` — the 1-instance image differs from the 8-instance power golden);
  `capture_fsgl3_render_goldens` now captures four goldens.
- **Patterns**: FrameCadence segments inserted into all three plans — HighFps ×2, Texture ×1,
  Transitions ×3. Coverage denominator is now pattern-specific via new pure
  `qualifier_expected_phases()` (legacy FSGL = 8 phases, v7 plans = 9); the old fixed
  `EXPECTED_PHASES = 8` + `[false; 8]` would have panicked on phase code 8. A v7-pattern run that
  skips FrameCadence is Inconclusive.
- **Contract**: `F2_QUALIFICATION_CONTRACT_VERSION = 8` — pre-cadence positives cannot unlock Apply;
  negatives stay conservative. Discovery contract (v4 PowerRender) untouched.
- **Validation**: `cargo check --workspace` clean; `cargo test --workspace` 484/0 (core 78, gpu-stress
  40, service 355); clippy shows only the pre-existing baseline. No VF write, Forge, Apply or GPU
  stress was run.
- **HW next (manual, user-run)**: clear persisted forge state → run Forge Standard → confirm the four
  golden captures succeed and the three patterns execute FrameCadence phases (look for
  `frame-cadence` in phase metrics/JSONL), then re-test the known game-crashing point: v8 should
  reject it at the voltage v7 accepted.

## Backend checkpoint (2026-07-04) — held-clock thermal rule for exact Apply (code-complete, NOT HW-tested)
- **Why**: a full run ended with ZERO profiles. The power-bound top point `1935 MHz @ 956 mV` (pinned
  at the 200 W cap) failed exact-Apply as `ExactApplyInconclusive` — three HighFPS dwells tripped NVML
  `thermal_throttled` from a memory-junction hotspot at only ~67-69 C core, while the card actually
  HELD >= 1935 MHz with no silent error and full coverage. The guardrail rejected evidence where the
  card never left the qualified point.
- **Fix (two layers, must stay in sync)**: a thermal-slowdown flag disqualifies exact-Apply stability
  only when p5 sagged below target beyond 30 MHz (`F2_CLOCK_DROP_TOL_MHZ`). (1) classifier
  `classify_f2_stress_dwell` (`ApplyQualification` arm), (2) publish gates
  `apply_qualification_p99_at_anchor` + `current_apply_qualification_p95_clock_at_anchor` via new
  `apply_qual_reading_trustworthy` (`f2_observation.rs`). Fails closed on unknown clock.
- **Kept strict**: PowerDiscovery power calibration (`f2_power_measurement_usable`,
  `current_discovery_observation_at_anchor`) still rejects any throttle — a throttled sample understates
  the V<->W map.
- **Safety**: audited SAFE twice (`nidavellir-safety-auditor`). Publish aggregation is max-only, so a
  held-throttled reading can only raise published wattage (never understate) and raise p95 (stricter
  voltage reconciliation). Triad completeness / reset-clean / boot-flag untouched.
- **Validation**: core 78/0, service 355/0. No VF write, Forge or Apply run automatically.
- **NEXT**: user runs a controlled rerun to confirm 1935 @ 956 mV now advances
  HighFPS->Texture->Transitions and publishes. Changes are in the working tree on `master`, NOT
  committed — rebuild picks them up.

## Latest backend/frontend checkpoint (2026-07-03) — stage-aware Forge time model
- **Structured plan:** progress now publishes Cmax, the inclusive 90%-floor real clock, real-clock
  count and a conservative absolute total ceiling. All fields are additive/defaulted.
- **Phase-aware ceiling:** pre-Cmax remains explicitly `Refining`; post-Cmax uses the exact physical
  domain. Frontier discovery/qualification, possible three-attempt p99 backfills and up to three
  exact-Apply v7 pairs are accounted separately, then tightened as each stage becomes concrete.
- **UI:** Forge Progress separates live remaining, estimated run total, maximum estimated total and
  elapsed wall time, with readable stage copy and no inference from technical logs.
- **Safety/status:** progress-only change. No tuning algorithm, qualification decision, hardware
  write or Safe Loop behavior changed; no Forge was run automatically.

## Latest backend/frontend checkpoint (2026-07-03) — qualification v7 + cooperative cancellation
- **Automated oracle:** Standard/Long use three deterministic patterns targeting high frame cadence,
  texture/ROP/mixed graphics pressure and rapid load transitions. Each retains stock-golden
  verification; older FSGL evidence is excluded from v7 Apply.
- **Strict regime:** synthesis uses measured p95, not p5, to identify electrical support. Any higher
  sustained regime must have a measured target/Apply anchor and all three current qualification
  patterns; no one-bin alias tolerance remains.
- **Responsive Stop:** cancellation reaches discovery and qualification render/compute loops,
  prevents new bounded batches, returns through normal checked stock reset and cannot blacklist or
  validate the interrupted point.
- **UI/IPC load:** refresh cycles do not overlap; secondary diagnostics poll every 3 s during Forge;
  Stop displays `stopping` immediately; live log payload is capped at 240 lines while JSONL evidence
  remains complete.
- **Hardware status:** code/tests only. No VF write, Forge or Apply was run automatically.

## Latest backend/frontend checkpoint (2026-07-01) — confirmed sustained-p99 frontier/calibration (code-complete, NOT HW-tested)
- **Workload unchanged**: live F2 discovery still uses the textured, bounded-frame `PowerRender`;
  compute-only `POWER_SHADER` was not substituted.
- **Discovery v4 telemetry**: mean, sustained p99 and raw maximum watts persist separately, with
  maximum temperature, NVML thermal-slowdown, measured-voltage and render coverage evidence. An
  anomalous adjacent-bin p99 in the same p5 regime repeats the exact bin up to three total attempts;
  two readings must agree and the highest measured p99 is retained. No consensus is ineligible.
- **Applied-bin truth**: after the unchanged +12 mV margin snaps to a physical bin, synthesis resolves
  that exact bin's current PowerRender observation. Selection and cards use its p99 plus apply-bin p5;
  mean/raw maximum remain diagnostic. If warm-start skipped the exact target/apply pair, Forge now
  backfills it with discovery-only PowerRender under the same v4 p99 consensus; no FSGL3 rerun.
  Missing/current-invalid p99 still fails closed with no profiles.
- **Boundary**: any discovery `ClockDrop` still at 99%+ of the cap by p99 is
  `PowerBoundClockDrop` and continues voltage descent, including after a prior sustained point.
  `Validated` at cap also continues; Standard/Long only launch FSGL3 from a confirmed off-cap bin.
- **Compatibility**: discovery contract is v4; qualification remains FSGL3 contract v4. v3 positives
  and unconfirmed power telemetry cannot enter new synthesis/resume; negatives stay conservative.
  Apply rejects any restored F2 profile without valid p99.
- **Hardware checkpoint**: a Standard run held p99 near the 200 W cap and continued 1950 MHz from
  1150 mV through 950 mV, where discovery first validated. FSGL3 A then reset-clean rejected that
  target on the unchanged heavy-phase p5 guardrail. A control-flow bug incorrectly ended the entire
  ladder; the rejection now completes only 1950 MHz so lower clocks can discover the qualified Cmax.
- **Hardware next**: rerun Standard and confirm the ladder advances below a reset-clean rejected clock,
  then compare the forged apply-bin p99/raw peak against the known game scene.

## Latest backend checkpoint (2026-06-30) — margin boundary + continuous recovery (code-complete, NOT HW-tested)
- **Margin stop**: equivalent FSGL3 heavy phases produce one robust p5 per dwell; A/B histories stay
  separate. `MARGIN_DROP_TOL_MHZ = 30` requires two prior stable references before the relative arm,
  while the existing target-minus-30 arm remains available.
- **Ambiguity**: two same-point retries use 1.5× dwell. Exhaustion records Inconclusive, completes the
  current clock safely and continues the outer frontier; it neither marks the point bad nor claims
  qualification.
- **Recovery**: 0x116/0x117 are OC/TDR classes. Exact `f2_undervolt_probe` TDR/Unknown events do not
  advance Safe Mode; unrelated crashes do. DeviceLost no longer increments before startup recovery,
  and blacklist insertion is idempotent and scoped to the exact F2 intent.
- **Resume/final state**: persisted mode is a stable `fast|standard|long` id. Restored `interrupted`
  state triggers one automatic non-destructive Reset+Start on UI reconnect. `finished` is emitted only
  for complete qualified profiles; Fast is `provisional`.
- **Apply margin**: `APPLY_MARGIN_MV = 12` snaps to an exact higher physical bin. Additive
  `boundary_voltage_mv` and `apply_margin_mv` preserve transparent boundary/apply semantics.
- **Pre-hang**: 300 ms missing-valid-sample stall is recorded/logged only. No concurrent reset was
  added; activation waits for hardware calibration plus cooperative cancellation.
- **Hardware**: not run. Gate must check margin ClockDrop frequency, clock-to-clock continuation,
  repeated TDR recovery, reconnect resume and the applied margin bin in game.

## Latest backend checkpoint (2026-06-30) — FSGL3 golden-sample qualification (code-complete, NOT HW-tested)
- **Battery**: added FSGL3 A/B plans, positional REDUCE3, deterministic stock-golden capture and
  per-frame on-GPU comparison. Golden mode runs six-frame bursts separated by 4 ms; legacy
  FSGL1/FSGL2 still use the unchanged REDUCE/self-reference/250 ms path.
- **Forge wiring**: Standard/Long capture power, boost and texture/ROP goldens after stock reset and
  seed derivation, with one fresh `GpuCtx` per configuration. Capture failure aborts safely. Goldens
  thread through `run_confirmed_f2_clock_discovery` to `single_qualifier_dwell` and never persist.
- **Default and Apply gate**: `qualify_anchored_candidate` now constructs FSGL3 A/B purposes.
  `F2_QUALIFICATION_CONTRACT_VERSION = 4`; only current FSGL3 A+B evidence unlocks Apply.
- **Validation so far**: `cargo build --workspace`, `cargo check --workspace`, `cargo test
  --workspace`, `npm.cmd run build`, `cargo clippy --workspace --all-targets` and `git diff --check`
  pass. Clippy reports only the existing baseline warnings. Hardware validation remains pending.
- **Hardware next**: clear persisted Forge state, then confirm FSGL3 rejects 1920 MHz @ 912 mV and
  1935 MHz @ 918 mV before any Standard/Long acceptance or in-game comparison.

## Latest backend checkpoint (2026-06-29) — Cmax descent interleaves qualification per VF bin (code-complete, NOT HW-tested with new flow)
- **Why**: discovery descended `PowerRender` to the deepest survivable bin then qualified THAT (most
  aggressive) bin with the failure-seeking loop — risking a TDR during qualification and wasting the
  descent below the bin that ultimately qualifies. Operator asked to interleave: qualify the first
  sustained point, then descend one bin at a time, gating each step by qualification.
- **What** (`crates/service/src/gpu_undervolt.rs`, `run_confirmed_f2_clock_discovery`): for Standard/Long,
  the per-clock loop now PowerRender-descends to the first under-cap `Validated` bin, qualifies it with the
  full N passes (new helper `qualify_anchored_candidate` returning `F2QualificationOutcome`), and only then
  steps one real VF bin lower (PowerRender measures its power and gates the next qualify). First
  qualification failure stops the descent, keeping the last qualified bin. The heavy qualifier never runs
  more than one bin below a proven point. The old descend-to-floor-then-qualify-deepest + upward back-off
  is removed. Fast (qualification_passes==0) is unchanged (provisional descend-to-PowerRender-floor).
- **Downstream unchanged**: a failed qualification writes an `is_bad()` observation, so
  `last_discovery_good_for_target`/`first_bad_for_target` already select the deepest QUALIFIED bin. Locked
  by new core test `interleaved_qualification_failure_selects_shallower_qualified_point`. Cmax/90% floor,
  synthesis, Safe Loop arm/verify/reset per dwell, resume/warm-start untouched.
- **Trade-off**: N passes × each qualified bin → longer Standard/Long; initial ETA under-counts and
  self-corrects upward (contract note added for Codex).
- **Validation**: `cargo check` clean; core 69 + service 319 tests; clippy no new warnings in touched
  files. **No hardware run with the new flow.** NEXT = one supervised Standard run; inspect that the
  qualifier only runs at/one-bin-below proven points and that a deeper rejection keeps the bin above.
  Recommended `nidavellir-safety-auditor` pass on the diff before commit.

## Latest backend checkpoint (2026-06-29) — Safe Mode unstick: Reset clears the latch + deep reset (Fix A/B/C HW-CONFIRMED by operator)
- **HW update**: operator rebuilt + ran in console; logs show a prior armed boot-flag recovered
  (blacklist+recede, consecutive_crashes accounted) and "Reset all" then cleared `forge_state` and a
  fresh F2 forge ran — "funcionou perfeitamente". The stuck-Safe-Mode / Needs-Attention dead-end is
  resolved on the rig. (Fix C clean-shutdown marker not separately stress-tested.)
- **Why**: operator reported the app stuck in Needs Attention / Interrupted with "no option," surviving
  manual PC restarts; the new Reset all did not release Safe Mode. Root cause: `safe_mode` is a one-way
  latch — `gpu_apply::reset` (the `ResetGpuTuning` body, gpu_apply.rs:236) reset hardware + boot-flag +
  applied profile but **never rewrote `safe_loop.json`**, and nothing anywhere set `safe_mode=false`.
  Plus each clean reboot in Safe Mode re-ran `EnterSafeMode` and incremented `consecutive_crashes`, and
  a manual restart during an armed boot-flag was counted as a crash.
- **Fix A** (`crates/core/src/safe_loop.rs` + `crates/service/src/gpu_apply.rs`): new
  `SafeLoopRecord::clear_recovery_latch()` (safe_mode→false, consecutive_crashes→0, state→idle, PRESERVES
  blacklist/last_validated/crash_log); `reset()` now load→clear_recovery_latch→save before clearing the
  boot-flag. The existing Reset all button now releases Safe Mode (no UI change).
- **Fix B** (`safe_loop.rs` + `safe_loop_runtime.rs`): new `RecoveryAction::RemainSafeMode` for a clean
  boot already in Safe Mode — stays hands-off without incrementing the streak. `EnterSafeMode` (the
  incrementing path) is now only the armed-flag threshold trip.
- **Fix C** (`safe_loop.rs` store + `safe_loop_runtime.rs` + `service_impl.rs`): graceful service
  Stop/Shutdown writes a one-shot `clean_shutdown.txt`; startup consumes it and treats armed-flag +
  marker as a clean interruption (no crash). Fail-closed: no marker ⇒ crash, parachute intact.
- **Deep reset** (`crates/core/src/ipc.rs` + `ipc_server.rs` + `gpu_apply::clear_all_learning`): new
  additive IPC `ResetGpuTuningFull` = `ResetGpuTuning` + wipe blacklist (record→default), F2
  `f2_observations.jsonl` and `gpu_knowledge.json`. UI button requested from Codex (contract 2026-06-29).
- **Validation**: `cargo check` clean; `nidavellir-core` 68 + `nidavellir-service` 319 tests pass; clippy
  no new warnings in touched files. **No hardware / VF write / apply / stress / reboot exercised.**
- **NEXT**: (1) supervised manual check — force Safe Mode, confirm Reset all returns to a forgeable state
  and survives a reboot; confirm a clean restart mid-forge no longer adds a crash; (2) Codex wires the
  Full reset button; (3) recommended `nidavellir-safety-auditor` pass on Fix C before commit. Nothing
  committed or pushed.

## Latest backend checkpoint (2026-06-29) — FailureSeekingGameLoop VF qualification
- `run_render_stress` remains the unchanged eight-instance `PowerRender` used by discovery, benchmark
  and legacy callers.
- `run_vf_qualifier_stress` executes PowerOpening, BoostEdge, HeavySpike, TextureRop, ComputeBurst,
  IdlePulse, MixedGame and PowerClosing. Each phase has independent checksum/coverage evidence; the
  failing phase is logged.
- Only the Standard/Long reset/reapply qualification motor selects the transient workload. Fast,
  CLI probes and all discovery candidates keep the steady workload.
- Mixed qualifier p5 cannot create `ClockDrop`; `Pass`/`Fail`/`Inconclusive` coverage is persisted in
  `f2_observations.jsonl`. Current Apply qualification counts only current-contract qualification
  passes, not discovery/legacy positives.
- Qualification rejection backs off to the next physical VF bin and performs fresh `PowerRender`
  discovery before restarting all passes. No manual bad-point registry was added.
- `ResetGpuTuning` is an explicit recovery escape hatch after TDR/interruption: it bypasses the normal
  start/apply lease, stops marked-running work, resets stock, clears Safe Loop, and releases the F2
  Forge handle after reset succeeds. It also removes the visible `forge_state.json` checkpoint so the
  UI can return to an idle/new-run state without deleting automatic F2 observation history. The Forge
  worker also catches panic/unwind so `running` is not left true forever.
- UI recovery is wired: after TDR/Needs Attention/Interrupted, **Recover & continue** calls
  `ResetGpuTuning` and then starts the selected Forge mode, preserving F2 observations for backend
  resume. **Full reset** is separate and maps to `ResetGpuTuningFull` with destructive confirmation.
- Crash, `SilentError`, `Unstable`, reset and Safe Loop behavior remain fail-closed.
- Code/unit validation only. No VF write, confirmed Forge, apply, reboot or GPU stress was executed.

## Latest backend/UI checkpoint (2026-06-28) — durable learning and visible progress
- Qualification refinement: frontier extended to 90% Cmax; next-clock warm-start is one real bin above
  the prior minimum with conservative ClockDrop fallback; Fast profiles are explicitly provisional.
- Standard = 10 s discovery + 2×60 s qualification; Long = 10 s discovery + 3×120 s qualification.
  Qualification failure backs off one physical bin and restarts all passes. UI and backend both block
  F2 Apply until qualified.
- The 18:08–18:27 Fast Forge preserved 72 observations (37 at 1935 MHz, 35 at 1920 MHz). The apparent
  loss was the UI restoring only the last complete forge state.
- Root cause of the stop before 1905 MHz: reset-clean SilentErrors incremented
  `consecutive_crashes`; the third-clock preflight then refused the run. Fixed so only DeviceLost/TDR
  increments crash streaks.
- Live progress now checkpoints each completed dwell and exposes structured current clock/voltage,
  completed/estimated steps, ETA, last outcome, learned count and frontier-complete status.
- Lower clocks warm-start at the prior target's last power-bound ClockDrop, with one conservative
  overlap. Technical log lines stream per candidate and remain visible after the run.
- Validation: `cargo test -p nidavellir-service` 311/311; `cargo check -p
  nidavellir-service`; `apps/ui npm.cmd run build`. No new hardware Forge was run.

## Latest backend checkpoint (2026-06-28) — integrated F2 frontier corrected; ready for supervised hardware QA
- Live path: `PowerSweepHandle::start_with_mode` → `measure_multiclock_undervolt_forge` →
  `run_confirmed_f2_clock_discovery` → the proven per-candidate
  arm→anchored-write→verify→dwell→checked-reset motor.
- Highest real clock is tried first. A pre-sustain clock drop near 99–100% power cap continues down
  voltage; off-cap failure advances the clock. First sustained target = Cmax. The outer loop then
  covers all real bins through 90% Cmax.
- Autonomous target/ladder/live discovery traverses the physical VF domain with no 3/6-step budget.
  Stop is the first terminal signal or hardware floor; `DeviceLost`/reset failure aborts the Forge.
- Fast/Standard/Long use the same frontier: provisional 10 s discovery / 2×60 s qualification /
  3×120 s qualification. Confidence uses actual dwell, samples, and independent validations.
- Observations are appended immediately and keyed by NVML UUID. Resume skips confirmed bins and
  known brackets. Only a complete Cmax→90% frontier can produce the active profile set.
- Safety: global IPC GPU lease, checked modern VF reset, Safe Loop arm-before-write, boot flag retained
  through crash accounting, no auto-apply.
- Validation: workspace check; core 64 / NVAPI 40 / service 309 tests; diff check clean. Read-only
  dry-run at 1950 MHz reports ceiling 1950 and 83 physical anchors (no 3/6/+210/+15 discovery cap).
  Repository-wide rustfmt remains a pre-existing baseline failure, so no broad rewrite was accepted.
- Hardware checkpoint protocol: operator present and prepared for TDR/reboot; start with Fast only if
  explicitly authorized; inspect Cmax, every per-clock stop reason, reset readback, Safe Loop state,
  observation checkpoints, and absence of profile persistence on interruption before Standard/Long.

## Latest backend checkpoint (2026-06-27) — F2 PHASE 2: Apply path wired to F2 anchored undervolt (code-complete, NOT HW-tested)
- **What**: the three Apply actions (`ApplyPowerGodforge/Brokkrs/DeepCalm`) now APPLY the F2 undervolt
  instead of refusing it. Closes the Phase-1 gap (forge produced F2 profiles that could not be applied —
  apply was attached to F1 flatten-down, the wrong op for an undervolt point).
- **Scope (operator-confirmed)**: WIRE the apply only; F2 is the main algorithm but **F1 was NOT removed**
  (still the live path for legacy `is_undervolt==false`). Advisory: `synthesize_forge_profiles` + `ForgePolicy`
  are SHARED by F2 — must stay; the now-dead F1 apply/forge code (`apply_core`/`apply_vf_ceiling`,
  `run_power_sweep`/`build_frontier`) is kept for a separate Phase 3 cleanup.
- **How (reuse, not reinvent)**: new `gpu_undervolt::apply_anchored_undervolt(target_mhz, anchor_mv)` reuses
  the proven `RealF2Ops` primitives for a one-shot apply (prev_offset=0): read live VF base →
  `select_anchor_bin` → `apply_bounded_anchored_positive_offset` → `verify_anchored_positive_offset`. ONLY
  `AnchoredRaiseVerified` leaves the curve applied; any miss/anchor-fail/writer-reject → `reset_to_stock` +
  per-bin readback confirm + `Err` (fail-closed, nothing left applied). `gpu_apply::apply_and_persist_undervolt`
  mirrors `apply_and_persist` (arm boot flag → write → persist → clear flag after 8 s survival window) and
  persists a new `AppliedProfile.undervolt: Option<UndervoltApply{target_mhz, anchor_mv}>` (`#[serde(default)]`,
  legacy JSON → None). `reapply_on_boot` branches on `undervolt` (F2 re-derives the anchored curve from the
  LIVE table and requires the exact validated anchor bin (missing bin → fail closed; never silently deeper);
  F1 unchanged). IPC: `ApplyPower*` route via `apply_forge_profile` on `prog.is_undervolt`
  (F2) else `apply_power_profile` (F1, unchanged); `refuse_undervolt_apply` removed.
- **Files**: `crates/service/src/{gpu_undervolt.rs, gpu_apply.rs, ipc_server.rs}`,
  `docs/contracts/ui-backend.md` (Backend→Frontend 2026-06-27: apply WIRED, UI must un-gate).
- **Apply axes from the forge point**: `target = target_clock_mhz ?? clock_mhz`,
  `anchor = vf_table_voltage_mv ?? voltage_mv` (`undervolt_apply_params`, unit-tested). Survives restart:
  whole `PowerSweepProgress` incl. `is_undervolt` round-trips through `forge_state.json` + seeds the handle.
- **Validation (no hardware)**: cargo check clean; tests **core 61 / nvapi 38 / service 300**; clippy
  ZERO new service warnings (the 3 introduced clone-on-Copy fixed;
  remaining 21 are pre-existing). NOT run: any apply / VF write / `--confirm` / hardware.
- **NEXT**: independent `nidavellir-safety-auditor` pass on the diff (recommended), then ONE supervised
  manual apply on the rig — forge F2, click Apply, confirm: anchored VF write verifies, `gpu_applied.json`
  carries `undervolt`, survives the 8 s window, reapplies on next service start; reset clears it.
- Integrated into the master closeout; final validation and publish proof are recorded in `memory.md`.

## Latest frontend checkpoint (2026-06-27) — Phase 2 F2 Apply un-gated; contract evidence wired
- F2 Godforge / Brokkr's / Deep Calm remain visible as **Discovered** until applied; Apply is now enabled
  and calls the unchanged `ApplyPower*` methods. The existing Active state takes over after success.
- Applied-state matching uses the F2 target clock + anchor carried by `GpuApplyStatus.core`; legacy F1
  matching remains unchanged. The read-only legacy curve verifier reports F2 as metadata-only rather than
  running the F1 flatten-down classifier against an anchored curve.
- `PowerSweepPoint.confidence` / `validation_count` and
  `PowerSweepProgress.power_bound_collapse` are now structured, backward-compatible payload fields.
- The Phase 2 safety closeout also resets to stock if memory-offset application fails after the F2 core
  write, preserving the contract that a failed apply leaves no F2 curve resident.

## Earlier backend checkpoint (2026-06-27) — FORGE → F2 UNDERVOLT pivot; Phase 1 DONE (e4bd006, pushed); next = Phase 2
- **Why**: 2 supervised HW runs proved the live button's F1 multi-clock forge COLLAPSES on the RTX 3060 Ti —
  it's pinned at 200 W, and F1 flatten-down can't lower power on a power-bound card (lowering a frequency
  ceiling does nothing when already power-capped). F2 anchored undervolt CAN: 1800 MHz @ 875 mV = 157 W vs
  200 W (−43 W, same clock). Operator call: forge's PRIMARY method pivots to F2. Full rationale + the verified
  gap analysis = top entry of `decisions.md`.
- **Reuse map (don't reinvent)**: motor `run_confirmed_f2_multi_step` (gpu_undervolt.rs:1362); ladder
  `run_anchored_ladder_sweep` (gpu_undervolt.rs:2657); synthesis bridge `learned_frontier` →
  `frontier_to_points`/`to_power_sweep_point` (f2_observation.rs:371/394) → `synthesize_forge_profiles`
  (gpu_power_sweep.rs:1284); persist `save_forge_state` (gpu_power_sweep.rs:340). F2 writer for Phase 2 apply:
  `apply_bounded_anchored_positive_offset` (gpu-nvapi). The GAP: F2 wired only to the CLI (gpu_undervolt.rs:1792);
  the button (`run_power_sweep`:3811) + Apply IPC (`apply_core`→`apply_vf_ceiling`, gpu_apply.rs:99) are F1.
- **F2 brings learning/memory F1 lacked**: observation store records every sweep across runs; learned_frontier
  accumulates; descent resumes from deepest validated; confidence grows with validations (resolves the Option-A
  IDLE gap).
- **Phase 1 — DONE + pushed (`e4bd006`)**: `measure_multiclock_undervolt_forge` (gpu_power_sweep.rs:4100)
  drives the F2 motor per clock via `run_confirmed_f2_clock_descent` (gpu_undervolt.rs:2863, reuses the motor
  unchanged) → `learned_frontier` → `frontier_to_points` → `synthesize_forge_profiles` → 3 profiles → persist.
  Button routed via `start_with_mode` (gpu_power_sweep.rs:253). Apply GATED: additive `is_undervolt` flag
  (ipc.rs) + `refuse_undervolt_apply` (ipc_server.rs:384) in all 3 ApplyPower* handlers. F1 `run_power_sweep`
  kept `#[allow(dead_code)]`. Mode → clock breadth only. Verified: tests core 61 / nvapi 38 / service 296;
  safety audit GO; reset-on-every-path; no auto-apply. Contract note implemented by Codex: the UI shows F2
  profiles as Discovered and disables Apply in Phase 1. **NOT hardware-tested** — a supervised button run
  on the rig is safe (apply gated) and
  is the recommended first check next session.
- **Phase 2 (NEXT — start here)**: wire the F2 writer `apply_bounded_anchored_positive_offset` (gpu-nvapi,
  today called only from gpu_undervolt.rs:1792) into the Apply IPC (`apply_power_profile`/`apply_core` route
  F2 picks to the anchored-offset writer instead of the F1 `apply_vf_ceiling`), with Safe Loop arm/verify/
  persist/reapply-on-boot; then flip `refuse_undervolt_apply` to allow F2. RISKIEST piece → own safety audit +
  supervised HW run before shipping. **Phase 3**: fold Fast/Long modes into F2 depth, reapply-on-boot, retire
  F1 button path.
- **Git note**: abandoned F1 knee-seeking commit `cc8710a` dropped (F1-specific, moot under F2; reflog-recoverable
  in the diagnosis worktree). The F2-pivot decision/plan is re-recorded here + `decisions.md` (the diagnosis
  session's local plan commit `02e07c2` was not pushed; its substance is preserved here).

## Earlier backend checkpoint (2026-06-26) — F1b BUTTON MODES (Fast / Standard / Long) — committed 3c82e96 + pushed; HW test pending
- **What**: DEFERRED #1 from the Option-A checkpoint — the live power-sweep button gains TWO new modes
  around the proven Standard run. FAST = quick discovery (fewer probes, shallower); LONG = broader+deeper
  discovery + repeated per-pick ceiling soaks (confidence built in ONE session, no IDLE wait). The
  multi-clock analogue of F2's `--validation-passes` depth knob.
- **STATUS: committed `3c82e96` + pushed to master** — cargo check clean; `core 59 / nvapi 38 / service 293`
  tests pass (+1 new); clippy ZERO new warnings (the two `too_many_arguments` are pre-existing
  `build_frontier_two_phase` / `real_probe_step`). NO hardware run yet — one supervised test of the button is
  pending (see MANUAL TEST PATH).
- **Design — Standard is byte-identical to the just-HW-validated button** (pinned by a new test
  `power_sweep_mode_tuning_preserves_standard_and_bounds_fast_long`): the plain `StartPowerSweep` IPC still
  maps to `BUTTON_MAX_PROBES=24` / per-target `3` / one 35 s ceiling soak. FAST = `12 / 2 / 1` (≈half the
  discovery). LONG = `40 / 4 / 3` (more clocks + one deeper bin + 3× ceiling soaks per pick). All knobs are
  named consts in `gpu_power_sweep.rs` (easy to tune).
- **Backend**: `PowerSweepMode {Fast,Standard,Long}` enum + `mode.tuning() -> (max_probes,per_target,passes)`;
  `PowerSweepHandle::start_with_mode` (plain `start` delegates to Standard); `run_power_sweep(.., mode)` builds
  `FrontierLimits` from the mode and loops the ceiling soak via new `validate_pick_ceiling_passes` (fail-closed
  on ANY pass; passes clamped to `POWER_SWEEP_MAX_VALIDATION_PASSES=5`). Mode label + per-profile pass count
  surfaced in `note`/`log` text only (no payload change).
- **IPC (additive, backward-compatible)**: two NEW unit methods `StartPowerSweepFast` / `StartPowerSweepLong`
  in `core/ipc.rs` + dispatch arms in `ipc_server.rs`. `StartPowerSweep` UNCHANGED (= Standard). No payload/
  field change → no contract break. UI toggle documented for Codex in `docs/contracts/ui-backend.md` (2026-06-26
  request), realising the `validation_passes` "IPC parameter when wired" the 2026-06-23 entry anticipated —
  delivered as a BOUNDED mode, not a free-form integer.
- **Safety (self-audit)**: `apply_vf_ceiling_monotone` / Safe Loop arm-clear / `reset_to_stock` / verifier /
  the probe + soak motor are all UNTOUCHED. The per-pick fail-closed 35 s ceiling soak runs ≥1× in EVERY mode
  (no weakening). FAST only REDUCES exposure. LONG adds MORE probes/soaks of the SAME bounded fail-closed
  motor — a longer supervised run, no new risk class; the global `max_probes` stays a hard cap; extra passes
  can only REJECT a marginal pick, never widen it. NO auto-apply (apply stays the separate `ApplyPower*` IPC,
  "confirme em jogo"); persist still only when `godforge.is_some()`. An independent `nidavellir-safety-auditor`
  pass is recommended before any confirmed LONG hardware run.
- **Files**: `crates/core/src/ipc.rs`, `crates/service/src/{gpu_power_sweep.rs, ipc_server.rs}`,
  `docs/contracts/ui-backend.md`.
- **DEFERRED (unchanged)**: #2 IDLE / cross-run multi-clock confidence accumulation (operator: later).
- **MANUAL TEST PATH (hardware, operator-present)**: send `StartPowerSweepFast` or `StartPowerSweepLong` (or
  plain `StartPowerSweep` for Standard). Watch `note`/`log` for the mode label, `est_wall_s`, and (LONG)
  `passagem i/N` lines. Restores stock at the end; persists `forge_state.json` only on a usable profile.
- **TO RESUME**: code is committed + pushed; remaining = (a) one supervised hardware test of the button on the
  test rig (confirm multi-clock button → 3 profiles → apply end-to-end; LONG also exercises multi-pass ceiling
  validation), then (b) DEFERRED #2 (IDLE / cross-run). Optional independent safety audit before the HW run.

## Earlier backend checkpoint (2026-06-23) — OPTION A: live power-sweep BUTTON rewired to the multi-clock forge algorithm — pushed
- **What**: the live "forjar/refinar" button (`StartPowerSweep` → `run_power_sweep`, `crates/service/src/gpu_power_sweep.rs`)
  was SINGLE-CLOCK; it now runs the MULTI-CLOCK forge algorithm (the proven `build-frontier` core) and produces the
  3 DIFFERENTIATED profiles (Godforge / Brokkr's 95% / Deep Calm 90%) via `synthesize_forge_profiles`.
- **STATUS: committed + pushed** — commit `ba48c7c` (code, +423/−380) on top of Codex's UI commit `837ab4c`; this
  handoff is the follow-up docs commit. Verified: cargo check clean; tests `nvapi 38 / core 59 / service 292` pass;
  clippy no new warnings; `build-frontier` dry-run byte-behavior-preserved (hardware-relative targets, floor
  discovered). Independent safety audit = GO-with-changes, and BOTH flagged fixes were applied (below). NO hardware
  run yet — the button has NOT been exercised on real hardware (see MANUAL TEST PATH).
- **How it works**: extracted `measure_multiclock_forge(store, stop, limits) -> Option<MultiClockForgeResult>` (the
  confirmed `build-frontier` core: derive_core_seed → regime(PowerLimited) → hw_floor → `candidate_clocks` (hardware-
  relative, NO fixed MHz) → derive_descent → plan_frontier → real_probe_step probe → build_frontier_two_phase →
  synthesize). `run_build_frontier` refactored to call it (dry-run output byte-identical). `run_power_sweep` calls it
  with BUTTON-default `FrontierLimits` (`BUTTON_MAX_PROBES=24`, `BUTTON_MAX_PROBES_PER_TARGET=3`, all else off),
  surfaces `est_wall_s` BEFORE the run (~8–12 min), maps the 3 profiles, validates each pick, persists on success.
- **Two safety fixes applied after the audit**:
  1. APPLY-AXIS: picks come from `probe_to_point` with `voltage_mv=0` (undervolt is in `vf_table_voltage_mv`); the
     Apply path keys on `voltage_mv` → `choose_ceiling_mv(curve,0)` = LOWEST bin = WRONG deepest undervolt. FIX:
     backfill `voltage_mv = vf_table_voltage_mv` on the 3 picks before validate/persist (`run_power_sweep` ~3814).
  2. VALIDATION FIDELITY: new `validate_pick_at_ceiling` (~3613) soaks each pick AT ITS DISCOVERED CEILING (arm Safe
     Loop → `apply_vf_ceiling_monotone(vbin, clk)` → read-only verify → 35 s game-power soak → reset+clear on EVERY
     path → fail-closed DROP the pick on any instability; no back-off). Multi-clock picks route here; `None`-ceiling
     legacy picks keep `arduous_validate`.
- **Safety invariants (verified)**: reset-to-stock on EVERY exit path (start / fail-closed `None` / post-validate);
  persist (`save_forge_state`) ONLY when `godforge.is_some()`; NO auto-apply (apply stays the separate `ApplyPower*`
  IPC, "confirme em jogo"); NO IPC contract change (`deep_calm` becoming `Some` is additive); `apply_vf_ceiling_monotone`
  / F1 / Safe Loop untouched.
- **HONEST CORRECTION on IDLE / learn-over-time**: the multi-clock frontier confidence is PER-RUN (in-run telemetry
  quality `s.confidence`), NOT cross-run. The single-clock `GpuKnowledge` (cross-run trials → `gpu_knowledge.json`)
  was the only cross-run learner and was REMOVED from the button (offset-keyed, single-clock-specific; struct kept
  test-only). So the button does NOT learn across runs today; cross-run/IDLE accumulation for the multi-clock
  frontier is a FUTURE item (operator confirmed IDLE = later, not priority).
- **DEFERRED — next-session work (operator's plan)**:
  1. TWO BUTTON MODES: a **LONG** run (everything + the bigger validations in ONE session — skip IDLE, for users who
     want it all up front) and a **FAST** run (quick discovery; leave confidence-building to IDLE / later manual runs).
     This is the multi-clock analogue of the `--validation-passes` depth knob; needs a UI toggle (→ Codex contract) +
     a backend depth parameter (likely vary `BUTTON_MAX_PROBES`/`max_probes_per_target` + per-pick soak passes).
  2. IDLE / cross-run learning: accumulate stability confidence across runs on the multi-clock frontier (keyed by
     clock+voltage, persisted), feeding the Wilson gate — the future auto-runs-when-idle scheduler builds on this.
- **MANUAL TEST PATH (hardware, operator-present)**: click the live power-sweep button (`StartPowerSweep`) to run the
  multi-clock forge end-to-end; OR CLI `build-frontier --confirm` (discovery only, no persist). The button shows the
  duration estimate first, then 3 differentiated profiles, restores stock at the end, persists `forge_state.json` only
  on success. Capture service log lines `multiclock-forge:` / `build-frontier probe:` / `Validação árdua`.
- **TO RESUME**: pick up the two button modes (#1) — the smaller, higher-value next step — before IDLE (#2). Optional
  before that: one supervised hardware run of the button to confirm the multi-clock flow end-to-end on the test rig.

## Earlier backend checkpoint (2026-06-23) — F2 multi-clock profile package (Brokkr's 0.95 + descending ladder + confidence opt-in) — pushed
- **What**: 3 approved changes toward the v0.5 multi-clock profile frontier. Implemented by the code-surgeon,
  independently validated + safety-audited (GO). No hardware run. Committed + pushed to `origin/master`:
  `f065d4a` (code) + `79c3081` (docs).
- **THE margin answer**: applied-voltage conservatism (e.g. 906 mV vs the 868 the sweep reached) is the **Wilson
  confidence gate** (0.85), NOT a margin. `synthesize_forge_profiles` selection is voltage-agnostic; a once-validated
  deep point has confidence ~0.21 and is filtered until it earns repeat confirmations.
- **Part 1**: `ForgePolicy::balanced` Brokkr's floor 0.98→0.95 (Deep Calm 0.90, gate 0.85 unchanged) — selection
  only. 3 floor tests decoupled to explicit 0.98; new test pins 0.95.
- **Part 2 (Caminho B)**: `ladder_target_descent_bounds` makes `run_anchored_ladder_sweep` direction-aware —
  DESCENDING starts at the prior clock's last-good (ceiling) with the base floor (each lower clock finds its own
  deeper min-V); ASCENDING unchanged. Confirmed loop chains `prev_good` forward.
- **Part 3**: `--validation-passes N` (default 1, cap 20) confidence opt-in for `--auto-sweep` — re-validates ONLY
  the deepest validated point up to N-1 extra times (reuses the safe motor + per-pass precheck, stops on any
  non-Validated, records 1 obs/pass). Default 1 = no-op. Mode 1 kept; idle-validation = FUTURE.
- **UI contract**: `docs/contracts/ui-backend.md` (2026-06-23) — multi-clock profiles, Brokkr's 95%, honest
  collapse, confidence-is-a-gate messaging, "Build confidence now" opt-in (default OFF), idle future.
- **Validation**: nvapi 38 / core 59 / service 292 pass; clippy no new warnings; dry-runs confirm default +30 &
  manual-prior +250 unchanged, auto-sweep shows +210 horizon + validation-passes line, descending ladder plans
  per target. Safety audit = GO (8/8 PASS).
- **Files**: `crates/service/src/{gpu_power_sweep.rs, gpu_f2_sweep.rs, gpu_undervolt.rs}`, `docs/contracts/ui-backend.md`.
- **MANUAL TEST PATH (available now, no button needed)**: everything above is in the service CLI. Read-only
  dry-runs (safe, no hardware): `undervolt-probe --target-mhz 1800 --auto-sweep [--validation-passes N]` and
  `undervolt-probe --ladder-sweep --targets 1830,1815,1800,1750,1700` — both print the "classifier bridge"
  block (`synthesize_forge_profiles` with Brokkr's 0.95, read-only). On the current single-clock store it shows
  the honest collapse. To see the 3 profiles DIFFERENTIATED you need multi-clock data → run the confirmed
  descending ladder (`… --confirm`, HARDWARE, operator-present, never HW-run before), then re-run the dry-run.
- **DEFERRED — Option A: wire the live forge/refine button to the multi-clock algorithm (F1b Phase 2)**. The
  operator deferred this until usage limit returns. Goal: the button (`StartForgeAll` → the live power-sweep at
  `crates/service/src/gpu_power_sweep.rs:3770`) should run a MULTI-CLOCK sweep (a few descending clocks anchored
  at the validated top) and select via `synthesize_forge_profiles(&frontier, &ForgePolicy::balanced())` instead
  of today's single-clock `select_brokkrs_v2` (line 3799) + max-voltage Godforge (3788) + Deep Calm=None (3802).
  Key facts for resuming cold: (a) the live forge is SINGLE-CLOCK today, so a naive selector-swap would COLLAPSE
  (`distinct_clocks<=1`) and DEGRADE the working button + change the applied/persisted profile — it must become a
  real multi-clock sweep; (b) types match (`ForgeProfiles` fields are `Option<PowerSweepPoint>`, same as the
  payload) → NO IPC contract change; (c) it makes the button a LONGER hardware run and touches apply/persist
  (`ApplyGodforge`, `save_forge_state`) → needs care + a safety audit before any confirmed run. Until then, the
  CLI manual-test path above is the way to exercise the new algorithm.

## Earlier backend checkpoint (2026-06-22) — F2 LEARNED OFFSET HORIZON implemented (+210 abs / +15 step); HW run HELD
- **What**: target-sweep-specific progressive absolute-offset horizon. Commit `c40a78d`
  (`feat(service): add f2 target sweep learned offset horizon`), pushed to `origin/master`. No hardware run executed.
- **Change**: gpu-nvapi gains `TARGET_SWEEP_HORIZON_MAX_MHZ = +210` + `PositiveOffsetLimits::target_sweep_learning_horizon(floor, ceiling)`
  (abs +210, per-step STILL +15 — unlike `manual_prior`, which widens both). Only the `--auto-sweep` dispatch in
  `gpu_undervolt.rs` builds it; default/ladder/manual-prior keep `conservative` (+30/+15). `gpu_f2_sweep.rs` dry-run
  names the horizon cap + shows per-candidate step delta. 8 new tests. Files: `crates/gpu-nvapi/src/lib.rs`,
  `crates/service/src/gpu_undervolt.rs`, `crates/service/src/gpu_f2_sweep.rs`.
- **Validation**: cargo check clean; gpu-nvapi 38 / core 59 / service 284 tests pass; clippy zero new warnings;
  independent safety audit = **GO** (all 11 items PASS; no unsafe clock/floor bypass; no single +210 jump — ~14
  validated +15 steps; confirmed sweep still bounded by `F2_CONFIRMED_MAX_STEPS`=3; no profile persist).
- **Dry-runs (no --confirm)**: default still `abs +30 / +15` (unchanged); manual-prior still `+250` (unchanged);
  `--auto-sweep` shows `abs +210` horizon, resumes from prior validated 962 mV/+30, PLANS 6 candidates continuing
  below 962 (#4 962/+45, #5 956/+45, #6 950/+60; each step Δ ≤ +15).
- **Why the HW run was HELD (operator choice)**: today's live curve has THREE bins within +30 at the top
  (981/+15, 975/+15, 968/+30), so a confirmed run — capped at 3 candidates, descent restarting from the curve top —
  would reach only **968 mV** (shallower than the 962 frontier) and would NOT advance discovery. The +30 cap is NOT
  today's binding limit; the 3-step budget + descent-start is. Spending a TDR-risk run to re-validate known-good
  points is poor value. (Safety auditor independently flagged the same — its C1.)
- **State (untouched)**: no `--confirm` run; observation store still 8 records / `last_good 962 mV` / `first_bad None`;
  no `gpu_applied.json` / `boot_flag.json`; no profile apply/persist/promotion. Implementation pushed to `master`
  (worktree branch `claude/adoring-lewin-2a7c8b`); tree clean after the docs commit.
- **NEXT**: scoped, separately-reviewed follow-up — let the confirmed sweep RESUME ITS DESCENT START near the
  validated baseline (skip already-validated shallow bins) so the deep candidates (962/+45, 956, 950) fall within
  the 3-step budget; THEN one supervised confirmed run actually advances the frontier. Alt: bounded LADDER over
  1815/1830.

## Earlier backend checkpoint (2026-06-22) — F2 1800 MHz second confirmed chained run; frontier saturated at +30 cap — PASS
- **What**: third confirmed official target sweep (`undervolt-probe --target-mhz 1800 --auto-sweep --confirm`)
  at HEAD `01b97ca`. One confirmed command, operator present. No code change (hardware validation only).
- **Result — PASS** (exit 0): **3/3 Validated**, `CompletedAllPlanned`. #1 981/+15 (1815/1815, 191 W),
  #2 975/+15 (1803/1800, 198 W), #3 968/+30 (1815/1815, 193 W). All reset + boot-flag cleared; no TDR/crash/
  DeviceLost/Unstable/ClockDrop. `first_bad None`, frontier updated, ended safe.
- **Key finding**: the 1800 MHz conservative sweep is now **absolute-cap-bounded**. This session's VF read sat
  higher (boost top 1935), so the deepest reachable bin within the **+30 abs cap** was 968 mV/+30; the next
  needs +45 → fail-closed. The chained baseline relaxes only the PER-STEP cap, never the ABSOLUTE cap, so it
  can't push below ~962 mV. `last_good` stays **962 mV** (prior run's deeper point). The frontier has hit its
  conservative floor — re-running 1800 only adds confidence/observations.
- **State after run (all safe)**: `gpu_applied.json`/`boot_flag.json` ABSENT; `forge_state`/`gpu_knowledge`/
  `heartbeat`/`safe_loop` byte-identical; `f2_observations.jsonl` 5→8 (7 validated + 1 preserved abort). git
  clean.
- **NEXT**: stop re-running 1800 (saturated); start a bounded LADDER over additional targets (1815/1830) to
  build the real multi-clock frontier — supervised, one confirmed run at a time.

## Earlier backend checkpoint (2026-06-22) — F2 CHAINED DESCENT refinement + first FULL-descent HW run (1800 @ 962 mV) — PASS
- **What**: the planner refinement the PASS-PARTIAL run called for, committed `fcdf04d`
  (`feat(service): refine f2 target sweep descent baseline`), then its first confirmed hardware run
  `undervolt-probe --target-mhz 1800 --auto-sweep --confirm`. One confirmed command, operator present.
- **Fix**: observation-aware chained same-target descent. The confirmed motor bounds each candidate's per-step
  increase against the LAST VALIDATED offset (prior candidate this run — only reached after it validated — or
  the deepest prior validated same-target/same-GPU observation for candidate 0; 0 when none), not stock +0.
  The absolute +30 cap still bounds each candidate's absolute offset. Files: `crates/core/src/f2_observation.rs`
  (`validated_descent_baseline` + tests), `crates/service/src/gpu_undervolt.rs` (`chained_prev_offset`,
  `RealF2Ops.prev_offset_mhz`, `RealF2MultiOps.baseline_offset_mhz`, `run_anchored_target_sweep`),
  `crates/service/src/gpu_f2_sweep.rs` (dry-run "chained baseline" line). **gpu-nvapi / apply_vf_ceiling_monotone
  / verifier / manual-prior UNCHANGED.** Tests: core 59/0, service 282/0, gpu-nvapi 33/0.
- **Result — PASS** (exit 0): **3/3 Validated**, `CompletedAllPlanned`. #1 975/+15 (avg 1803, p5 1770, 198 W),
  #2 968/+15 (1800/1800, 190 W), #3 **962/+30** (1800/1800, 191 W) — the +30 point that aborted before now
  validates. Min stable voltage **962 mV** (was 975), `first_bad None`, frontier updated, ended safe.
- **State after run (all safe)**: reset + boot-flag cleared for all 3; `gpu_applied.json`/`boot_flag.json`
  ABSENT; `forge_state`/`gpu_knowledge`/`heartbeat`/`safe_loop` byte-identical (no persist/apply/promote, no
  new blacklist); `f2_observations.jsonl` 2→5 records (prior 2 incl. the old abort preserved). `git` clean.
- **NEXT**: bounded LADDER over multiple targets (real multi-clock frontier), supervised, one confirmed run
  at a time.

## Earlier backend checkpoint (2026-06-22) — F2 OFFICIAL target sweep FIRST HARDWARE RUN (1800 @ 975 mV) — PASS-PARTIAL
- **What**: first bounded hardware run of the OFFICIAL F2 target sweep (progressive anchored descent, NOT
  manual-prior): `undervolt-probe --target-mhz 1800 --auto-sweep --confirm` at HEAD `8dbd296` (freshly-built
  debug binary). One confirmed command, operator present, no second run.
- **Result — PASS-PARTIAL** (exit 0): #1 **Validated** 975 mV / base 1785 / +15 → 1800; **RaiseVerified**;
  dwell **Stable** avg/p5 **1815 MHz**, **191 W**. #2 **aborted_by_safety_gate** (planner per-step +30 > +15
  cap; **no VF write**, `not_run`). `last_good=975`, `first_bad=None`, frontier updated. No TDR/DeviceLost/
  Unstable/ClockDrop/reboot.
- **State after run (all safe)**: `reset_to_stock_ok` + `boot_flag_cleared` true for both candidates;
  `gpu_applied.json`/`boot_flag.json` ABSENT; `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt`
  byte-identical; `safe_loop.json` content unchanged (`safe_mode=false`, blacklist 4 entries unchanged). 2
  observations appended to the now-existing `f2_observations.jsonl`. `git` clean.
- **Key finding (algorithm, NOT changed)**: each candidate starts from stock (+0); with the +15 per-step cap,
  only base-within-+15 (1785) is reachable, so the deeper anchors (base 1770, +30) self-abort and the 1800
  sweep validates ONE point per run. To bracket the min stable voltage the planner must carry the prior
  validated offset as the next baseline (or widen the same-target descent step) — a future reviewed task.
- **NEXT**: planner refinement for same-target descent, then re-run the 1800 sweep to bracket below 975 mV.

## Earlier backend checkpoint (2026-06-22) — F2 discovery/learning algorithm IMPLEMENTED (not yet HW-validated)
- **What**: the four-block F2 discovery/learning algorithm. **Code + tests + docs only — no hardware, no
  `--confirm`, no VF write, no profile apply/persist/promote.** Commits `0df6179` (store + target sweep),
  `cb125b6` (ladder + learned frontier).
- **Files**: `crates/core/src/f2_observation.rs` (NEW — pure store/queries/frontier/bridge),
  `crates/core/src/lib.rs` (module), `crates/service/src/gpu_f2_sweep.rs` (NEW — mapper/recorder/
  formatters/ladder helpers), `crates/service/src/gpu_undervolt.rs` (CLI args/parse/dispatch +
  run_anchored_target_sweep + run_anchored_ladder_sweep + usage), `crates/service/src/gpu_power_sweep.rs`
  (classify_f2_frontier_summary read-only classifier bridge), `crates/service/src/main.rs` (module).
- **CLI**: `undervolt-probe --auto-sweep` (same-target min-stable-voltage discovery; official progressive
  caps; bounded by F2_CONFIRMED_MAX_STEPS; records observations on --confirm only) and `--ladder-sweep
  --targets a,b,c` (multi-target; lower target's last-good used only as a conservative FLOOR; stops on
  safety failure). Dry-run default; both write nothing.
- **Learning**: observations → `f2_observations.jsonl` (JSONL, append-only, learning data NOT a profile);
  `learned_frontier()` derives per-target best/first-bad/bracket; `classify_f2_frontier_summary` bridges to
  the EXISTING `synthesize_forge_profiles` (read-only preview of Godforge/Brokkr's/Deep Calm — nothing
  applied). last_good = lowest validated anchor; first_bad = highest failure; instability that resets clean
  is learning data, not a safety failure (only ResetFailed/crash stops a sweep/ladder).
- **Untouched**: default progressive + manual-prior; F1/build-frontier; apply_vf_ceiling_monotone; Safe
  Loop; reset_to_stock; verifier; synthesize_forge_profiles (reused). v1 GPU-only; CPU/RAM/UI deferred.
- **Validated (no HW)**: core 56/0, service 278/0, nvapi 33/0; clippy clean; all dry-runs write nothing
  (`f2_observations.jsonl`/`boot_flag.json`/`gpu_applied.json` absent).
- **NEXT**: first bounded hardware run of the official target sweep — `undervolt-probe --target-mhz 1800
  --auto-sweep --confirm` (operator present). NOT another manual validation.

## Earlier checkpoint (2026-06-21) — F2 MANUAL-PRIOR anchor mode HARDWARE VALIDATED (1800 @ 875 mV, +210) — PASS
- **What**: opt-in `--manual-prior` for `undervolt-probe` — anchors at an explicit `--start-mv` with a
  SEPARATE larger bounded offset cap to validate a KNOWN point fast (`1800 MHz @ 875 mV`). NOT the default,
  NOT for unknown GPUs. **Code + tests + docs only — no hardware, no `--confirm`, no VF write.**
- **Files**: `crates/gpu-nvapi/src/lib.rs` (+`PositiveOffsetLimits::manual_prior`),
  `crates/service/src/gpu_undervolt.rs` (planner + formatter + refusal + dispatch + 13 tests),
  `crates/service/src/main.rs` (doc comment only).
- **Default unchanged**: progressive anchored descent + conservative caps (+30/+15) are the official
  unknown-GPU path. Manual-prior branches BEFORE the default dispatch (gated on `args.manual_prior`).
- **Cap**: `F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ = 250` (default `+30` untouched). Fail-closed: an
  offset above the cap is REFUSED, never clamped; the stock clock ceiling still caps the effective clock.
- **Gates**: `--manual-prior` requires `--start-mv`; confirmed requires `--steps 1` (delegates to
  `confirmed_f2_refusal`); confirmed reuses `run_confirmed_f2_step`/`RealF2Ops` with `manual_limits`; one
  candidate; no persist/apply/promote. F1/`apply_vf_ceiling_monotone`/Safe Loop/reset/verifier untouched.
- **Dry-run `1800 @ 875`**: selected **875 mV**, base **1590 MHz**, required **+210 MHz**, cap **+250**,
  within bounds, **AnchoredRaiseVerified**, no-op/no-write. Default `1800 --steps 3` unchanged (975/968/962).
- **Tests/review**: 269 service + 33 nvapi tests pass; manual safety review no blockers. Implementation
  commit `34581d0`.
- **HARDWARE PASS (one confirmed run, operator present)**: `undervolt-probe --target-mhz 1800 --start-mv
  875 --steps 1 --manual-prior --confirm` → exit 0, outcome **Validated**. Anchor 875 mV / base 1590 /
  **+210 → 1800**; verify **AnchoredRaiseVerified**; dwell **Stable** avg/p5 **1815 MHz** at **157 W** (~26 W
  under the 975 mV/183 W run); `reset_to_stock` OK (all bins cleared); boot flag cleared; not blacklisted;
  **no persist/apply/promote** (`last_validated` null). No TDR/crash/reboot. `safe_loop.json` byte-identical
  (mtime-only); `boot_flag.json`/`gpu_applied.json` absent.
- **NEXT**: clocks above 1800 at 875 mV are NOT assumed (discover progressively). Options: descend below
  875 mV for 1800 (minimum stable voltage), or progressive discovery for 1815+. No second confirmed run was
  made.

## Earlier checkpoint (2026-06-21) — F2 ANCHORED MULTI-STEP descent IMPLEMENTED (not yet HW-validated)
- **What**: bounded SAME-TARGET ANCHORED multi-step descent for `undervolt-probe`. `--steps 2..=3` (anchored)
  executes a short sequence of anchored candidates at ONE target, safer/higher voltage → lower voltage,
  STOPPING at the first non-stable candidate and keeping the last good point. **Code + tests + docs only —
  no hardware, no `--confirm`, no VF write, no Safe Loop mutation outside tests.**
- **Files**: `crates/service/src/gpu_undervolt.rs` (planner + orchestrator + trait + refusal + formatters +
  RealF2MultiOps + 12 tests), `crates/service/src/main.rs` (doc comment only).
- **Step cap**: `F2_CONFIRMED_MAX_STEPS = 3`, enforced by `confirmed_f2_multi_refusal` (`--steps` 1..=3 else
  FAIL CLOSED). `--steps 1` = the validated single-step path (untouched). `--simple` = single-step only.
- **How it works**: `plan_anchored_undervolt_descent` (anchored analog of `plan_undervolt_probe`; chains the
  +15 per-step cap, stops at first rejection) → `run_confirmed_f2_multi_step` drives the SAME validated
  `run_confirmed_f2_step` motor per candidate via the `F2MultiStepOps` candidate-cursor trait. `select(i)`
  re-checks Safe Loop + blacklist before each write. Continues only on a stable `Validated` candidate
  (dwell stable + reset confirmed + flag cleared); stops on `VerifierFailed`/`Unstable`/`DeviceLost`/
  `ClockDrop`/`ResetFailed`/`Blacklisted`. New `F2DwellOutcome::ClockDrop` (p5 < target − 30 MHz on an
  otherwise-stable dwell) stops the descent; additive — single-step Stable/Unstable/DeviceLost unchanged.
- **Validated (no hardware)**: 256 service tests + 33 nvapi tests pass (incl. F1/build-frontier + single-step).
  Dry-run `--target-mhz 1800 --steps 3` → 3 candidates (975 mV +15 → 968 mV +30 → 962 mV +30, stop=budget),
  preflight OK, no-op line; `--help`/`--steps 1` unchanged; `boot_flag.json`/`gpu_applied.json` absent.
- **NEXT (hardware, one confirmed run, operator present, stop after first non-stable)**:
  `undervolt-probe --target-mhz 1800 --steps 3 --confirm`. No persist/apply/promote. NOT yet HW-validated.

## Earlier checkpoint (2026-06-21) — F2 ANCHORED undervolt FIRST CONFIRMED HARDWARE VALIDATION — PASS
- **One supervised confirmed run** (operator present, ONE confirmed command, no second) validating the `747a11b`
  anchored branch on real hardware: `undervolt-probe --target-mhz 1800 --steps 1 --confirm`. **First real ANCHORED
  positive-offset VF write.** HEAD = origin/master = `747a11b`, tree clean. **Fresh worktree binary built first**
  (`cargo build -p nidavellir-service`) — `target/debug/nidavellir-service.exe` was ABSENT; built mtime newer than
  `747a11b`.
- **Preflight PASS**: tree clean; `gpu_applied.json`/`boot_flag.json` absent; `safe_mode=false`;
  `boot_flag_armed=false`; `consecutive_crashes=1`; planned anchored point NOT blacklisted. Help = usage only;
  dry-run = mode **ANCHORED**, exactly ONE candidate + no-op line (no arm/apply/dwell/VF write).
- **Result: exit 0, outcome `Validated`.** No TDR / black-screen / reboot / DeviceLost / Unstable / silent error.
- **Anchored candidate (live curve)**: target **1800 MHz**, anchor bin **975 mV**, base **1785 MHz**, offset
  **+15 MHz** → 1800; **27** higher-voltage bins capped DOWN to 1800 (max flatten **-150 MHz**), **59** lower bins
  elastic. Within +15 step / +30 abs caps. (Earlier dry-run had read 981 mV / 25 / -135 / 61; the live curve at
  confirm time put the +15 anchor at 975 mV — 981 mV was already at base 1800 → capped +0.)
- **Sequence (motor end-to-end)**: Safe Loop armed BEFORE write → `apply_bounded_anchored_positive_offset` applied →
  `verify_anchored_positive_offset` = **`AnchoredRaiseVerified`** → dwell **Stable** (avg **1815 MHz**, p5 **1815 MHz**,
  **183 W**, no silent error) → `reset_to_stock` ran + CONFIRMED stock (all written bins cleared) → boot flag cleared
  after clean reset. Not blacklisted. **No profile persisted/applied/promoted** (Validated reported only).
- **Post-run**: `boot_flag.json`/`gpu_applied.json` absent; `safe_loop.json` **byte-identical** (sha256 unchanged,
  mtime touched only); `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; tree clean; HEAD `747a11b`.
- **Boost constrained vs prior SIMPLE F2**: simple run boosted elastically above target (avg **1868**, p5 **1845**,
  **199 W**); anchored run pins a flat plateau (avg **1815** = p5 **1815**, **183 W**, ~**16 W** lower). avg==p5
  confirms the plateau caps prevent boost above 1800; the +15 over target is within the 15 MHz verifier tolerance.
- **Meaning**: the F2 ANCHORED-undervolt HARDWARE path is PROVEN at one bounded point — the classic `MHz @ mV`
  undervolt SHAPE (anchor raise + plateau cap + elastic lower bins) holds on real hardware and the
  **arm → write → verify → dwell → reset → clear** motor is recoverable. First result that directly supports the
  intended method (map stable voltage per clock → repeat across clocks → synthesize Godforge / Brokkr's Best /
  Deep Calm). Does NOT yet prove the MINIMUM stable voltage for 1800 MHz.
- **Next (do NOT immediately run another confirmed command before this record is committed)**: a bounded, supervised,
  same-target **MULTI-STEP** anchored probe at 1800 MHz descending voltage until verifier fail / instability / clock
  drop / floor / budget, with the same Safe Loop / verification / reset guarantees. Detail in `decisions.md` (top).

## Latest backend checkpoint (2026-06-21) — F2 ANCHORED undervolt planning IMPLEMENTED (no hardware)
- **What**: F2 moves from a single positive offset at one VF bin to a true CLASSIC anchored undervolt
  point. The planner now RAISES the anchor bin to target AND CAPS every higher-voltage bin DOWN to the
  same target (≤ 0 offsets), leaving lower bins elastic. **ANCHORED is the DEFAULT** mode; `--simple`
  keeps the old single-bin descent. Code + tests + docs only — **no `--confirm`, no VF write, no Safe
  Loop mutation, no build-frontier, no stress, no power sweep.**
- **Why**: the prior confirmed F2 run (below) proved the positive-offset MOTOR but was NOT anchored — the
  GPU still boosted ABOVE the 1800 MHz target (dwell avg 1868). Classic `MHz @ mV` undervolt must test an
  anchored curve point, not one raised bin with the boost curve still free above it.
- **New symbols** (all SEPARATE from F1/build-frontier; `apply_vf_ceiling_monotone` UNTOUCHED):
  `plan_bounded_anchored_positive_offset` / `apply_bounded_anchored_positive_offset` /
  `AnchoredPositiveOffsetPlan` (gpu-nvapi, the anchor reuses the bounded single-bin planner →inherits all
  caps/floor/ceiling rules); `verify_anchored_positive_offset` / `AnchoredOffsetVerification` (gpu_verify);
  `UndervoltMode` / `plan_anchored_undervolt` / `select_anchor_bin` / `anchored_plan_lines` /
  `run_anchored_undervolt_probe` (gpu_undervolt). `RealF2Ops` gained `mode` + `anchored` (writes the full
  curve, verifies with the anchored verifier, confirms ALL written bins read ~0 on reset).
- **Confirmed branch (anchored, NOT executed)**: ONE anchored curve plan, single-step (`--steps 1`), arms
  Safe Loop before write, resets on every post-arm exit, clears boot flag only after a confirmed reset, no
  persistence/apply/promotion. `confirmed_f2_refusal` reuses the anchor as the candidate.
- **Validation**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service` **240
  passed**; `cargo test -p nidavellir-gpu-nvapi` **33 passed**. F1/build-frontier + simple-F2 tests still
  green. **Read-only dry-run** (`undervolt-probe --target-mhz 1800 --steps 1`, NO `--confirm`): anchor
  **981 mV base 1785 +15 → 1800** (same point as the prior confirmed run), **25** higher-voltage bins
  capped DOWN to 1800 (max flatten **-135 MHz**), 2 already at target, **61** lower bins elastic; `plan
  self-check = AnchoredRaiseVerified`; no-op (no arm/apply/dwell/VF write). `--help` and `--simple` both
  verified. No Safe Loop / forge state mutated (only the 3 source files changed).
- **Hardware NOT yet validated for anchored mode.** First future anchored validation should be:
  `undervolt-probe --target-mhz 1800 --steps 1 --confirm` — ONE candidate, operator present, NOT
  multi-step, no second confirmed run. Detail in `decisions.md` (top).

## Latest backend checkpoint (2026-06-21) — F2 true-undervolt FIRST CONFIRMED HARDWARE VALIDATION — PASS
- **One supervised confirmed run** (operator present, ONE confirmed command, no second) validating the `78ecfc7`
  F2 confirmed branch on real hardware: `undervolt-probe --target-mhz 1800 --steps 1 --confirm`. **First real
  positive-offset VF write.** HEAD = origin/master = `78ecfc7`, tree clean. **Fresh worktree binary built first**
  (`cargo build -p nidavellir-service`) — `target/debug/nidavellir-service.exe` was ABSENT; built mtime newer
  than `78ecfc7`.
- **Preflight PASS**: tree clean; `gpu_applied.json`/`boot_flag.json` absent; `safe_mode=false`;
  `boot_flag_armed=false`; `consecutive_crashes=1`; planned point NOT blacklisted (`blacklisted_points=0`). Help =
  usage only; dry-run = exactly ONE candidate + no-op line (no arm/apply/dwell/VF write).
- **Result: exit 0, outcome `Validated`.** No TDR / black-screen / reboot / DeviceLost / Unstable / silent error.
- **Candidate**: target **1800 MHz**, bin **981 mV**, base **1785 MHz**, offset **+15 MHz** (within +15 step /
  +30 abs caps).
- **Sequence (motor end-to-end)**: Safe Loop armed BEFORE write → `apply_bounded_positive_offset` applied →
  `verify_positive_offset` = **`RaiseVerified`** → dwell **Stable** (avg **1868 MHz**, p5 **1845 MHz**, **199 W**,
  no silent error) → `reset_to_stock` ran + CONFIRMED stock (offset cleared) → boot flag cleared after clean reset.
  Not blacklisted. **No profile persisted/applied/promoted** (Validated reported only, never written to
  `last_validated`).
- **Post-run**: `boot_flag.json`/`gpu_applied.json` absent; `safe_loop.json` **byte-identical** (sha256 unchanged,
  mtime touched only — `safe_mode=false`, `consecutive_crashes=1`, blacklist 4 entries, `last_validated=null`);
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; tree clean; HEAD `78ecfc7`.
- **Meaning**: the F2 true-undervolt HARDWARE path is PROVEN at one bounded positive-offset point — the
  **arm → write → verify → dwell → reset → clear** motor is viable and recoverable on real hardware. It does NOT
  prove an optimal undervolt profile (minimum-viable path only). The dwell clock above 1800 MHz (1868 avg) is
  EXPECTED — this probe does not lock the clock; the GPU still boosts per curve/power; `RaiseVerified` confirms
  the +15 raise on the 981 mV bin.
- **Next (do NOT immediately run another confirmed command)**: one of — (1) bounded/supervised F2 MULTI-STEP probe
  for the same target; (2) explicit `--start-mv` confirmed single-step if not yet supported; (3) result recording /
  Forge Knowledge for validated F2 candidates without promotion. First optimization = search the lower-voltage
  limit around 1800 MHz with the same Safe Loop / verification / reset guarantees. Detail in `decisions.md` (top).

## Latest backend checkpoint (2026-06-20) — F2 CONFIRMED single-step branch IMPLEMENTED, not executed (no hardware)
- **What**: the first real confirmed F2 hardware branch (`undervolt-probe --confirm`). Single-target,
  single-step only. IMPLEMENTED but NOT executed — no `--confirm` run, no VF write, no Safe Loop mutation.
- **Confirmed state machine** (`gpu_undervolt.rs`, trait-isolated + mock-tested): `run_confirmed_f2_step`
  over `F2Ops` = arm boot flag → apply ONE bounded positive offset (`apply_bounded_positive_offset`) →
  verify (offset-presence; idle freq=None) → dwell once → `reset_to_stock` on EVERY exit → clear flag ONLY
  after a CONFIRMED reset. Outcomes: ArmFailed/ApplyFailed/VerifyFailed/Unstable/DeviceLost/ResetFailed/
  Validated.
- **Boot-flag / reset policy** (unit-tested): real `reset_to_stock` re-reads the bin offset and returns Ok
  ONLY if it confirms ~0 (unreadable/non-zero → fail closed → flag RETAINED — F2 never leaves a curve
  applied). Flag cleared only on confirmed reset; RETAINED on DeviceLost + on reset failure. DeviceLost/
  Unstable blacklist the point; only Stable+confirmed-reset → Validated (reported only, never written to
  `last_validated`). No persist/apply/promotion.
- **Preflight** (`confirmed_f2_refusal`, pure): refuses unless --steps 1; not Safe Mode; no armed flag;
  consecutive_crashes < 3 (`SAFE_MODE_CRASH_THRESHOLD`); candidate exists + within bounds; not blacklisted
  (3-axis F2 intent OR 2-axis freq/vf_bin). `run_undervolt_probe_cmd` runs startup recovery on --confirm.
- **Help fixed**: `--help`/`-h` short-circuits before any hardware/plan/Safe-Loop access; prints usage +
  --confirm WARNING (may write VF; operator required).
- **F1 untouched**: `apply_vf_ceiling_monotone` + build-frontier unchanged; `gpu_power_sweep.rs` edits are
  additive/visibility only (`reset_to_stock` pub(crate); new `single_load_dwell`/`SingleDwell` reusing
  `load_and_measure`). No power-limit/TDP/clock-lock. Dry-run output unchanged except footer + help.
- **Files**: `crates/service/src/gpu_undervolt.rs` (confirmed branch + tests), `gpu_power_sweep.rs`
  (adapters), `main.rs` (help + confirm dispatch), `gpu-nvapi/src/lib.rs` UNCHANGED this task.
- **Validation (no hardware)**: `cargo check` clean; `cargo test -p nidavellir-service` **228/0** (+15);
  `cargo test -p nidavellir-gpu-nvapi` **25/0**; dry-run + `--help` exercised read-only.
- **First future run** (operator present, ONE run only): `undervolt-probe --target-mhz 1800 --steps 1 --confirm`.
- **Hardware: STILL BLOCKED / not validated.** Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-20) — F2 true-undervolt foundation IMPLEMENTED (pure, no hardware)
- **What**: the first isolated F2 (true-undervolt) foundation. F2 needs BOUNDED POSITIVE VF offsets (raise a
  lower-voltage bin to hold the target clock) — the OPPOSITE of F1/build-frontier's flatten-down. F1's
  `apply_vf_ceiling_monotone` refuses positive offsets and its verifier treats clock-above-target as failure,
  so F2 is a SEPARATE path with its own bounded, fail-closed symbols. F1/build-frontier is UNCHANGED.
- **Files**:
  - `crates/gpu-nvapi/src/lib.rs`: pure `plan_bounded_positive_offset` + windows `apply_bounded_positive_offset`;
    `PositiveOffsetPlan` / `PositiveOffsetLimits`; consts `POS_OFFSET_MAX_MHZ=+30`, `POS_OFFSET_STEP_MAX_MHZ=+15`.
  - `crates/service/src/gpu_verify.rs`: pure `verify_positive_offset` → `PositiveOffsetVerification`
    (RaiseVerified / RaiseIncomplete / OverRaise / Unverifiable). Flatten-down verifier untouched.
  - `crates/service/src/gpu_undervolt.rs` (NEW): pure `plan_undervolt_probe` search skeleton + pure
    `undervolt_preflight` (Safe Loop read-only refusal) + windows `run_undervolt_probe` (dry-run; `--confirm`
    fails closed).
  - `crates/service/src/main.rs`: `undervolt-probe` subcommand + `parse_undervolt_args` (dry-run default;
    `--confirm` parsed but REFUSED this task). `mod gpu_undervolt;` registered.
- **Fail-closed planner rules**: empty/foreign/non-sane base → Err; non-real bin → Err; below hardware floor →
  Err; offset ≤ 0 (positive-only) → Err; offset > +30 abs cap → Err; per-step delta > +15 → Err; planned clock
  > conservative ceiling → Err. NEVER silently clamps; bounds are CONSTANTS (not CLI-widenable); returns the
  plan BEFORE any write.
- **Scope (v1)**: one focus target, small bounded offset, NO persist/apply/promote, NO multi-target loop, NO
  autonomous crash-seeking. Confirmed mode (future) stops on first crash/TDR/instability/verifier-fail.
- **NOT touched**: `apply_vf_ceiling_monotone`, F1 flatten-down writer/verifier, Safe Loop, boot flag,
  `reset_to_stock`, blacklist, last-known-good, power-limit/TDP/clock-lock. Dry-run reads Safe Loop READ-ONLY.
- **Confirmed path NOT implemented** (explicit TODOs in `gpu_undervolt.rs`): arm boot flag before a positive
  write; clear only after clean dwell+reset; crash leaves recovery/blacklist state; last-known-good fallback.
- **Validation (no hardware)**: see the implementation commit for `cargo check` / `cargo test` results.
- **Hardware: BLOCKED.** No `--confirm`, no VF write, no apply/persist, no Safe Loop mutation. Next: dry-run
  review of `undervolt-probe`, THEN (after review) a first supervised one-step confirmed F2 validation. Detail
  in `decisions.md` (top entry).

## Backend checkpoint (2026-06-20) — F1c bounded-tail confirmed PASS + tail-richness follow-up
- **Confirmed run (2026-06-20) of the bounded tail (`8667bf0`) = PASS.** Safety/mechanics clean (exit 0, no
  TDR/crash/reboot, reset_to_stock ran, no persist/apply, state byte-identical, monotone writer
  positive_offsets=0). Phase A collapsed; Phase B focus 1800, started 1056 mV (below 1062 floor, skipped
  1075/1068/1062), crossed the knee (pcf 1.000@1012 → **0.215@1006 mV**), **continued past the first off-cap
  point to 1000 mV, captured 2 useful points**, stopped `KneeTailComplete`; **synthesis became `differentiated`**.
- **Remaining issue**: both tail points (1006 & 1000 mV) were ~199 W → Godforge/Brokkr's/Deep Calm all
  coincided at ~1811 MHz @ 1006 mV / 199 W. Differentiated (not collapse) but THIN.
- **This follow-up**: enrich the tail — `PHASE_B_MIN_USEFUL_POINTS` 2→**4**, `PHASE_B_POST_KNEE_TAIL_BINS`
  3→**5** (the synthesis collapse threshold `MIN_USEFUL_FRONTIER_POINTS` STAYS 2). Phase B now keeps a bounded
  tail until 4 useful off-cap points OR 5 post-knee bins. Opt-in / default OFF; no new CLI flag;
  `--phase-b-probes` + global `--max-probes` still bound it; failure/verifier/instability/floor/budget keep
  precedence.
- **Unchanged**: Phase A, synthesis, bind-seeking, full safety chain (writer/verifier/Safe Loop/reset_to_stock/
  floor/cluster/persistence/power-limit/clock-lock). File: `crates/service/src/gpu_power_sweep.rs` only.
- **Hardware**: one confirmed validation authorized for this follow-up (same flags) to see if power drops below
  the knee and the three profiles separate. Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-16) — F1c follow-up: Phase B captures a bounded below-knee TAIL (commit 8667bf0) — pure, no hardware
- **Why**: the FIRST confirmed knee-seeking run (2026-06-16, PASS-PARTIAL) found the real knee at **~1025 mV**
  (Phase B started 1056 mV below the 1062 Phase-A floor, descended to 1025 where **pcf dropped 1.000→0.437 in
  one 6 mV bin** — a steep knee). But Phase B stopped at that FIRST off-cap point → only **1** useful point →
  synthesis correctly still reported `POWER-BOUND COLLAPSE`. Stop policy, not budget, was the limiter.
- **What landed**: `descend_phase_b` now captures a BOUNDED below-knee tail. After the knee crossing (first
  `pcf < POWER_BOUND_FRAC` point) it keeps descending until `PHASE_B_MIN_USEFUL_POINTS` (=2) useful off-cap
  points OR `PHASE_B_POST_KNEE_TAIL_BINS` (=3) post-knee bins, then stops cleanly as new
  `BracketStop::KneeTailComplete`. ≥ 2 useful → existing synthesis differentiates; 1 → honest collapse.
- **Safety precedence preserved**: crash / abort / global drain / verifier failure / instability are checked
  BEFORE the tail and stop immediately; floor / `--phase-b-probes` / global `--max-probes` still bound it.
- **Confirmed-run safety (PASS)**: exit 0, no TDR/crash/reboot, `reset_to_stock` ran, no persist/apply
  (`gpu_applied.json`/`boot_flag.json` absent; state files byte-identical), monotone writer `positive_offsets=0`.
- **Unchanged**: Phase A, synthesis, bind-seeking, safety chain (writer/verifier/Safe Loop/reset_to_stock/
  floor/cluster/persistence/power-limit/clock-lock); opt-in / default OFF; no new CLI flag.
- **Files**: `crates/service/src/gpu_power_sweep.rs` only (BracketStop variant + 2 consts + `descend_phase_b`
  tail loop + dry-run plan line + tests).
- **Validation**: `cargo check` clean (0 warnings); `cargo test -p nidavellir-service` **203 / 0** (8 new).
- **Hardware STILL BLOCKED**. Next: NEW dry-run-only review of the bounded-tail plan output, before any
  further confirmed run. Non-goals unchanged. Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-16) — F1c follow-up: Phase B continues BELOW Phase-A floor (commit 9f35ec0) — pure, no hardware
- **What landed**: a budget-efficiency fix for F1c Phase B, acting on the dry-run-only review finding.
  Phase B now CONTINUES below the deepest bin Phase A already explored for the focused target instead of
  re-probing the inert top bins. Files: `crates/service/src/gpu_power_sweep.rs` only (no `main.rs` / no new
  flag). Pure: no hardware, no `--confirm`.
- **Why**: on this card's fine VF curve (~6–7 mV/bin), the `0ef4e68` Phase B re-started from the cap, so
  `--phase-b-probes 12` reached only ~1006 mV — re-covering Phase A's 1075/1068/1062 and stopping ~75 mV
  above the ~930 mV knee. Now each Phase-B probe lands on a new, deeper bin.
- **How**: two pure helpers in the orchestrator — `phase_a_deepest_bin` (focus target's deepest retained
  Phase-A bin) + `phase_b_start_below` (highest real bin strictly below it). Fallbacks: no Phase-A history
  for the target → safe-start cap; Phase A already at the floor → Phase B skipped cleanly. Dry-run plan adds
  a `knee start` line.
- **Unchanged**: Phase A, `descend_phase_b`, synthesis, safety chain (writer/verifier/Safe Loop/
  `reset_to_stock`/floor/cluster/persistence/power-limit/clock-lock); opt-in / default OFF; global
  `--max-probes` master cap.
- **Validation**: `cargo check` clean (0 warnings); `cargo test -p nidavellir-service` **195 passed / 0
  failed** (5 new). No hardware.
- **Hardware STILL BLOCKED**. Next: a NEW dry-run-only review confirming the improved plan (`knee start`
  line, deeper reach). Budget sizing remains the operator's call (~20+ Phase-B probes to cross a ~930 mV
  knee from a ~1062 mV Phase-A floor); this patch makes each probe count, default budget unchanged (12).
  Non-goals unchanged. Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-15) — F1c power-bound knee-seeking two-phase prototype IMPLEMENTED (commit 0ef4e68) — pure, no hardware
- **What landed**: an OPT-IN (default OFF) two-phase knee-seeking mode for `build-frontier`, the
  design-audit direction `NEED DEEPER POWER-BOUND DESCENT`. Phase A = the existing single-pass descent
  (byte-for-byte unchanged when OFF; extracted into `run_target_descents`). Phase B (only after a Phase-A
  power-bound collapse) detects the plateau (median power-bound clock), picks the lowest candidate target
  ≥ plateau, and descends THAT target deeper to cross the knee, then merges + re-synthesizes via the
  existing `synthesize_forge_profiles`. New CLI flags `--power-bound-knee-seeking` + `--phase-b-probes N`
  (default None → 12). Global `--max-probes` stays the master cap.
- **Why**: the validated `0996769` collapse only walked the top ~13 mV (bins `1075/1068/1062`), ~130 mV
  above the card's operating voltage, so the VF ceiling was inert and pcf stayed 1.000 — an honest
  diagnostic for a SHALLOW descent, not proof no frontier exists. Detail in `decisions.md` (top entry).
- **Knee model**: above the knee `pcf >= 0.95` (ceiling inert — keep descending); knee = first pcf drop
  below 0.95; clean deep stop at `pcf <= 0.50`; below-knee tail feeds Brokkr's / Deep Calm; Godforge =
  highest sustained off-cap clock (the knee region), NOT the highest requested clock. No knee ⇒ the honest
  `PowerBoundCollapse` is preserved.
- **Files**: `crates/service/src/gpu_power_sweep.rs` (helpers + `descend_phase_b` +
  `build_frontier_two_phase` + `FrontierLimits` fields + `validate_limits` + dry-run plan lines + wiring +
  tests); `crates/service/src/main.rs` (2 CLI flags + parse test).
- **Safety surfaces UNCHANGED** (diff audited): monotone writer, verifier gates, Safe Loop, `reset_to_stock`
  (runs after every build, both paths), floor/cluster derivation, per-target cap, warm-start default OFF,
  persistence/knowledge writes, power-limit / TDP / clock-lock.
- **Validation**: `cargo check` clean (0 warnings); `cargo test -p nidavellir-service` **190 passed / 0
  failed** (17 new). No dry-run / `--confirm` / hardware.
- **Hardware STILL BLOCKED**. Next: a SEPARATE dry-run-only review of the Phase-B plan output (no
  `--confirm`). A later confirmed run must be a bounded knee-seeking shape (one focused target descended
  deep past ~930 mV), NOT a same-config rerun. Non-goals unchanged: no power-limit/TDP, no clock-lock, no
  persistence/apply, no safety-chain change.

## Backend checkpoint (2026-06-15) — F1b power-bound collapse classification FIRST CONFIRMED HARDWARE VALIDATION (commit 0996769) — PASS
- **One supervised confirmed run** (operator present) validating `0996769`; HEAD = `origin/master` = `4880153`,
  tree clean; fresh worktree-local binary (built after `0996769`, not the stale main-repo target). Confirming
  dry-run gate passed first. Command: `build-frontier --confirm --max-targets 7 --max-probes 21
  --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`. **Exit 0; ~5.7 min.**
- **Safety PASS**: no TDR / crash / driver reset / black-screen / reboot; `reset_to_stock` ran; GPU back at
  stock/idle. After: `gpu_applied.json` / `boot_flag.json` absent; `safe_loop.json` idle/disarmed
  (`safe_mode:false`, no new crash/blacklist entry, mtime touched by startup recovery only); `forge_state.json`
  / `gpu_knowledge.json` / `heartbeat.txt` byte-unchanged; tree clean. Every probe `write_mode=monotone_static`,
  `positive_offsets=0`; no overshoot veto.
- **Mechanics**: 19 probes / 17 dwells; `--max-probes 21` not exhausted; **6 of 7 targets characterized**. 1920
  dropped on a benign verifier `LiveMismatch` at the start bin (verifier worked, no crash, run-variance); 1890
  hit a later `LiveMismatch`, kept its deepest verified bin. Descended to 1062/1068 mV. All dwells PowerLimited,
  `power_capped_frac=1.000`, ~199 W, ~1784–1825 MHz.
- **Reporting honesty PASS**: no `BoundBinding`, no `reason=Clock`. **Clock arm retirement validated** — probes
  with avg clock within 30 MHz of target (which would FALSE-bind under v2) correctly did NOT bind, descended to
  `PerTargetCap`. **`LeftPowerRegime` validated negatively** — evaluated each eligible probe, `bound=false
  reason=None`, no target stopped by it (none had pcf ≤ 0.50). **`PowerBound`/`PowerBoundCollapse` validated
  positively** — 6 points `[power-bound]`; reported `6 power-bound / 0 useful`; explicit *"power-bound collapse
  — cannot build a differentiated VF frontier under this workload/regime"*; frontier classes = `POWER-BOUND
  COLLAPSE (best-effort, NOT a differentiated VF frontier)`. Godforge/Brokkr's/Deep Calm collapsed to one
  best-effort point (1815 MHz / 199 W, R=0.00), confidence 0.21, all flagged not-differentiated — no fake
  frontier.
- **Verdict PASS** (safety + reporting honesty). Physical frontier still not useful here: the card is pinned at
  the ~199 W cap, now reported honestly. **Caveats**: `LeftPowerRegime` validated negatively only (a positive
  stop needs pcf ≤ 0.50, which this regime never produces); 1920 LiveMismatch is benign run-variance.
- **Direction**: accept the patch; **keep hardware BLOCKED for this same config**; do NOT repeat the run, do NOT
  bump the per-target cap, do NOT tune power-limit/TDP/clock-lock yet. Next is a design decision: a workload that
  doesn't saturate the ~199 W cap, candidate targets below the power-bound plateau, or a design pass for
  presenting "cannot differentiate under this workload/regime." Detail in `decisions.md` (top entry).

## Backend checkpoint (2026-06-15) — F1b power-bound collapse classification IMPLEMENTED (commit 0996769) — pure, no hardware
- **Commit `0996769 fix(service): classify power-bound frontier collapse`** (pushed to `origin/master` with
  the docs entry). Scope: `crates/service/src/gpu_power_sweep.rs` ONLY. Implements the SIMPLIFY patch from the
  audit below. No hardware, no `--confirm`, no dry-run.
- **Retired bind-seeking's Clock arm** — `classify_binding` is regime-only: a target binds (stops early) ONLY
  when it LEFT the power-limited regime (`power_capped_frac <= 0.50`). Removed `BIND_OVERSHOOT_MHZ`,
  `BindThresholds.overshoot_mhz`, `BindReason::Clock`; start-bin eligibility guard kept. Renamed
  `BracketStop::BoundBinding → LeftPowerRegime`.
- **Power-bound classification** (`POWER_BOUND_FRAC = 0.95`): pure `is_power_bound_frac` / `is_power_bound_point`
  / `useful_frontier_points` / `frontier_power_bound_collapse`. A pcf-saturated stable dwell = VALID raw
  bracket, NOT useful clock-frontier diversity. Invalid/missing pcf → not power-bound (fail open for
  classification), still fail-CLOSED for regime binding.
- **Collapse-aware synthesis**: `synthesize_forge_profiles` excludes power-bound points; < 2 useful → FLAGGED
  best-effort + diagnostic *"power-bound collapse — cannot build a differentiated VF frontier under this
  workload/regime"* (new `ForgeProfiles.power_bound_excluded` / `power_bound_collapse`). Catches the jittery
  ~1798–1819 MHz @ pcf 1.0 plateau the exact-distinct-clock check missed. NO power-bound points → legacy path
  byte-for-byte unchanged. RESULT output now prints per-point `pcf` + a `frontier classes` summary.
- **Safety surfaces UNCHANGED** (diff audited — no protected symbol added/removed): monotone writer, verifier
  gates, Safe Loop, `reset_to_stock`, floor/cluster derivation, per-target cap, warm-start default OFF,
  persistence/knowledge writes, power-limit/clock-lock.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  **173 passed / 0 failed** (was 169: +5 power-bound tests, +2 regime tests, −3 retired clock tests).
- **Hardware STILL BLOCKED**: pure code/test patch. Next confirmed run only AFTER reviewing the new
  classification/reporting in a fresh dry-run; the same-config rerun remains not recommended. Full detail in
  `decisions.md` (top entry).

## Backend checkpoint (2026-06-15) — build-frontier / F1b algorithm audit — verdict SIMPLIFY (read-only, pre-implementation)
- **Read-only audit only.** Inspected `crates/service/src/gpu_power_sweep.rs` + continuity docs. **No code edit,
  no tests, no `build-frontier`, no `--confirm`, no hardware, no VF write, no stress, no power sweep** were run.
  This entry records the audit conclusion BEFORE implementation so the next patch has a clear north star.
- **Verdict: SIMPLIFY CURRENT DIRECTION.** Not a redesign, not a full rollback. Do **not** run more hardware
  before the next pure/pure-ish patch; do **not** keep adding bind-seeking complexity. The discovery → descent
  → synthesis skeleton is still valid; the drift is concentrated in **bind-seeking / `BoundBinding`** semantics.
- **North star (unchanged)**: 1) find max core clock / top sustainable target; 2) start at max safe voltage /
  safe VF ceiling; 3) dwell holding the target; 4) descend real VF voltage bins while sustainable; 5) stop each
  target on an explicit reason — unstable, verifier failure, crash/abort/budget drain, voltage floor, per-target
  cap, **or power-bound regime/collapse**; 6) next lower target; 7) build a real stable frontier; 8) synthesize
  Godforge / Brokkr's / Deep Calm **only from meaningful (non-power-bound) data**.
- **Load-bearing — KEEP**: hardware-derived floor; cluster selection / sane-core VF filtering; real-bin descent;
  per-target probe cap; the typed hard/soft stops; confidence gate + best-effort fallback; monotone static-base
  writer; verifier gates; Safe Loop; `reset_to_stock`; no profile persistence during build-frontier.
- **Bind-seeking conclusion**: `BoundBinding` is the wrong **combined** abstraction — it mixes a **bad Clock
  arm** (false-binds under power cap: the cap, not the descent, sets the clock) with a **useful Regime arm**
  (`power_capped_frac <= 0.5`, card left power-limited behavior). The v2 start-bin guard was useful + validated
  but did NOT solve physical frontier collapse. The confirmed v2 run (`bf02971`) stayed power-limited
  throughout: `power_capped_frac=1.000`, ~199 W, ~1798–1819 MHz, confidence 0.21, profiles collapsed. ⇒ the
  remaining issue is **regime / power-bound collapse, not scheduler depth or per-target probe count**.
- **Decision**: stop treating a `Clock` bind as sufficient evidence of useful VF-bound behavior when
  `power_capped_frac` is saturated; **retire/neutralize the Clock arm**; keep the regime signal reclassified as
  **`LeftPowerRegime`**; add a first-class **`PowerBound` / `PowerLimitedPlateau` / `PowerBoundCollapse`**
  classification; strengthen synthesis so power-bound samples are valid raw brackets but **not** useful
  clock-frontier diversity. NOTE: the current collapse detector keys on exact-distinct clocks, so a jittery
  ~1798–1819 MHz plateau reads as ~6 "distinct" clocks and the warning never fires — even with bind-seeking OFF
  synthesis silently emits a falsely-differentiated frontier. Re-key it on pcf saturation.
- **Power-limited sample treatment**: valid bracket = yes; useful clock-frontier point = no (when pcf
  saturated); synthesis = raw input yes but excluded from differentiated selection; collapse diagnostic = yes
  (primary). Mark, don't discard (still a valid warm-start seed).
- **Next safest patch (pure / mostly pure, `gpu_power_sweep.rs`)**: add/rename stop classifications
  (`VoltageFloor`, `DepthCap`, `LeftPowerRegime`, `Unstable`, `VerifyFailed`, `Crashed`, `Aborted`,
  `BudgetDrained`, `PowerBound`/`PowerBoundCollapse`); strengthen `synthesize_forge_profiles` (detect the
  pcf-saturated plateau; don't treat jittery ~1800 MHz clocks as a real differentiated frontier; emit
  *"power-bound collapse — cannot build a differentiated VF frontier under this workload/regime"*); add tests
  over synthetic samples; **do not touch the hardware-writing path.** Optionally add read-only power-headroom
  telemetry.
- **Explicit non-goals**: no confirmed hardware run; no power-limit/TDP changes; no clock-lock changes; no
  target-generation redesign yet; no warm-start default change; no per-target cap change; no Safe Loop / reset /
  writer / verifier changes; no profile persistence / knowledge write change; no version bump.
- **Hardware: BLOCKED** until the power-bound classification + collapse report land and a fresh dry-run shows
  the new diagnostics. Re-running the proven-uninformative config is not justified. Full audit rationale in
  `decisions.md` (top entry); index line in `memory.md`.

## Backend checkpoint (2026-06-15) — bind-seeking F1b v2 strictness FIRST CONFIRMED HARDWARE VALIDATION — mechanism PASS / frontier PARTIAL (commit bf02971)
- **Supervised confirmed run** (operator present; this session). Validates `bf02971 fix(service): tighten
  bind-seeking stop criteria`; docs already at `3b8774c docs: record bind-seeking v2 strictness`
  (HEAD = origin/master = `3b8774c`, working tree clean).
- **Fresh worktree binary built first.** The worktree-local `target/debug/nidavellir-service.exe` was ABSENT
  and the only existing binary was STALE (main-repo `target/debug`, built 2026-06-07 — predates the entire
  bind-seeking feature). Built `cargo build -p nidavellir-service` → worktree binary created AFTER the build
  (mtime after the build marker, size differs from stale); the **stale main-repo binary was NOT used**;
  working tree stayed clean (target/ is gitignored).
- **Dry-run gate passed** (no `--confirm`): bind-seeking ENABLED; v2 strict "start bin is NOT bind-eligible"
  note; thresholds `avg_clock_overshoot <= 30 MHz` + `power_capped_frac <= 0.50`; coverage-bounded scheduler;
  `max_probes=Some(21)`; `max_probes_per_target=Some(3)`; targets `[1935,1905,1875,1845,1815,1785,1755]`;
  first-pass bins `[1075,1068,1062]`; warm-start OFF; no applied-profile warning; no Safe Loop conflict
  warning; dry-run no-op line (no Safe Loop arm / apply / dwell / VF write).
- **Confirmed command**: `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`.
- **Safety: PASS.** Exit 0; **no TDR / driver reset / black-screen / reboot / crash**. Startup recovery clean
  ("clean boot, nothing to restore"); `reset_to_stock` ran ("GPU restored to stock; no profile applied or
  persisted"). After: `boot_flag.json`/`gpu_applied.json` **absent**; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` **unchanged** (no persistence, no knowledge write); `safe_loop.json` stayed **idle**
  (`safe_mode:false`), size unchanged — only mtime bumped by startup recovery, no new blacklist/crash entry.
  GPU back at stock idle.
- **Probe result**: **15 probes/dwells** total; **all 7 targets physically characterized**. 6 stopped via
  **`BoundBinding`** (1935, 1905, 1875, 1845, 1815, 1785, each `probes_used=2`); 1 stopped via
  **`PerTargetCap`** (1755, `probes_used=3`). No target dropped. Global `--max-probes 21` **not exhausted**
  (15/21). No `overshoot_veto`. Every probe `write_mode=monotone_static`, `positive_offsets=0`.
- **v2 mechanism — PASS (start-bin guard validated).** **Every** 1075 mV start bin reported
  `eligible=false / bound=false` — v2 definitively prevents start-bin binding. All 7 targets descended to
  **1068 mV**; **1755** descended further to **1062 mV**. Earliest binding occurred only AFTER a real bin
  descent (the 6 bound targets bound at the 2nd bin, 1068, `reason=Clock`). Bind telemetry present on every
  probe: `eligible / bound / reason / avg_clock_mhz / p5_clock_mhz / power_capped_frac`. Regime arm
  (`power_capped_frac <= 0.5`) never fired (pcf saturated at 1.000) — binding came solely via the avg-clock
  path.
- **Physical frontier — PARTIAL (did NOT de-collapse).** All dwells were **PowerLimited** with
  **`power_capped_frac=1.000` throughout** (199 W flat). Achieved clocks clustered **~1798–1819 MHz**; all
  targets converged to the same power-bound operating point (vf_bin 1068, 1755 at 1062). Synthesis confidence
  stayed **0.21** (R=0.00); Godforge/Brokkr's/Deep Calm collapsed to **~1800 MHz / 199 W**.
- **Interpretation**: v2 fixed the **procedural** start-bin binding bug (the v1 collapse). The **remaining
  collapse is power/regime-related, not scheduler depth and not the per-target cap** — the card is pinned at
  the 199 W limit, so every target above the power-bound clock degenerates to one point. **Do NOT repeat the
  same hardware run; do NOT increase the per-target cap as the immediate next action (1755 went deeper to 1062
  and still produced ~1811 MHz / 199 W); do NOT jump directly to risky power-limit / clock-lock changes.**
- **Next design work (analysis first, no further confirmed run yet)**: add/adjust **regime-aware binding
  semantics**; distinguish a true `Clock` bind from a `PowerLimitedPlateau` / `PowerBoundCollapse`; consider
  **vetoing `Clock` binding when `power_capped_frac` is saturated near 1.0**; add explicit collapse
  diagnostics and power-headroom / power-drop telemetry. **Stop for analysis before any further confirmed
  hardware run.**
- **Scope of this entry**: docs/continuity only (`handoff.md`, `decisions.md`, `memory.md`). The only commands
  run this validation were one debug build, one dry-run, and one confirmed run — **no code edits, no tests, no
  further hardware**.

## Backend checkpoint (2026-06-15) — bind-seeking F1b v2 strictness IMPLEMENTED + PUSHED (commit bf02971) — now hardware-validated (see entry above)
- **Commit `bf02971 fix(service): tighten bind-seeking stop criteria`** — pushed to `origin/master`
  (HEAD = origin/master = `bf02971`). Scope: `crates/service/src/gpu_power_sweep.rs` ONLY (no other file).
- **Why**: the v1 supervised hardware run (`--bind-seeking`, this session) was **safety/mechanics PASS but
  semantic PARTIAL** — v1 allowed `BoundBinding` on the **first/start bin at 1075 mV**, so every viable target
  stopped immediately with no descent; the frontier stayed degenerate/single-bin (all ~1075 mV / ~199 W) and
  Forge synthesis confidence stayed low (~0.21).
- **v2 changes** (`classify_binding` now returns `BindDecision` and takes an `eligible` flag):
  - **Start bin is NOT bind-eligible** — a target must descend ≥1 real VF bin before `BoundBinding` can fire;
    earliest bind = the **2nd probed real VF bin** (`bind_eligible(probes_before, cur_bin, start_bin)`).
  - **Clock binding uses the AVERAGE/achieved clock** (`avg_clock_mhz - target <= 30`), not p5/sustained;
    p5 remains telemetry/reporting only; zero/absent avg fails closed (no clock binding).
  - **Regime arm unchanged** (`power_capped_frac <= 0.5`) but **invalid/missing cap_frac fails closed**
    (NaN / <0 / >1 → no regime binding, via `valid_cap_frac`).
  - **Bind telemetry** added (live run, per verified+stable probe): `eligible`, `bound`, `reason`
    (`BindReason::None/Clock/Regime`), `avg_clock_mhz`, `p5_clock_mhz`, `power_capped_frac`.
  - **Dry-run** prints the new `binding eligibility: start bin is NOT bind-eligible …` caveat + v2 threshold
    wording (`avg_clock_overshoot <= 30 MHz`).
- **Stop precedence PRESERVED**: crash → abort → budget drain → verifier failure → dwell instability →
  **binding** → per-target cap → floor (only the binding arm is now gated by eligibility).
- **Safety boundaries UNCHANGED**: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  persistence/profile apply, hardware-floor derivation; **warm-start default remains OFF**.
- **Validation before commit (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p
  nidavellir-service` **169 passed / 0 failed** (new: start-bin-not-eligible, avg-not-p5, invalid-cap-frac
  fail-closed, skips-start→binds-at-second). **Dry-run only** (no `--confirm`) passed:
  `build-frontier --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`.
  No hardware boundary crossed.
- **Hardware validation: DONE 2026-06-15** (see the FIRST CONFIRMED HARDWARE VALIDATION entry above) via
  `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075
  --bind-seeking` — mechanism PASS (start-bin guard), frontier PARTIAL (still power-limited / collapsed).

## Backend checkpoint (2026-06-14) — bind-seeking F1b v1 IMPLEMENTED + PUSHED, hardware-validated PARTIAL → superseded by v2 (commit 08f745e)
- **Commit `08f745e feat(service): add opt-in bind-seeking to build-frontier`** — pushed to `origin/master`
  (HEAD = origin/master = `08f745e`). Scope: `crates/service/src/gpu_power_sweep.rs` +
  `crates/service/src/main.rs` ONLY. This builds the bind-seeking direction set after the `5248758` run.
- **Feature**: new opt-in CLI flag **`--bind-seeking`** + `FrontierLimits.bind_seeking` (default **OFF** —
  absence reproduces current behavior byte-for-byte). Per target, the descent stops at the first verified +
  dwell-stable **binding** point instead of walking a fixed number of bins, so each target can contribute a
  distinguishable point (vs the 1832–1867 MHz / 194–199 W collapse in the `5248758` run).
- **Binding signal v1 (Clock + regime)** — pure `classify_binding`: a verified + stable probe binds iff EITHER
  `sustained - target <= BIND_OVERSHOOT_MHZ (30)` (sustained = p5 if present, else avg) OR
  `power_capped_frac <= BIND_CAP_FRAC (0.5)` (card no longer power-pinned).
- **Power-drop deliberately NOT a v1 stop-condition**: no top-power reference tracking. May become
  telemetry/log later — never a binding/stop rule in v1.
- **Scheduler**: adds `BracketStop::BoundBinding` — a CLEAN stop (`is_hard_failed()==false`), carry-forward
  eligible when it recorded a `lowest_verified_mv`. Binding is checked ONLY on a verified+stable sample, AFTER
  the failure arms. **Precedence preserved**: crash/hard-failure → aborted → global budget drained →
  verifier-failure/unverified → dwell-unstable/silent-error → **then binding** → then per-target cap / floor.
- **Interactions**: `--max-probes` stays the hard global cap; `--max-probes-per-target` stays the per-target
  attempt/depth cap; bind-seeking may stop EARLIER than the per-target cap; **warm-start stays default OFF**.
- **Safety boundaries UNCHANGED**: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  hardware-derived floor, persistence/profile apply — none modified. No power-limit / clock-lock changes.
- **Validation before commit (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p
  nidavellir-service` **165 passed / 0 failed** (incl. classifier 30/0.50 boundaries, the full failure
  precedence, BoundBinding carry-forward, and dry-run reporting). **Dry-run only** (no `--confirm`):
  `build-frontier --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`
  → exit 0; printed `bind-seeking: ENABLED`, thresholds (`clock_overshoot <= 30 MHz OR power_capped_frac
  <= 0.50`), the live-metrics caveat, warm-start OFF; **no Safe Loop arm / apply / dwell / VF write**.
- **Hardware validation: NOT yet run for `08f745e`.** Next step (separate, operator-present, NOT this task): a
  clean confirming dry-run, then the supervised confirmed run
  `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075 --bind-seeking`.
- **Scope of this entry**: docs/continuity only. No code/test/hardware commands run in the docs pass.

## Latest backend checkpoint (2026-06-13) — F1b `--max-probes-per-target` FIRST CONFIRMED HARDWARE VALIDATION — coverage PASS / profile PARTIAL (commit 5248758)
- **Supervised confirmed run** (operator present, after a clean confirming dry-run that showed no plan drift;
  HEAD/origin/master at `5248758`; required commits `47f39be`/`f90981d`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 14 --max-probes-per-target 2 --safe-start-cap 1075`
  — **warm-start OFF**. **Exit 0; ~4 min; no TDR, no driver reset, no black-screen, no reboot, no crash markers.**
- **Safety state**: startup recovery clean; Safe Loop armed/cleared **per probe**, ended **idle/disarmed**;
  `reset_to_stock` ran ("GPU restored to stock; no profile applied or persisted"). After:
  `boot_flag.json`/`gpu_applied.json` **absent**; `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt`
  unchanged (no forge-state persistence, no knowledge write); `safe_loop.json` content/size **unchanged**
  (idle, `safe_mode` false) — mtime touched only, **no new blacklist/crash entry**. GPU back at stock idle.
- **Coverage — PASS (the fix works)**: **13 hardware dwells spread across all 7 targets** (not depth-first on
  one). 6 targets stopped cleanly via **`PerTargetCap`** (`probes_used=2` each, bins **1075 + 1068 mV**);
  global `--max-probes 14` was **not exhausted** (13 used). The per-target cap successfully prevented one
  target from consuming the whole budget — the exact fix vs the prior 34-on-1935 depth-first run.
- **Target 1905 dropped** after its 1st probe: ceiling 1075 mV → **`LiveMismatch`**, **`overshoot_veto=true`**,
  `eff_cov=0.963`, 1 unexplained zero — a conservative verifier rejection (neighbors 1935 `NoDownCapNeeded`
  and 1875 `VerifiedCurve` passed), not a hardware fault. **6/7 produced frontier points.**
- **Writer/verifier**: every probe `write_mode=monotone_static`, `positive_offsets=0`; verdicts
  `NoDownCapNeededCeiling` (1935) + `VerifiedCurve` (1875–1755). No VF persistence; no Safe Loop / reset /
  verifier / writer regression. Voltage band shallow only — **1075 and 1068 mV**; did **not** touch
  875/868/862/856/850 mV (the cap-2 stop holds at the top two bins).
- **Profile goal — PARTIAL**: lower targets did **not** produce distinct clock/power. Achieved clocks
  clustered **1832–1867 MHz**, power **194–199 W**; the live plateau stayed ~1890 MHz with `overshoot`
  growing 1875:+30 → 1755:+135 → the near-stock flatten does not govern the achieved clock.
  Godforge/Brokkr's/Deep Calm collapsed to one point (**1860 MHz / 194 W**, target 1755); FORGE confidence
  stayed low (best 0.21, single-trial → best-effort synthesis).
- **Key conclusion**: **shallow near-stock coverage at 1075/1068 mV is non-binding on this hard power-capped
  RTX 3060 Ti** — the ceiling does not materially govern the achieved clock at that high-voltage band.
  `--max-probes-per-target` **solved budget distribution, not binding/differentiation**.
- **Direction (next design = bind-seeking F1b)**: do NOT repeat the same flags; do NOT use per-target cap 3
  next; do NOT enable warm-start next; do NOT jump straight to power-limit/clock-lock changes. Instead, per
  target: **keep descending while the point is stable but non-binding, and stop when it actually BINDS** (the
  ceiling materially governs the clock), fails the verifier/dwell, or hits the global/per-target cap — the
  goal is the **first useful (binding) point per target**, not the deepest voltage. **No further hardware
  commands were run.**
- **Scope**: docs/continuity only (`handoff.md`, `decisions.md`, `memory.md`). No code/test/IPC/hardware
  change in this pass.

## Backend checkpoint (2026-06-13) — Hardware-derived / bin-based floor FIRST CONFIRMED HARDWARE VALIDATION — PASS (commit f90981d)
- **Supervised confirmed run** (operator present, after a clean bounded dry-run on a fresh debug build;
  HEAD/origin/master at `c99dbf1`+`f90981d`; required commits `23b70c4`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 34 --safe-start-cap 1075 --warm-start-brackets`.
  `--max-probes 34` was chosen so the descent reaches **868 mV** (one real bin below the old 875 mV floor)
  but stops BEFORE **862 mV** (a historical reboot-zone / blacklisted bin). **Exit 0; no TDR, no driver
  reset, no black-screen, no reboot, no crash markers.**
- **Safety state**: startup recovery clean ("clean boot, nothing to restore"); Safe Loop armed at startup
  and cleared back to **idle**; `reset_to_stock` ran ("GPU restored to stock; no profile applied or
  persisted"). After: `boot_flag.json`/`gpu_applied.json` **absent** (as before); `forge_state.json`/
  `gpu_knowledge.json`/`heartbeat.txt` unchanged; `safe_loop.json` **byte-identical** (idle, `safe_mode`
  false, size unchanged) — mtime touched at run start only, **no new blacklist/crash entry**
  (`consecutive_crashes` still 1, `crash_log` still `["unrelated"]`). GPU back at stock idle.
- **Probe budget — 34 hardware dwells, ALL on target 1935** (ceilings 1075→868 mV; benign_zeros 27→60).
  The per-target `bracket_carry` line logs `probes_used=35` with `stop_reason=BudgetExhausted` — the 35th
  increment is the scheduler ATTEMPTING the next step (→862 mV) and finding the budget spent; **no
  `ceiling_mv=862` line exists; 862 mV was never set or dwelled.** Targets 1905/1875/1845/1815/1785/1755
  were NOT physically characterized this run (budget exhausted on the hardest target; each logged a
  `probes_used=1` bookkeeping entry, no dwell, "no stable point in safe range — dropped").
- **Voltage coverage**: reached **875 mV** (probe 33) and **868 mV** (probe 34), both
  `NoDownCapNeededCeiling`, `eff_cov=1.000`, `overshoot=0`; **did NOT reach 862 mV**. Safety goal met —
  validated **one real bin below the old 875 mV floor** while **avoiding the historical 862/855 mV
  reboot-zone bins** on this first bounded run. Warm-start observed: 1935 started from cap 1075
  (`warm_started=false`); 1905 carried 1935's bracket (`warm_started=true`, `start_mv=893` = 868 + 25 mV).
- **Writer/verifier**: every probe `write_mode=monotone_static`, `positive_offsets=0`, `down_caps=0`, no
  `overshoot_veto`, all verified `NoDownCapNeededCeiling`, `eff_cov=1.000`.
- **IMPORTANT interpretation**: this validated **safe WRITING/descent of the static VF ceiling down to
  868 mV** — it did NOT prove core stability when actually RUN at 868 mV. The GPU stayed **power-limited
  (~198 W)** the whole descent, so the ceiling was **non-binding** (the core's power-governed operating
  point was already at/below each ceiling → no down-cap needed). Frontier point: 1935 target →
  **1839 MHz @ 868 mV vf_bin, 198 W** (p5 1800). **PASS for the first bin-based floor validation;
  partial/insufficient for profile synthesis** — FORGE confidence stayed low (best 0.21), single
  sustainable clock (1800 MHz), so Godforge/Brokkr's/Deep Calm collapsed identical.
- **Direction**: do NOT jump straight to `--max-probes 40`. `--max-probes 35` would deliberately touch
  **862 mV** if the goal is pure reboot-zone boundary mapping (operator present; NB the 862 blacklist
  entry is keyed `freq=1755`, so a 1935-target ceiling at 862 would not match it — Safe Loop is the
  backstop, not prevention), but that is NOT the best path for useful profiles. **Primary next step: pivot
  to F1b / multi-clock characterization, and/or a regime that makes the ceiling actually BIND (e.g. raise
  the power limit) before descending deeper** — since the descent was power-limited, deeper ceilings add
  reboot-zone exposure for ~zero characterization gain. **No further hardware commands were run.**

## Backend checkpoint (2026-06-13) — Hardware-derived / bin-based build-frontier floor SHIPPED (commit f90981d)
- **Change** (`f90981d feat(service): derive build-frontier floor from real VF bins`, on `origin/master`):
  the hardcoded active **875 mV** descent floor is GONE. `build-frontier` now derives the floor from the
  GPU's real VF / core-cluster voltage bins — `hw_floor_mv = seed.cluster_v_min_mv` (lowest real
  graphics-core bin). No replacement fixed floor (no 825/800); `FRONTIER_LOWEST_SAFE_MV` deleted.
- **Bin-based descent**: `FrontierDescent` carries `bins_desc` (real descending bins) built by
  `derive_descent` from `CoreSeed.cluster_bins_mv`; `descend_target` walks **real bins only** — it does
  NOT invent 25 mV requested voltages off the curve. Warm-start snaps its margin to the **conservative
  real bin ≥ the requested margin target** and **never starts below the previous `lowest_verified_mv`**
  (B1). `--max-probes` remains the exposure cap; `--warm-start-brackets` stays default OFF; no new flag.
  Empty/underivable bin domain → **fail closed before any hardware write**. Dry-run prints the
  hardware-derived floor, the exact descent bin sequence, the real bin count, and worst-case dwells.
- **Scope**: only `crates/service/src/gpu_power_sweep.rs`. **Unchanged**: monotone static-base writer,
  verifier gates, Safe Loop, `reset_to_stock`, persistence, profile apply. `cargo check` clean;
  `cargo test -p nidavellir-service` **142 passed**.
- **Historical note**: the older `1755 @ 875` validations below remain valid for that point but are NO
  LONGER the active floor; future runs may descend **below 875** where real bins exist + budget allows.
- **NOT hardware-validated yet.** The descent may now reach **below the historical ~855 mV reboot zone**.
  **Suggested next operational step (DRY-RUN ONLY, no `--confirm`):**
  `build-frontier --max-targets 7 --max-probes 70 --safe-start-cap 1075 --warm-start-brackets` → review
  the hardware-derived floor, exact bin sequence, worst-case dwell count, and whether `--max-probes` is
  enough. ONLY THEN consider a separate supervised `--confirm` run (operator present + able to reboot).

## Backend checkpoint (2026-06-13) — Warm-start voltage-bracket carry-forward SHIPPED + HARDWARE-VALIDATED (commits 23b70c4, 6f2f061)
- **Feature** (`23b70c4 feat(service): add warm-start bracket carry-forward`, on `origin/master`):
  a **generic** scheduler primitive (NOT Godforge-specific) for ordered hardest→easiest core-clock
  voltage descents. An easier target reuses the previous harder target's verified + dwell-stable
  bracket as its descent start (`lowest_verified_mv + 1 step`), skipping dominated high-voltage
  probes. Opt-in CLI flag **`--warm-start-brackets`, default OFF** — no runtime behavior changes
  unless the flag is passed. Preserves the monotone static-base VF writer, verifier gates,
  `overshoot_veto`, Safe Loop, `reset_to_stock`, persistence/profile apply, and the 875 mV floor.
  Safety constraints B1/B2/B3 (see `decisions.md`).
- **Hardware validation (2026-06-13) — PASS.** Supervised
  `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075 --warm-start-brackets`
  on a fresh debug build, after a clean bounded dry-run (warm-start shown ENABLED;
  `gpu_applied.json`/`boot_flag.json` absent; state mtimes unchanged). **Exit 0; no TDR/reboot**;
  startup recovery clean; Safe Loop armed/cleared per probe; `reset_to_stock` ran ("GPU restored to
  stock"). After: `boot_flag.json`/`gpu_applied.json` absent; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged; `safe_loop.json` mtime touched at run start (idle/disarmed, size
  unchanged); GPU back at stock idle.
- **Scheduler behavior (33 probes, all 7 targets produced points):**
  - 1935 (first) started at cap 1075, descended to floor 875 (boost-top NoDownCapNeeded everywhere),
    `lowest_verified=875`.
  - **B2 exercised — 1905**: inherited an optimistic 900 mV warm start from 1935's boost-top bracket,
    failed verify at 900 (`LiveMismatch`, `overshoot_veto=true`) → **fell back ONCE to cap 1075**,
    re-descended, target preserved (point at 1075). Fallback did not loop and did not fire on a drain.
  - 1875/1845 collapsed to cap (`lv+step ≥ cap`); 1815/1785/1755 warm-started below cap and skipped
    1 / 2 / 3 dominated high-V probes respectively.
  - **Probes: 33** vs **32** baseline (≈ flat raw) but **−5 vs the equivalent from-cap descent (38)**
    for an identical frontier — net of one B2-fallback probe. Benefit is modest on this RTX 3060 Ti
    because mid targets stop early on verify-axis residual overshoot regardless of start voltage.
- **Frontier preserved.** Critical low-V region re-validated exactly: **`1755 @ 900`**
  `NoDownCapNeededCeiling`, plateau **1665..1755**, overshoot 0, dwell stable; **`1755 @ 875`**
  `NoDownCapNeededCeiling`, plateau **1620..1755**, overshoot 0, ≈**1755 MHz @ 875 mV, ≈176 W**.
  Every probe `write_mode=monotone_static`, `positive_offsets=0`. Residual single-bin 15 MHz
  overshoot on non-1755 targets persists (safe verify-axis early stop, as before). FORGE synthesis
  low confidence (best 0.21) — unrelated Wilson metric.
- **B1/B2/B3 held in live logs** (B2 actually exercised). Feature is hardware-validated as **safe
  behind the opt-in flag**.
- **Observability follow-up** (`6f2f061 feat(service): surface build-frontier scheduler logs`):
  log-only. `run_build_frontier` previously emitted only `result.profiles.log`; now emits the
  scheduler/frontier `result.log` (bracket carry / warm-start / fallback / probes_used) FIRST, then
  the synthesis log, deduping shared lines (pure `ordered_frontier_logs` helper + 2 unit tests). No
  tuning behavior changed. Closes the validation finding that bracket telemetry existed but was
  invisible in CLI output.
- **Keep `--warm-start-brackets` default OFF** until more runs justify flipping it. **Next** (later,
  optional): 1–2 more warm-start runs; a benign-zero-only (NoDownCapNeeded) bracket-seeding refinement
  so a boost-top bracket doesn't mis-seed the next sub-boost target (the 1905 B2 case); broader
  frontier/profile confidence work. **Do NOT mix with profile persistence yet.**

## Backend checkpoint (2026-06-12) — Phase 2B.2-c: monotone static-base VF writer HARDWARE-VALIDATED (commit 8503182)
- **Milestone — supervised hardware revalidation of the monotone static-base VF ceiling writer.**
  After a clean bounded dry-run on a fresh `origin/master` debug build at `8503182`
  (`gpu_applied.json`/`boot_flag.json` absent; state mtimes unchanged), the user approved one
  confirmed run: `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075`.
- **Safety: exit 0, no TDR, no reboot.** Startup recovery clean ("clean boot, nothing to restore");
  Safe Loop armed for the VF writes and cleared; `reset_to_stock` ran at the end ("GPU restored to
  stock; no profile applied or persisted"). After the run: `boot_flag.json` absent,
  `gpu_applied.json` absent; `forge_state.json` / `gpu_knowledge.json` / `heartbeat.txt` unchanged
  (build-frontier never persists); `safe_loop.json` mtime touched at run start, size unchanged
  (startup-recovery bookkeeping only). GPU back at stock idle (nvidia-smi ~66 W / 7% util / 200 W
  limit). The audited safety contract held end-to-end on real hardware.
- **Functional: monotone writer confirmed working.** Every one of the **32 probes** logged
  `write_mode=monotone_static` with **`positive_offsets=0`** (static-base-anchored monotone-down
  offsets only) over `static_base_points=132`.
- **Primary fixed case — `1755 @ 900 mV` no longer overshoots.**
  - OLD: raw_cov 0.891, eff_cov 1.000, **`overshoot_veto=true`**, plateau **1755..1845**, result
    **`LiveMismatch`** (blocked).
  - NEW: `positive_offsets=0`, eff_cov 1.000, **overshoot=0**, plateau **1665..1755** (max 1755),
    veto not triggered, result **`NoDownCapNeededCeiling`** (pass). Plateau max dropped 1845 → 1755
    exactly; overshoot collapsed to 0.
- **Run continued to `1755 @ 875 mV` and it verified**: `NoDownCapNeededCeiling`, overshoot=0,
  plateau **1620..1755**, dwelled (~19 s); achieved ≈ **1755 MHz @ 875 mV, ≈179 W**. The `1755`
  ceiling descent shows overshoot decaying cleanly to 0 from 950 mV down (950/925/900/875 all
  overshoot=0).
- **Minor residual (not a blocker for the writer fix)**: a few **non-1755** probes at low ceilings
  still show a single-bin **15 MHz** overshoot with `overshoot_veto=true` (e.g. 1905@1050, 1875@950,
  1845@975, 1815@950, 1785@950). All `1755` probes are overshoot=0.
- **Unrelated note**: FORGE synthesis reported low confidence (best 0.21 < 0.85) → best-effort
  profiles. This is the single-trial Wilson confidence metric, not a writer/overshoot issue.
- **Next technical phase**: design **warm-started voltage-bracket reuse** for F1b / Godforge (carry
  a verified bracket forward across targets to cut probes). **Do NOT mix this with persistence /
  profile apply yet** — keep build-frontier non-persisting until the bracket-reuse design lands.

## Backend checkpoint (2026-06-11) — Phase 2B.2-c: FIRST confirmed run (SAFE, 0 points) + c.1 stock-equivalent verifier fix (IMPLEMENTED, not committed)
- **Milestone — first supervised hardware run executed.** After a Fable 5 blocker audit (GO) and a
  clean bounded dry-run (fresh worktree debug build of 6881cd7; `gpu_applied.json` absent; mtimes
  unchanged), the user approved and we ran
  `build-frontier --confirm --max-targets 1 --max-probes 6 --safe-start-cap 1075`.
  **Safety: exit 0, no TDR, no reboot (~10 s)**; startup recovery clean; Safe Loop armed before the
  VF write and cleared after; `reset_to_stock` fired on the verify reject and again at run end; no
  profile applied/persisted (`gpu_applied.json` absent; `forge_state`/`gpu_knowledge` mtimes
  unchanged; `safe_loop.json` re-saved by startup recovery = bookkeeping only); GPU back at stock
  idle (1% util / 44 °C / 64 W). The full audited safety contract held on real hardware.
- **Functional: 0 frontier points.** The only probe — target=1935 (the stock cluster boost top),
  ceiling 1075 mV — was rejected by the verify gate: `verify=LiveMismatch offsets=20/27
  plateau=1935..1935 overshoot=0`, so the descent stopped before any dwell. Root cause: flatten-to-
  boost-top needs ZERO offset on the 7 top bins already at 1935 in stock → the ≥90% offset-presence
  gate under-counts. The frequency evidence (plateau exactly at target, no overshoot) showed the
  ceiling WAS in effect.
- **c.1 fix (this checkpoint, code+tests only, NO hardware run yet)**: narrow stock-equivalent
  acceptance — pure `is_stock_equivalent_ceiling` in `gpu_verify.rs`, consulted ONLY on a
  `LiveMismatch`, accepting only when: target within tol of the caller-supplied stock boost top;
  ALL offsets readable; NO bin above target (overshoot rejected even within tol); all bins within
  tol below target; every zero-offset bin EXACTLY at target (offset 0 ⇒ GetStatus shows stock base;
  correct flatten writes `target−base`, so only `base==target` explains a missing offset). Carried
  as service-internal `LiveCeilingEval.stock_equivalent` + `stock_equivalent_bins`;
  **`CurveVerification` IPC untouched**. `eval_ceiling_evidence`/`classify_live_ceiling` gain
  `stock_top_mhz: Option<u32>`; `verify_applied_curve` passes `None` (byte-identical);
  `real_probe_step` passes `Some(seed.stock_boost_max_mhz)`, accepts `VerifiedCurve ||
  stock_equivalent`, logs the branch as `verify=StockEquivalentCeiling stock_equiv_bins=N`.
  Safe-Loop/reset/abort flow in the probe unchanged. Condition 1 is DIRECTIONAL (`target ≤ top &&
  top − target ≤ tol`) — a target above the stock top is an overclock, never stock-equivalent.
- **Files**: `crates/service/src/gpu_verify.rs`, `crates/service/src/gpu_power_sweep.rs`.
- **Tests**: `cargo check` clean · service **109/109** (+11: first-run reproduction accepted;
  plateau-miss / any-overshoot / below-boost-top / zero-offset-not-exact / no-stock-top /
  unreadable-offset all rejected; normal VerifiedCurve never consults the path; fully-degenerate
  all-zero-at-target accepted; tol-boundary accept@15 / reject@16 on bin-freq AND target-vs-top;
  directional above-top reject) · core 46/46.
- **Next**: re-run the bounded DRY-RUN on the rebuilt binary, then (user approval required) the
  SAME bounded `--confirm` — expect the 1935-target probe to verify stock-equivalent and dwell
  (~6 probes, ~120 s). 11D (persisted stock base / exact-offset proof) still deferred.

## Backend checkpoint (2026-06-08) — F1b Phase 2B.2-c.0: first-run limiter flags (pushed, 6881cd7)
- **Bounded first run.** `build-frontier` gains `--max-targets N`, `--max-probes N`,
  `--safe-start-cap MV` so the first supervised QA validates the pipeline without the full 84-dwell
  plan. Dry-run + confirmed both honor them; defaults preserve the full plan.
- **Behavior**: `--max-targets` truncates to the top N; `--safe-start-cap` lowers the descent start
  to the cap when below the derived cluster top (never raises above it, never below the crash floor);
  `--max-probes` hard-stops total probe executions (short-circuits remaining, then resets to stock +
  clears the flag). FAIL CLOSED on absurd values (0; cap ≤ crash floor; non-numeric/missing).
- **Pure helpers**: `FrontierLimits` / `validate_limits` / `apply_frontier_limits` (gpu_power_sweep);
  `parse_frontier_limits` (main.rs). Dry-run prints a `limits` line + the capped dwell budget.
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/service/src/main.rs`. **No IPC/contract/
  core/apps-ui/Safe-Loop/gpu_apply/nvml_gpu/Phase-3/11D change; no auto-apply; no persistence; no
  hardware.**
- **Tests**: `cargo check` clean · service **95/95** (+7: validate/apply/max-probes-cap + 3 parse
  tests) · core 46/46.
- **Dry-run QA** (stock, no --confirm, no state-file writes — mtimes unchanged):
  `build-frontier --max-targets 1 --max-probes 6 --safe-start-cap 1075` → targets=[1935],
  descent 1075→875 mV (9 bins), 6 dwells (~120 s, capped by --max-probes). NB: the soft-max warning
  still cites the derived cluster top (1150 mV) even when --safe-start-cap lowers the effective start
  (1075) — accurate about the curve, cosmetic next to the capped descent.
- **Next — Phase 2B.2-c (supervised hardware QA, separately gated)**: a bounded `--confirm` run
  (e.g. the flags above) with the user present and able to reboot. 11D deferred to after Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.4: stock core VF cluster seeding (IMPLEMENTED, not pushed)
- **Refines b.3.** b.3's generic guard rejected absurd values but still let `safe_start` = global max
  of all sane points (1150 mV on the 3060 Ti — the hard-cap boundary / a non-core point). b.4 derives
  safe_start/boost from the actual contiguous core VF cluster instead.
- **`select_core_cluster`** (pure, `gpu_power_sweep.rs`): sort sane points by voltage; split into
  contiguous runs where voltage gap ≤ 60 mV; pick the LARGEST (ties → lowest voltage = dense core);
  FAIL CLOSED if < 8 points. `derive_core_seed` seeds boost/safe_start from the cluster top and
  reports isolated high-V outliers above it. b.3 generic hard guards (500..3500 MHz, 600..1150 mV)
  retained.
- **Dry-run diagnostics** now print raw/retained/rejected counts, rejected extremes, selected
  core-cluster mV+MHz range, outliers-above count, stock reference (cluster top), safe_start source,
  and a WARNING when a profile appears applied (`gpu_apply::load_applied()`).
- **Files**: `crates/service/src/gpu_power_sweep.rs` + docs. **No IPC/contract/core/apps-ui/Safe-Loop/
  gpu_apply/nvml_gpu/Phase-3/11D change; no auto-reset; no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **88/88** (cluster tests: isolated
  1150 rejected; ends-at-1075→1075; legit-1150→1150; empty/ambiguous fail-closed; targets seed from
  cluster not outlier; diagnostics report cluster range) · core 46/46.
- **Stock dry-run QA PENDING the user's manual reset to stock** (this patch does NOT auto-reset).
  Then run `nidavellir-service.exe build-frontier` (no --confirm) and confirm: no arm/apply/dwell/
  VF-write, no state-file mtime change, plausible targets, safe_start = stock core cluster top,
  applied-profile warning if not reset. **`--confirm` remains forbidden until reviewed.**
- NB: b.3 + b.4 are both UNCOMMITTED — the eventual commit bundles them unless split. Future: NVML
  `max_clock_info(Graphics)` could corroborate boost (nvml_gpu.rs frozen here).

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.3: core-domain seeding guard (IMPLEMENTED, not pushed)
- **Safety fix.** The first `build-frontier` dry-run (read-only) caught a seeding bug:
  `run_build_frontier` derived candidate clocks + safe_start from the UNFILTERED global max of
  `read_vf_curve_modern()` (includes non-core / memory-domain points) → bogus plan (targets
  7001..6311 MHz, safe_start 1237 mV). The dry-run gate blocked it with zero hardware risk.
- **Guard** (pure, `gpu_power_sweep.rs`): `sane_core_points` keeps freq ∈ [500,3500] MHz & voltage ∈
  [600,1150] mV; `derive_core_seed` seeds boost/sustained/safe_start from sane points only, records
  rejected max freq/voltage, soft-warns (>3200 MHz / >1125 mV), FAILS CLOSED (Err) if no sane points
  or a derived value exceeds a hard guard. `run_build_frontier` aborts (no arm/apply/dwell/VF-write)
  on Err or any candidate target > 3500 MHz. Consts are sanity guards, NOT tuning targets.
- **Files**: `crates/service/src/gpu_power_sweep.rs` + docs. **No IPC/contract/core/apps-ui/
  Safe-Loop-behavior/gpu_apply/nvml_gpu/Phase-3/11D change; no auto-reset; no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **86/86** (+5 guard tests:
  sane_core rejects 7001/1237 & keeps plausible; seed uses sane max not global max; fail-closed on no
  sane points; targets never > hard max; soft-limit warnings) · core 46/46.
- **Dry-run QA (read-only, no --confirm, no state writes — all 4 mtimes unchanged)**: 132 raw VF
  points → 88 sane-core retained, 44 rejected (incl. 7001 MHz / 1237 mV); boost~1935 MHz; targets
  [1935,1905,1875,1845,1815,1785,1755]; 1150→875 mV step 25 (12 bins); 84 worst-case dwells
  (~1680 s); WARNING safe_start 1150 mV > soft max 1125 mV. **NB**: the live curve is in an APPLIED
  state, so the numbers reflect the applied curve, not stock; a stock read (reset first) would be
  cleaner, and safe_start 1150 mV is high for a 3060 Ti core (~1075) → review before --confirm.
- **`--confirm` remains forbidden** until this fixed plan is reviewed. **Next — Phase 2B.2-c**
  (supervised hardware QA, separately gated): optionally reset to stock first for a clean plan,
  re-review the dry-run, then `--confirm` with the user present and able to reboot. 11D deferred.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.2: real probe + supervised build-frontier (CODE ONLY, not run, not pushed)
- **Real Windows probe `real_probe_step`** (the `build_frontier` seam under `--confirm`):
  abort/boundary guard → snap vbin to a real VF bin → arm Safe Loop → `apply_vf_ceiling(bin,target)`
  → read-only verify via shared `classify_live_ceiling` (+ 11C diag log) → on not-VerifiedCurve
  reset+clear+return → `load_and_measure` dwell → clear flag → `measured_to_probe` + set `vf_bin_mv`.
  Dwell CRASH → reset to stock + set `abort` so remaining probes short-circuit (run drains safely);
  a normal Unstable/unverified only stops that clock's descent.
- **`run_build_frontier(store, confirm)`** + console `build-frontier` (main.rs): always prints the
  `plan_frontier` plan. Dry-run (no `--confirm`) = read-only (no arm/apply/dwell/VF-write, no startup
  recovery). `--confirm` = startup recovery (parachute) first, then `build_frontier` with the real
  probe, then ALWAYS `reset_to_stock` + clears the flag. **No auto-apply; no forge_state; no
  gpu_knowledge writes.**
- **Conservative first-run consts** (review the printed dry-run plan before any run): lowest_safe=875
  mV (above the ~855 mV known reboot), 25 mV step, 30 MHz clock step, 0.90 floor; idle Unconstrained
  regime clamped → PowerLimited (no OC on a first run); sustained ≈ curve top freq; confidence 0.21.
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/service/src/main.rs`. **No IPC/contract
  /core/apps-ui/Safe-Loop-behavior/gpu_apply/nvml_gpu/Phase-3/11D change. Hardware path NOT executed.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **81/81** (+1 `--confirm` arg parse)
  · core 46/46. `real_probe_step`/`run_build_frontier` are hardware → not unit-tested; the abort
  short-circuit PATTERN is covered by the 2B.2-b.1 fake-probe test.
- **Commands**: dry-run `nidavellir-service.exe build-frontier`; confirmed (DO NOT RUN until QA)
  `nidavellir-service.exe build-frontier --confirm`.
- **Next — Phase 2B.2-c (supervised hardware QA, separately gated)**: run the dry-run, review the
  plan, then `--confirm` with the user present and able to reboot; verify gpu_applied.json /
  forge_state.json unchanged, boot flag armed/cleared per probe, abort on TDR. 11D deferred to after
  Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-b.1: seeding + dry-run plan + vf_bin (IMPLEMENTED, not pushed)
- **Pure prep for 2B.2-b.** Exposed `classify_live_ceiling` / `LiveCeilingEval` / `CurveDiag` as
  `pub(crate)` in `gpu_verify.rs` (intra-crate visibility only — NO IPC/contract change) so the
  future transient-ceiling probe reuses one classification path.
- **Pure seeding** in `gpu_power_sweep.rs`: `derive_descent(curve_bins, lowest_safe, step) ->
  FrontierDescent` (safe_start = top live bin, clamped ≥ operator crash floor) + read-only
  `plan_frontier(targets, &descent, dwell_ms) -> FrontierPlan` (worst-case dwell count + wall-time +
  safety notice). Targets via existing `classify_regime` / `candidate_clocks`.
- **Internal `ProbeSample.vf_bin_mv: Option<u32>`** (NOT IPC): the actually-applied snapped bin.
  `probe_to_point` records `vf_table_voltage_mv = vf_bin_mv.or(descent vbin)`; `measured_to_probe`
  leaves it None (the real probe fills it after the apply in 2B.2-b.2).
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/service/src/gpu_verify.rs`. **No real
  probe; no `apply_vf_ceiling`/`load_and_measure`; no `build-frontier` subcommand / `--confirm`; no
  Safe-Loop arm/clear; no startup-recovery wiring; no forge_state / gpu_knowledge writes; no
  Phase-3/11D/apps-ui/core/contract change; no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **80/80** (+7 pure: regime→targets,
  derive_descent, plan_frontier estimates, vf_bin propagation + fallback, mapper-leaves-None,
  build_frontier abort short-circuit via fake probe) · core 46/46 (untouched).
- **Next — Phase 2B.2-b.2 (NOT started, separately gated)**: real `#[cfg(windows)]` probe closure
  (arm Safe Loop → `apply_vf_ceiling(vbin,target)` → `classify_live_ceiling` verify + 11C diag →
  `load_and_measure` dwell → clear → `measured_to_probe` + set `vf_bin_mv`) + supervised
  `build-frontier --confirm` console subcommand (dry-run default via `plan_frontier`; runs startup
  recovery; print/log-only, no auto-apply, no persistence). Then supervised hardware QA (2B.2-c).
  11D deferred to after Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.2-a: shared live-ceiling classifier (IMPLEMENTED, not pushed)
- **Pure refactor** in `gpu_verify.rs`: extracted `classify_live_ceiling(live, ceiling_idx,
  ceiling_mv, target, tol)` (read-only; offset-readback evidence build) + pure
  `eval_ceiling_evidence(target, anchor_idx, &expected, tol)` (runs the UNCHANGED offset-presence
  `classify_curve` gate + 11C `compute_curve_diag`) → `LiveCeilingEval`. `verify_applied_curve` now
  routes through it.
- **Behavior identical**: `VerifyAppliedProfile` output is byte-for-byte unchanged (same classifier,
  diagnostic, inputs); only inline duplication removed. Offset-presence stays the gate; plateau spread
  stays diagnostic; voltage never affects classification. This is the shared path the 2B.2-b
  transient-ceiling probe will reuse to verify a JUST-applied ceiling (not the persisted profile).
- **Files**: `crates/service/src/gpu_verify.rs` only. **No core/contract/`apps/ui`/Safe-Loop/synthesis
  /Phase-3/11D change; no real probe; no `apply_vf_ceiling`/`load_and_measure`; no `build-frontier`
  subcommand; no hardware.** Pure seeding helpers deferred to 2B.2-b (would be dead code now).
- **Tests**: `cargo check -p nidavellir-service` clean · service **73/73** (+5 `eval_ceiling_*` pure
  tests; all pre-existing verify tests green) · core 46/46.
- **Next — Phase 2B.2-b (NOT started, separately gated)**: real `#[cfg(windows)]` probe closure (arm
  Safe Loop → `apply_vf_ceiling(vbin,target)` → `classify_live_ceiling` verify + 11C diag →
  `load_and_measure` dwell → clear → `measured_to_probe`) + supervised `build-frontier --confirm`
  console subcommand (print/log-only; runs startup recovery; no auto-apply). Then supervised hardware
  QA. 11D deferred to after Phase 2B.

## Backend checkpoint (2026-06-07) — F1b Phase 2B.1: pure probe-mapping prep (IMPLEMENTED, not pushed)
- **Pure, hardware-free half of Phase 2B.** `measured_to_probe(&Measured, curve_verified, confidence)
  -> ProbeSample` in `gpu_power_sweep.rs` — the seam the real probe closure (2B.2) will use to feed
  `build_frontier`. No hardware I/O; conservative interpretation of already-collected dwell data only.
- **Conservative rules**: Stable→`ProbeOutcome::Stable` ONLY if clock/power quality ≥ Medium AND p5
  present; else (SilentError / Crash / TDR-degenerate, or weak telemetry) → Unstable. p5 preserved
  (0 → None); measured voltage = ramp-filtered avg, None when missing (never 0).
- **Additive schema**: `PowerSweepPoint.target_clock_mhz: Option<u32>` (serde default, no schema
  bump). Phase 2A `probe_to_point` now stamps the target; the single-clock live sweep sets None.
- **Files**: `crates/service/src/gpu_power_sweep.rs`, `crates/core/src/ipc.rs`,
  `docs/contracts/ui-backend.md`, decisions/memory/handoff. **No real probe, no `apply_vf_ceiling`,
  no `load_and_measure` loop, no supervised console cmd, no Safe-Loop/synthesis/`apps/ui`/Phase-3/11D,
  no hardware.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **68/68** (+7 mapping/target tests) ·
  core **46/46** (+2 serde roundtrip + legacy-load). No hardware run.
- **Next — Phase 2B.2 (NOT started, separately gated)**: the real `#[cfg(windows)]` probe closure
  (arm Safe Loop → `apply_vf_ceiling(vbin,target)` → read-only offset-readback verify + 11C diag →
  `load_and_measure` dwell → clear → `measured_to_probe`) + a supervised console subcommand that calls
  `build_frontier` with it behind explicit confirm. Then a supervised hardware QA run. 11D
  (exact-offset stock-base persistence) deferred to AFTER Phase 2B unless QA shows need.

## Backend checkpoint (2026-06-06) — Patch 11C: read-only live VF-ceiling diagnostic (IMPLEMENTED, not pushed)
- **Read-only diagnostic** added to `gpu_verify::verify_applied_curve` (and the `verify-applied`
  console subcommand): pure `compute_curve_diag` over the existing per-point evidence + one
  `LiveSnapshot`. No mutation, no stress, no apply. Classifier semantics UNCHANGED (offset-presence
  gate; live voltage above the VF anchor never downgrades; GetStatus freq stays diagnostic).
- **New evidence**: first modified bin idx/mv, modified vs expected bin count, GetStatus freq-match,
  GetStatus plateau min/max MHz, max target overshoot/undershoot, 3 offset samples (first/anchor/
  highest), and a live snapshot (NVAPI voltage + first NVML clock/power/util/temp/limit/cap). Surfaced
  via additive `Option`/`serde(default)` fields on `ApplyVerificationStatus` + one `apply_verify_diag:`
  log line. Additive IPC documented in `docs/contracts/ui-backend.md`.
- **Files**: `crates/service/src/gpu_verify.rs`, `crates/core/src/ipc.rs`,
  `docs/contracts/ui-backend.md`, `decisions.md`, `memory.md`, this file. **No apply/Safe-Loop/
  synthesis/F1b/`apps/ui`/`nvml_gpu.rs` change. P-state + full ThrottleReasons deferred.**
- **Tests**: `cargo check -p nidavellir-service` clean · service **61/61** (+9 pure diag tests) ·
  core **44/44** (additive serde fields, nothing broken).
- **Runtime QA** (`verify-applied`, read-only — confirmed non-mutating: all four
  `%ProgramData%\Nidavellir\*.json` mtimes unchanged across the run): curve=`VerifiedCurve` (62/64
  offsets present), load=`VerifiedUnderLoad`. Diagnostic revealed `anchor_offset_khz=+255000`,
  `highest_bin_offset_khz=−120000`, GetStatus plateau **1770–1830 MHz** (overshoot 45, undershoot 15),
  live `voltage_mv=1068 clock_mhz=1815 util_pct=6 temp_c=47 power_w=66 cap=200W capped=false`.
  Interpretation: offsets are resident and *curve-flatten-shaped* (big `+` at the 843 mV anchor, `−`
  at the top) → curve IS applied; the plateau spread + overshoot is consistent with BOTH normal
  GPU-Boost behavior AND the open overshoot suspect, but GetStatus idle noise (freq_match 18/64) keeps
  it **non-conclusive** — exactly what 11C was meant to surface. Live voltage 1068 mV ≫ 843 mV anchor
  confirms (again) measured voltage is NOT capped (telemetry only).
- **Exact-offset verification still deferred**: expected offset = `target − stock_base_mhz`, but
  per-point stock base is not persisted and GetStatus freq is idle-unreliable. Future "11D" options:
  persist the pre-apply stock curve, or validate the GetStatus `base` tuple (`StatusEntry.base`,
  currently decoded but discarded in `vfcurve::get_status`). Only then can the overshoot suspect be
  proven/refuted.
- **F1b Phase 2B**: still NOT started; UNBLOCKED by this diagnostic — Phase 2B's `curve_verified`
  gate (offset-readback) is the same axis 11C reports, so the supervised HW run can now log the
  plateau/offset evidence per dwell. Sequence the Codex copy fix + (optionally) 11D before relying on
  exact-offset proof.

## Backend checkpoint (2026-06-06) — Applied voltage semantics (Patch 11A, DOCS ONLY, not pushed)
- **Read-only investigation** confirmed the elastic VF ceiling caps **frequency, not voltage**:
  `apply_vf_ceiling` (`crates/gpu-nvapi/src/lib.rs`) writes per-point FREQUENCY offsets to every
  modern VF point whose table voltage ≥ the selected bin (flatten to `target_mhz`); points below
  are untouched. It writes **no voltage** and does **not** hard-cap measured/rail voltage in any
  P-state. The apply key is the deterministic `vf_table_voltage_mv` (VF/curve bin), re-derived by
  snapping measured voltage UP to the lowest table bin ≥ it (`nearest_vf_bin_at_or_above`).
- **Semantics resolved**: `measured_voltage_mv` / HWiNFO "GPU Core Voltage" are a DIFFERENT (rail,
  load-line/droop) domain and may legitimately read ABOVE the VF bin — idle ~1.075 V and in-game
  ~0.887–0.956 V for an ~850 mV bin are EXPECTED, not a mismatch. `VerifyAppliedProfile` proves
  offset PRESENCE (+ a stored-dwell load axis), nothing about effective voltage. Nidavellir must
  NOT imply a hard voltage cap; a true cap = the legacy voltage-lock (TDR) path → rejected.
- **Patch 11A (this change) = DOCS/CONTRACT ONLY**: updated `decisions.md` (new doctrine entry),
  `docs/contracts/ui-backend.md` (semantics clarification + Codex wording request: drop "MHz @ mV",
  use "target" + "VF bin", keep measured voltage separate), `memory.md`, this file. **No backend
  code, no `apps/ui`, no apply/verify change, no F1b Phase 2B, no hardware.**
- **Open suspect (deferred, read-only-testable — Patch 11C, not started)**: offsets are computed as
  `target − GetStatus_base` and GetStatus under-reports freq at idle → a plateau applied at idle may
  land above `target` (consistent with observed ~1815–1830 MHz vs ~1785, on top of normal 15 MHz
  boost-bin quantization). To be confirmed by a future read-only live diagnostic — NOT changed here.
- **Does NOT block F1b Phase 2B** (it already keys on the VF bin + offset-readback VerifiedCurve
  gate); sequence the Codex copy fix + (optional) 11C live diagnostic before the supervised HW run.

## Backend checkpoint (2026-06-06) — F1b Phase 2A: simulated multi-clock loop (DONE, not pushed)
- **`build_frontier(candidate_clocks, &FrontierDescent, &ForgePolicy, probe: impl Fn(u32,u32)->
  ProbeSample)`** in `gpu_power_sweep.rs` proves the multi-clock outer loop, per-target voltage-bin
  descent, stopping rules, known-unsafe boundary, frontier assembly, and synthesis wiring **without
  hardware**. The injected probe closure is the only seam to (future) hardware.
- **Loop rules**: descend from `safe_start_mv` by `voltage_step_mv`, never below `lowest_safe_mv`
  (known-crash floor as config); keep deepest stable; stop on first `Unstable`; stop/drop on
  simulated `curve_verified=false` (Phase-2B Patch-A gate); drop a clock with no stable point.
  Partial frontier allowed; empty → synthesis all-`None` (safe). Points record `vf_table_voltage_mv`
  (deterministic bin); measured voltage stays telemetry.
- **No hardware wired**: no `load_and_measure`, no `apply_vf_ceiling`, no VF write, no stress, no
  Safe Loop interaction, no real power sweep. New types/fn `#[cfg(windows)] #[allow(dead_code)]`.
- **Files**: `crates/service/src/gpu_power_sweep.rs` only. No IPC/persistence/`apps/ui` change.
  `cargo check` clean · service **52/52** (+8 sim; 3060 Ti 1830/1815/1740 + 4090 2880/2860/2700
  proven through the loop).
- **F1b Phase 2B (next, NOT started)**: fill the real probe closure — apply ceiling at the bin →
  Safe-Loop-armed `load_and_measure` dwell → offset-readback `VerifiedCurve` gate → map to
  `ProbeSample`; wire `build_frontier` into a **supervised/approval-gated** entry point; feed
  `candidate_clocks(...)` from a live `classify_regime`; add `target_clock_mhz` to points if needed.
- **Phase 3 (future)**: knowledge re-key to `(target_clock, vf_table_voltage_bin)` + global
  voltage-floor crash boundary; backward-compatible `gpu_knowledge.json` migration.

## Backend checkpoint (2026-06-06) — F1b Phase 1: policy-driven multi-clock synthesis (DONE, pushed)
- Pure, service-internal in `gpu_power_sweep.rs`. **`ForgePolicy`** centralizes thresholds —
  Balanced `brokkrs_min_clock_frac=0.98` / `deep_calm_min_clock_frac=0.90` / `confidence_threshold=
  0.85`; Conservative (0.99/0.92/0.95) and Aggressive (0.97/0.85/0.70) presets.
- **`synthesize_forge_profiles(frontier, &ForgePolicy)`** now applies clock floors:
  Godforge = highest **sustainable** clock (prefers `p5_clock_mhz`, falls back to `clock_mhz`;
  ties→lowest power); **Brokkr's = max R within the Brokkr's clock floor** (real trade: clock<gc,
  power<gp); Deep Calm = max MHz/W within the Deep Calm floor. **Selection never uses measured
  voltage** — `vf_table_voltage_mv` stays the deterministic apply axis. **Single-clock collapse**
  detected + logged (still returns all three). **4090 doc ambiguity resolved: Brokkr's = 2860**
  (max-R-within-floor).
- Added Phase-2 helpers (pure, `#[allow(dead_code)]` until wired): `Regime` enum,
  `classify_regime(...)`, `candidate_clocks(...)`.
- **Files**: `crates/service/src/gpu_power_sweep.rs` only. No IPC, no `apps/ui`, no Safe Loop,
  no hardware path. `cargo check` clean · service **44/44** (3 F1a tests unchanged + 9 F1b).
- **F1b Phase 2 (next, NOT started)**: real multi-clock measurement loop over the safe flatten
  sweep — build a **simulated/inject outer-loop scaffold first** (test loop/knowledge/stopping
  without a GPU), then a **supervised, approval-gated** hardware run; verify the ceiling per dwell
  (Patch A offset readback); SyntheticDwell context only; add `target_clock_mhz` to points then.
- **Phase 3**: re-key knowledge by (target_clock, vf_table_voltage_bin) + global voltage-floor
  crash boundary; backward-compatible `gpu_knowledge.json` migration.

## Backend checkpoint (2026-06-06) — Forge action consolidation audit (recorded, no code change)
- Backend has **two engine generations**. **Canonical Forge GPU core path = `gpu_power_sweep.rs`
  (Power Sweep)**: `set_core_offset_mhz` + `apply_vf_ceiling` (elastic VF ceiling), game-power
  render dwell, Safe-Loop-guarded, **no voltage lock**. Apply via `ApplyPowerGodforge/Brokkrs/
  DeepCalm`. **F1b must extend ONLY this engine.**
- **Legacy (voltage-lock, TDR risk)**: `gpu_sweep_real.rs` (Real Sweep — `lock_core_voltage_mv`
  L239/L370, ALU load) and `gpu_forge_all.rs` (Forge Everything — fixed `CORE_VOLTAGE_MV=900`
  lock L193, VRAM around a fixed-voltage core) + the legacy `ApplyGodforge/Brokkrs/DeepCalm` trio.
  → hide from normal UI, schedule removal AFTER F1b. Keep IPC methods wired for now (no mid-stream
  break).
- **Memory/VRAM** (`gpu_mem_sweep.rs`): no core voltage lock, but runs independent of the forged
  core. **VRAM tuning remains future work and must adapt to the forged core curve** (run after
  core VF forge + validation, never define/destabilize it). Advanced Diagnostic until redesigned.
- **Action audit table + answers**: see this session's audit; frontend request in
  `docs/contracts/ui-backend.md`; rationale in `decisions.md`. No code removed, no `apps/ui` change.

## Backend checkpoint (2026-06-06) — Patch B load-state classification (IMPLEMENTED, pushed)
- Adds an orthogonal **LOAD axis** to `ApplyVerificationStatus`: `load_state: LoadVerification`
  (`NotEvaluated/VerifiedUnderLoad/TelemetryInsufficient/LoadMismatch/WorkloadStateMismatch
  (reserved)/LoadVerificationFailed`) + `load_reason`, `telemetry_match`, and diagnostic dwell
  fields (`p5_clock_mhz`, `min_clock_mhz`, `avg/min/max_measured_voltage_mv`,
  `voltage_sample_count`, `voltage_quality`, `telemetry_quality`). `status` stays the curve axis.
- **Source**: existing synthetic-dwell stats only — NO new stress run. `gpu_power_sweep::
  load_restored_progress()` (read-only, reads `forge_state.json`) → `find_applied_point` matches
  by label→named slot (Godforge/Brokkr's Best/Deep Calm) with a clock check, fallback = unique
  `points` entry; ambiguous→None. `classify_load`: curve must be VerifiedCurve; `p5_clock ≥
  target−30 MHz` (two bins) AND `telemetry_quality ≥ Medium` → VerifiedUnderLoad; voltage is
  telemetry-only (implausible→TelemetryInsufficient); `stable=false`→LoadMismatch; bad power→
  LoadVerificationFailed; missing p5/quality→TelemetryInsufficient. `effective_status` derivation:
  load upgrades VerifiedCurve→VerifiedUnderLoad, never downgrades; LiveMismatch stays LiveMismatch.
- **Files**: `crates/core/src/ipc.rs` (LoadVerification + fields), `crates/service/src/gpu_verify.rs`
  (find_applied_point, classify_load, effective_status, fill_load_axis, tests),
  `crates/service/src/gpu_power_sweep.rs` (load_restored_progress), `docs/contracts/ui-backend.md`.
  Additive only; `verify-applied` stays read-only.
- **Tests**: check clean · service 35/35 (+10 load tests).
- **Runtime QA** (`verify-applied`, read-only): curve=VerifiedCurve(63/65), forge_state loaded
  (17 pts), matched Brokkr's slot, **load_state=TelemetryInsufficient** ("legacy point without
  dwell quality" — the persisted point predates the richer-dwell-stats patch), status=verified_curve.
  No writes (`gpu_applied.json` + `forge_state.json` mtimes unchanged). To get VerifiedUnderLoad a
  fresh sweep (HW, supervised) must produce a point carrying the new dwell stats.
- **Limitations**: WorkloadStateMismatch reserved (live real-game context = future); load axis only
  as good as the persisted dwell stats. **Next: Forge Action Consolidation.**

## Backend checkpoint (2026-06-06) — Applied curve verifier, Patch A (IMPLEMENTED, pushed)
- **Read-only `VerifyAppliedProfile` IPC** + new `crates/service/src/gpu_verify.rs`. Answers
  "does the live modern VF curve match the applied profile?" → `CurveVerification` =
  `NotApplicable | MetadataOnly | VerifiedCurve | LiveMismatch | VerificationFailed`.
- **Table-to-table only**: re-derives the deterministic ceiling bin the same way apply does
  (`nearest_vf_bin_at_or_above(core.voltage_mv)` — NOT measured voltage); reads
  `read_vf_curve_modern` (GetStatus) + `vf_get_point_khz` (offset corroboration, logged only).
  Rule: points with `mv ≥ ceiling` should read `target ±15 MHz`; ≥90% match (and ≥1) →
  VerifiedCurve, else LiveMismatch; empty/unmappable → VerificationFailed.
- **Read-only**: never applies/reapplies/writes/stresses. Patch B (telemetry/load),
  Patch C (workload context, stock fingerprint, ExternalUnknown) NOT implemented.
- **Files**: `crates/core/src/ipc.rs` (enum `CurveVerification`, `ApplyVerificationStatus`,
  `VerifyAppliedProfile` request, `ApplyVerification` response), `crates/service/src/gpu_verify.rs`,
  `main.rs` (mod), `ipc_server.rs` (handler), `docs/contracts/ui-backend.md`. Additive only.
- **Tests**: `cargo check -p nidavellir-service` clean · service 26/26 (+7 verifier pure tests).
- **Read-only runtime path (2026-06-06)**: added console subcommand
  `nidavellir-service.exe verify-applied` (`run_verify_only` in `main.rs`) — runs the verifier
  with NO `run_startup_recovery`/`spawn_heartbeat`/`reapply_on_boot`/pipe server, so **no apply,
  no VF write**. Prints `ApplyVerificationStatus` JSON + the `apply_verify:` log. Proven
  non-mutating (`gpu_applied.json` mtime unchanged across a run).
- **Patch A.1 — offset-based verification (2026-06-06) — DONE**: runtime QA proved GetStatus
  actual-freq is unreliable at idle (under-reported the plateau 31/65 while the flatten offsets
  were resident 63/65). `classify_curve` now gates on the **GET-control offset readback**
  (`vf_get_point_khz`): a point ≥ ceiling counts as flattened if it carries a **non-zero** offset
  (presence, not exact value — per-point stock base isn't persisted); ≥90% → VerifiedCurve;
  unreadable offsets → VerificationFailed (safer than mismatch). GetStatus freq match stays a
  logged diagnostic (`getstatus_freq_match=...`). Re-ran `verify-applied` → **VerifiedCurve**
  (offset_match 63/65, getstatus 31/65), no write (`gpu_applied.json` mtime unchanged). Service
  25/25, check clean. **Known caveat**: presence-only offset check can't yet distinguish a
  Nidavellir flatten from an external tool's offsets (ExternalUnknown = Patch C); and it can't
  detect an offset that's present but wrong-valued (would need persisted stock base).
- **Unblocks**: Patch B (load classification) can reuse the applied `PowerSweepPoint` dwell stats.

## Backend checkpoint (2026-06-05) — Richer dwell stats (IMPLEMENTED, pushed)
- Second patch off the Sensor Audit. **`PowerSweepPoint` gains optional dwell-quality
  fields**: `min_clock_mhz`/`p5_clock_mhz`, measured-voltage `avg/min/max` +
  `voltage_sample_count`, `dwell_sample_count`/`dwell_duration_ms`, `start/end/avg_temp_c`,
  and `voltage_quality`/`telemetry_quality` (new `DwellQuality` enum in `core/ipc.rs`:
  high/medium/low/unavailable).
- **Voltage stats are ramp-filtered + sanity-checked (500–1250 mV)**; the legacy unfiltered
  voltage max (`volt_mv` → `voltage_mv`/`measured_voltage_mv` + the apply-key snap) is
  **UNCHANGED** (restriction: don't touch the apply-key decision). min/p5 clock from the
  retained post-ramp clock samples; temp from NVML per-sample reads. Per-point
  `dwell_stats:` log line (not per-sample).
- **Files**: `crates/core/src/ipc.rs`, `crates/service/src/gpu_power_sweep.rs`,
  `docs/contracts/ui-backend.md`. No `apps/ui`, Safe Loop, synthesis, or F1b change.
  Additive serde-default fields; `PowerSweepPoint` stays `Copy`; old `forge_state.json` loads.
- **Tests**: `cargo check -p nidavellir-service` clean · core 44/44 · service 19/19.
- **Limitations (next work)**: full NVML limiter reasons deferred (needs `NvmlGpuReading`
  in core); voltage cadence still ~480 ms (≈Medium quality, now surfaced); no per-sample
  timestamps; no hotspot/fan; `arduous_validate` soak path doesn't yet use the richer stats.

## Backend checkpoint (2026-06-05) — Voltage field separation (IMPLEMENTED, pushed)
- First patch off the Sensor Audit decision. **`PowerSweepPoint` now separates
  `measured_voltage_mv` (telemetry) from `vf_table_voltage_mv` (deterministic apply/
  frontier key)**; legacy `voltage_mv` retained for compat/display.
- **Apply path snaps measured voltage → real VF-table bin** (`nearest_vf_bin_at_or_above`
  in gpu-nvapi; `choose_ceiling_mv` in `gpu_apply.rs`) **before `apply_vf_ceiling`** — no
  longer keys the ceiling on raw measured voltage. Logs `voltage_semantics: …`.
- **Backward-compatible**: no schema bump; old `forge_state.json`/`PowerSweepPoint` JSON
  loads new optional fields as `None`; `VfPoint`/`gpu_applied.json` unchanged → apply
  re-snaps at runtime (legacy warning only if the live curve is empty). Additive IPC
  fields documented in `docs/contracts/ui-backend.md`.
- **Files**: `crates/gpu-nvapi/src/lib.rs`, `crates/core/src/ipc.rs`,
  `crates/service/src/gpu_apply.rs`, `crates/service/src/gpu_power_sweep.rs`,
  `docs/contracts/ui-backend.md`. No `apps/ui`, Safe Loop, or synthesis change.
- **Tests**: `cargo check -p nidavellir-service` clean · gpu-nvapi 5/5 · service 15/15.
- **Limitations (next work)**: frequency-only flatten unchanged; the ~1062 mV unfocused/
  desktop state is NOT solved here; richer dwell stats + applied-curve verification pending.

## Backend checkpoint (2026-06-05) — Sensor Quality Audit (Review 2, investigation-only)
- **No code/IPC/UI change.** GPU telemetry sources are right (NVML clock/power/cap/temp/
  util; NVAPI curve). Three structural gaps found:
  1. **Two disconnected telemetry worlds**: "sensor world" (`SensorEngine`/`GpuSensors`,
     **30 s cache, `voltage_mv` hardcoded `None`** → UI never gets GPU voltage) vs
     "sweep world" (`load_and_measure`, NVML 30 ms + NVAPI voltage ~480 ms, stored as
     `fetch_max`). Nothing reconciles them.
  2. **Voltage is the weakest signal**: NVAPI `core_voltage()` **string-parsed**, sparse,
     **max-only**, ramp-unfiltered — then **reused as the deterministic `apply_vf_ceiling`
     threshold** (`PowerSweepPoint.voltage_mv` → `AppliedProfile.core.voltage_mv` →
     `ceiling_mv`). This is the root of 837-vs-869 and makes apply fidelity unprovable.
  3. **One name, three meanings**: `voltage_mv` on `PowerSweepPoint` (measured max),
     `VfCurvePoint`/GetStatus (VF-table), `AppliedProfile.core` (measured, consumed as
     curve threshold).
- **KEY DECISION (see `decisions.md`)**: split voltage into **`vf_table_voltage_mv`**
  (deterministic, the **apply/frontier key**) · **`measured_voltage_mv`** avg/min/max
  (telemetry + HWiNFO cross-check only, never an apply key) · **`effective_rail_voltage_mv`**
  (future). **F1b must NOT key on measured dwell voltage.**
- **Verdicts NOT finalized**: 837-vs-869 ≈ undersampling + bin quantization (expected,
  not apply failure); constant ~1062 mV unfocused/desktop ≈ workload-scoped (P0/3D)
  ceiling leaving other states on stock curve + frequency-only flatten leaving voltage
  uncapped. To be confirmed by the verification work.
- **Other gaps**: perf-limiter only reads `SW_POWER_CAP` (NVML exposes the full
  `ThrottleReasons` set — thermal/voltage/util discarded); no timestamps/p5-clock-dip
  stats; no workload-context tag; no cross-validation (0 mV / 0 W dropouts stored as real).
- **Post-audit sequencing** (F1b stays on hold until 1–3 land): (1) split voltage fields +
  stop keying apply on measured voltage [must-fix]; (2) richer dwell stats + full limiter +
  context tag; (3) finalize Applied Curve Verification (table-to-table GetStatus plateau,
  not vs measured voltage) + verify IPC + `GpuApplyStatus.verification`; (4) F1b on the
  cleaned axis.

## Backend checkpoint (2026-06-05) — Applied Curve Verification review (investigation-only)
- **No code change.** Apply is **write-and-forget** (`apply_core` logs flattened count,
  no readback). `GetAppliedProfile` is **metadata-only** (`gpu_applied.json`, no live
  driver check) — "Applied ✓" = file exists, not curve verified. Verification must use
  the **modern ClkVfPoints GetStatus** path (`read_vf_curve_modern`), not legacy
  `read_curve`/`GetGpuCurve`. The flatten caps **frequency, not voltage** — a high VF bin
  can still be selected. Primitive for verification already exists (`read_vf_curve_modern`),
  just unwired. Feeds directly into the sensor-audit sequencing above.

## Backend checkpoint (2026-06-05) — forge-state persistence
- **F1b is on hold** pending two foundation reviews; cheap lower-clock probe plan is
  NOT approved. Both reviews (persistence/startup + sensor quality) are now done.
- **Shipped**: `forge_state.json` (under `%ProgramData%\Nidavellir`) persists the final
  `PowerSweepProgress` on successful sweep completion (only when a profile exists, so a
  failed sweep can't wipe a good snapshot). Startup seeds `PowerSweepHandle` from it when
  the GPU key (`read_curve().name`) matches; else idle. Fixes a service restart losing
  forged profiles/points/apply buttons. Files: `crates/service/src/gpu_power_sweep.rs`
  (+ `main.rs`, `service_impl.rs` seed both startup paths). Backend-only — no UI, IPC,
  Safe Loop, synthesis, or `gpu_knowledge.json` change.
- **Validation**: `cargo test -p nidavellir-service` → 11/11 pass;
  `cargo check -p nidavellir-service` → no warnings. No GPU stress run.
- **Remaining foundation work (in order)**:
  a) manual restart verification (apply a profile → restart service → UI still shows it);
  b) **must-fix**: split voltage fields (`vf_table_voltage_mv` / `measured_voltage_mv` /
     `effective_rail_voltage_mv`) + stop keying `apply_vf_ceiling` on measured voltage;
  c) richer dwell stats (min/p5 clock, voltage avg/min/max, full `ThrottleReasons`,
     sample_count, timestamps, workload-context tag);
  d) finalize Applied Curve Verification (post-apply GetStatus plateau readback,
     table-to-table; verify IPC + `GpuApplyStatus.verification`);
  e) F1b redesign — only after b–d land and the direction is confirmed; key the frontier
     by (clock + VF-table point), NOT measured voltage.

## Where things stand
- **Brokkr's V1 (continuous per-GPU knowledge): implemented + HW-validated.**
- **V2 (confidence-gated selection): committed (5d72342).** `cargo test` 3/3. The
  gate is now reused as the confidence axis for all 3 product profiles.
- **Product reframe (this session, see `product.md`)**: 3 profiles forged from a
  clock×power frontier. **F1a done** — pure `synthesize_forge_profiles` + tests
  (6/6), not yet wired. **F1b** = produce the real multi-clock frontier.
- Architecture finding: two overlapping sweep engines (safe flatten vs unsafe
  lock-voltage frontier); F1b builds on the flatten one — tech debt to consolidate.
- Last supervised sweep explored to **+210 offset (~881 mV)** with NO crash and
  found Brokkr's = **1830 MHz @ 881 mV · 179 W · 10.24 MHz/W (off-cap)** — essentially
  the user's hand-tuned 1800 MHz @ 875 mV. Godforge = stock max-voltage point.
- GPU is at **stock**; service + UI were running; Safe Loop baseline is clean.

## Build / run (from repo root C:\Users\leona\dev\nidavellir)
- Build service: `cargo build --release -p nidavellir-service`
  (STOP the service first or the .exe is locked:
  `Get-Process nidavellir-service | Stop-Process -Force`).
- Run service (headless): `./target/release/nidavellir-service.exe console`
  (logs → `target/svc.log`).
- Run UI: `cd apps/ui && npm run tauri:dev`.
- Headless control (named-pipe client): `scripts/ipc.ps1 -Method <Name>`:
  `StartPowerSweep` / `GetPowerSweepProgress` / `StopPowerSweep` /
  `ResetGpuTuning` / `ApplyPowerBrokkrs` / `GetGpuCurve` / `GetSafeLoopStatus`.

## Learned knowledge (C:\ProgramData\Nidavellir\gpu_knowledge.json)
- `boundary`: highest_clean **210**, lowest_reboot **255** (silent_error/tdr null).
- 15 per-offset PointStats (0→210), 1 trial each, 0 failures.
- Next sweep's data-driven ceiling = **+240** (~870 mV), then it CONVERGES (cap
  ABS_MAX_OFFSET=240; never re-touches the 255 reboot).

## Pending / next actions
1. **Commit F1a** (synthesis + tests + `product.md`/decisions/roadmap) once reviewed.
2. **F1b**: extend the safe flatten sweep to several target clocks → real game-power
   clock×power frontier; knowledge keying by (clock, offset); wire
   `synthesize_forge_profiles` in (replaces the single-clock godforge/brokkrs picks).
3. Then F2–F7 (see `product.md`).
4. In-game apply test (`ApplyPowerBrokkrs`) — consistency in Overwatch; user present.

## Gotchas / safety
- **Deep undervolt can HARD-REBOOT** (not just TDR). +255/~855 mV did. Never
  auto-run deep exploration — supervised only. The knowledge bounds the search.
- The render is **heavier than real games**, so it destabilizes at a higher voltage
  than games → the validated point is conservative (good), but probing near the
  frontier under it still risks a reboot.
- **In-sweep a hard reboot does NOT auto-update `gpu_knowledge.json`** (only
  SilentError/TDR do). After a reboot, read the Safe Loop `boot_flag.json` offset
  and set `lowest_reboot` in the knowledge manually (until the integration lands).
- Rebuilding requires stopping the running service (file lock).
- Run-to-run thermal variance: start sweeps on a cool GPU for representative numbers.

## Files to know
- `crates/service/src/gpu_power_sweep.rs` — the sweep + knowledge model (V1) + the
  3-tier `FailTier` + `GpuKnowledge`/`BoundaryKnowledge`/`PointStat` + **V2**
  (`wilson_lower_bound`, `SweepProfile`, `select_brokkrs_v2`, unit tests).
- `crates/gpu-nvapi/src/lib.rs` — `vfcurve` mod (ClkVfPoints FFI), `apply_vf_ceiling`,
  `read_vf_curve_modern`, `vf_curve_supported`.
- `crates/service/src/gpu_apply.rs` — apply via VF ceiling (NVML cap = fallback).
- `crates/gpu-stress/src/lib.rs` — `run_render_stress` (game-power dwell).
- `crates/core/src/safe_loop.rs` — crash recovery.
- `docs/gpu-forge.md` — methodology + supported-GPU table.
