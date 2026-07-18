# Nidavellir — Decision Log

Durable technical decisions and their rationale. Newest first.

## Combine TextureRop with one CompositeGameLoad slam in Texture Hop v11 (2026-07-18)
- **Evidence:** collected v17/v18 organic silent-error rejections all failed in `texture-rop`; v19's
  remaining failures were inconclusive. Separately, the existing CompositeGameLoad is the suite's
  highest combined real-game-like draw because heavy render, texture and near-full VRAM gather share
  one submit and one core rail.
- **Decision:** contract v20 runs TextureRop first, enters CompositeGameLoad directly from a short
  idle and returns immediately to TextureRop. Roughly 71% of the dwell is dedicated to this pair;
  all prior broader coverage remains mandatory before acceptance.
- **Efficiency and safety:** use one composite segment per cycle to avoid repeating its large VRAM
  pool setup. Keep TextureStream severity-last and do not increase shader density or create a
  compute-only power virus. Standard/Long budgets and recovery behavior stay unchanged.
- **Evidence boundary:** the Texture pattern fingerprint becomes `f2q-texhop-v11-r1/v11-texture`
  and qualification contract 20 rejects pre-v20 positives. Hardware-local bad/good voltages remain
  regression controls only and are never encoded as blacklists or cross-GPU defaults.

## Replace the temporary Texture Lab with a supervised manual point (2026-07-18)
- **Evidence:** no synthetic method rejected the known-bad point, while automated Overwatch reached
  only its low-load menu. Keeping a method-comparison product surface no longer served the active
  experiment; real operator-played gameplay is the current oracle.
- **Decision:** Advanced Diagnostics accepts a clock and requested VF bin, resolves the nearest real
  bin within 8 mV and applies the normal bounded anchored F2 curve without starting any workload.
  The temporary Texture Lab IPC/UI and its unproven workload helpers are removed.
- **Safety:** the manual point is not persisted as a profile or observation, but its Safe Loop boot
  intent stays armed for the whole test. It blocks concurrent GPU writers and is reset on explicit
  operator action or graceful service shutdown. No hard voltage lock is used.

## Use manual real gameplay as the hardware oracle and make its trace self-proving (2026-07-18)
- **Evidence:** exact `1800@868` survived fixed Texture/transition/DX11/compute methods and a 172.5 W
  p99 Heaven + checksum co-load. Automated Overwatch stayed in its low-load menu, so neither elapsed
  time nor more synthetic density established the known field failure.
- **Decision:** the operator will apply `1800@869`, enable Game Trace and play the Overwatch workload
  that already reproduces the instability. Synthetic promotion remains blocked until its mechanism
  can discriminate the same point and pass the known-stable control.
- **Trace contract:** `game-trace-v2` captures the effective VF curve, high-rate telemetry timing and
  validity, honest fresh voltage reads, frequent buffer flushes and before/after TDR-event
  correlation. These additions are observation-only and cannot change GPU tuning.

## Make Texture Hop an operator-calibrated workload before versioning it again (2026-07-18)
- **Problem:** four fixed lab methods can identify a winning family but cannot determine which
  computational density or load transition actually rejects the hardware oracle. Repeated source
  bumps would confound the experiment and require a build for every calibration step.
- **Decision:** expose three numeric Texture Hop controls only in the temporary lab: dependent shader
  rounds (8–256), rendered frames per queue-drain hop (1–64), and true idle gap (0–500 ms). Presets
  are named coordinate shortcuts, never separate workload contracts.
- **Integrity:** shader rounds change deterministic output, so every unique configuration captures
  and caches a matching stock golden before apply. Parameters enter evidence provenance and the
  in-memory trial record. The fixed Forge v19 path uses none of these runtime inputs.
- **Safety:** 256 rounds is the hard compute ceiling. Configurations above the fixed 64-round Forge
  default render in 16 preemptible bands with a 500 ms per-band pre-hang cutoff, so high density does
  not become one monolithic TDR-class submission. Longer gaps are idle, not extra computation;
  duration stays capped at 55 seconds and prior Safe Loop/reset evidence boundaries remain active.

## Isolate the hardware oracle in a temporary exact-point Texture Lab (2026-07-18)
- **Evidence:** the interrupted run was explicitly learning: persistent and terminated multiple
  clocks as BlacklistedBoundary; it therefore could not measure first-run organic convergence.
- **Product decision:** Clean Run is permanently visible in every primary run selector. Full Reset
  may auto-select it, but it is no longer discoverable only while that one-shot arm is active.
- **Experiment decision:** pause full Forge iteration and compare existing aggressive qualifier
  methods at the fixed known-bad 1800@869 and 1815@875 requests. The backend resolves each request
  to the nearest real VF bin, never changes the point during a trial and caps each method at
  55 seconds. Results stay in memory and are not promoted into Forge Knowledge.
- **Safety boundary:** existing blacklist evidence is ignored only for executing this explicit lab
  point. Safe Mode, the service-wide GPU write lease, Safe Loop arm/apply/verify/reset and TDR
  recovery remain mandatory. Reset-clean failures do not pollute later Forge evaluation; device
  loss remains durable safety evidence.
- **Acceptance gate:** find a repeatable method that organically rejects both known-bad points in
  under one minute per method. Only then promote that method into the bounded Forge contract; retire
  the temporary tab after the new contract is proven on clean hardware runs.

## Make Texture Hop v10 the hardware oracle; bound Standard and retire Fast (2026-07-18)
- **Observed problem:** the clean run accepted `1800 MHz @ 856 mV`, while repeated field testing puts
  the credible boundary near `1800 MHz @ 875 mV`; `1800 MHz @ 869 mV` is therefore the immediate
  hardware regression oracle. These values are validation evidence for this GPU, never hardcoded
  tuning limits for other hardware.
- **Detector decision:** contract v19 replaces v9 with Texture Hop v10: 64 rounds with four dependent
  texture samples per round, an immediate TextureRop entry and irregular 2/3/5/7-frame burst/gap
  transitions around composite and power load. Its new semantic fingerprint invalidates older
  positive evidence for Apply.
- **Time decision:** Standard uses 30 s frontier qualification plus 2 min Texture Hop and 5 min
  Endurance for each publishable exact pair. It cancels active GPU work at 59 minutes, reserves the
  last minute for reset/checkpoint, preserves learning and releases no incomplete profile. Long is
  the only explicit mode without that ceiling and retains 60 s + 5 min + 20 min proof.
- **Product decision:** remove Fast from all current UI selectors and provisional semantics. Keep its
  old IPC method only as a mixed-version alias that executes Standard. Put the Standard/Long selector
  beside the main Forge action; the one-shot Clean Run inherits the Standard budget.
- **Acceptance gate:** software tests prove workload construction and fail-closed orchestration only.
  Hardware acceptance requires a clean Standard run to reject the known-bad `1800@869` neighborhood
  while allowing the search to recover toward the known-good `1800@875` boundary within one hour.

## Evaluate v18 organically before changing stress weights; make its power evidence honest (2026-07-17)
- **Evidence boundary:** the latest promising profiles were produced by persistent learning spanning
  prior Forge/game evidence, so they do not prove first-run convergence. Texture v9 is already
  TextureRop-first and Endurance already front-loads its adversarial rejection tier. Tightening
  weights again before one new Clean Run would confound the experiment rather than improve it.
- **Reset decision:** successful Full Reset arms the next UI start as a one-shot Clean Run, returning
  the selector to Standard after completion. Durable real-world condemnations remain append-only
  product truth, but Clean Run scopes reads to the new run so old evidence cannot steer evaluation.
- **Energy decision:** once Texture v9 proves an exact pair already exceeds the shared publication
  ceiling, skip Endurance and exclude the pair from this run's profile pool. Power-bound is not
  instability: never blacklist it and never infer that more voltage is a repair.
- **Scoring decision:** profile selection and publication use the worst sustained p99 from a complete
  Texture + Endurance gate; Endurance is not merely an off-cap peak side channel. Stock-relative
  efficiency is calculated only from this GPU's converged stock p5/p99 measurements.
- **UX decision:** the home progress surface contains only comprehensible progress/current-next/ETA
  information. Candidate evidence remains in Advanced Diagnostics. Profile cards are collapsed
  purpose-first disclosures and reveal only target MHz/mV, maximum measured power, efficiency versus
  stock, MHz/W and actions when expanded.

## Close every viable same-clock bin; v18 uses binding tests first (2026-07-16)
- **Problem:** the first vertical-repair patch stopped after two attempts, used one undifferentiated
  voltage policy for all profiles, read a stale condemnation snapshot and could persist non-Silent
  outcomes under the silent-error ledger kind. Meanwhile the 38-minute exact gate spent five minutes
  in DX11 and eight in standalone TransitionShock even though collected runs showed TextureRop and
  continuous Endurance as the actual rejectors.
- **Vertical-closure decision:** no attempt-count cap. A reset-clean physical `SilentError`,
  `Unstable` or `ClockDrop` may climb to the next viable real bin at the same clock; an inconclusive,
  coverage or orchestration result stops incomplete without blacklist or inferred repair. Reload
  condemnation state at each decision. Persist the exact-Apply silent quarantine only for a real
  `SilentError`; never encode a different outcome under that name.
- **Profile decision:** retain the common 94%-of-cap publication ceiling. Godforge may climb to the
  physical limit under it. Brokkr's voltage ceiling is one real bin below Godforge; Deep Calm is one
  bin below the lowest stronger profile. At an exhausted Godforge clock, carry that voltage to the
  next lower real clock, calibrate exact power and require the full gate; a Godforge-only selection
  override prevents the efficiency tie-break from discarding that candidate.
- **Qualification decision:** contract v18 makes Texture v9 TextureRop-first and front-loads the
  aggressive TextureRop/composite/cap-slam tier inside the still-continuous 20-minute Endurance.
  Current exact Apply requires Texture v9 5 min + Endurance 20 min. DX11/TransitionShock cease to be
  mandatory and DX11 golden capture leaves the active startup path; their persisted evidence remains
  backward-readable. This reduces a fully passing pair from 38 to 25 dwell minutes without reducing
  the continuous thermal proof.
- **Hardware gate:** compile/unit tests prove orchestration and workload composition, not silicon
  specificity. The next evidence must come from a new Clean Run with old learning excluded.

## Resynthesis must score the conservative post-gate p99 (2026-07-17)
- **Bug proven by the 2026-07-17 run**: selection scored candidates with the calm PowerRender
  calibration p99 while publication printed the conservative post-gate basis. Deep Calm was
  selected as 1740@812 at 157 W (11.1 MHz/W); the gate measured 188 W (Texture 300 s p99 =
  Endurance peak); only `max_power_w` (off-cap basis) was raised, so every resynthesis kept
  scoring 157 W and published a dominated "efficiency" profile — Godforge-class worst-case power
  (188 vs 185 W) for 135 MHz less clock.
- **Decision**: when an exact-Apply gate passes, raise the classified point's `power_p99_w` (and
  `perf_per_watt`) to `current_apply_qualification_p99_at_anchor` — the SAME basis publication
  uses — before the next resynthesis. Selection, dominance pre-gate and publication now see one
  honest number. Strictly conservative: only ever raises. Regression test:
  `resynthesis_scores_post_gate_conservative_p99_not_calm_calibration`.
- **Known consequence (accepted)**: unqualified candidates still carry the optimistic calm
  calibration, so the efficiency slots may walk down through a few optimistic candidates (one
  38-min gate each) before settling on the honest best — bounded by the 90%/95% clock floors.
  On this card Deep Calm may legitimately converge to Brokkr's point: that is measurement, not a
  bug. Follow-up that eliminates the walk (with P2/v9): measure a texture-class p99 at every
  exact Apply bin during the existing calibration gap-fill (~65 s × ~12 bins once per run), so
  every candidate carries a gate-representative selection basis before the first synthesis.

## Manual Forge pause is explicit, exact-compatible and evidence-preserving (2026-07-16)
- **Problem:** the old Stop was terminal and restart recovery could appear to continue despite not
  proving whether the program, driver or GPU still matched the saved evidence. A TDR interruption is
  materially different from an operator-requested pause.
- **Decision:** Stop is cooperative and becomes `paused` only after a confirmed stock reset and
  durable checkpoint. Resume has its own IPC method and requires exact package/build, GPU/adapter and
  driver identity. It keeps the same run ID, reuses only compatible reset-clean observations from
  that run and retries the interrupted candidate. Start always means new run; TDR, panic, Sentinel
  cancellation and Reset remain non-resumable recovery paths.
- **UX contract:** current/next task IDs and durations are structured backend data. The frontend may
  locally advance clocks between backend callbacks, but it must not infer work or safety state from
  log prose. Semantic live-log colors are therefore presentation-only.
- **Telemetry contract:** unavailable fan/voltage stays `None`; zero fan duty is real data. VRAM
  speed is primary and capacity/usage secondary. Sensor/UI changes never alter Forge evidence.

## Forge learning modes: experimental clean run vs production persistence (2026-07-16)
- **Why**: during algorithm development every version must be evaluated on a fully ORGANIC search —
  pre-classified pairs, historical seeds and prior condemnations contaminate the comparison. The
  durable ledger stays right for the product; it is wrong as an experiment input.
- **Decision**: two modes. `ForgeLearning::Persistent` (production) keeps the P0 behavior. The new
  `ForgeLearning::CleanRun` (IPC `StartPowerSweepClean`, UI mode "Clean run · Experimental") starts
  organic: `f2_observations.jsonl` + `forge_state.json` archived to `forge-archive/<run_id>/`,
  `safe_loop.json` snapshotted and its GPU V/F blacklist regions stripped (Safe Mode, crash
  counters, incidents and non-GPU entries preserved), no run-sequence/profile carryover, and the
  condemnation ledger read RUN-SCOPED (only events with the current `run_id`).
- **Key invariant**: ledger WRITES never stop — TDR/crash/gate failures during a clean run still
  append to the global ledger (production truth is never lost) and, carrying the current run_id,
  still block and steer vertical repair WITHIN the run. Reads are what the mode scopes.
- **By construction a clean run cannot emit**: "dwells redundantes pulados", "fronteira prevista
  por fronteira v4 anterior", "retomando fronteira já delimitada" (all require prior observations,
  archived away) or a `BlacklistedBoundary` from pre-run evidence (record stripped + ledger
  run-scoped). During-run failures may still produce `BlacklistedBoundary` — intended.
- Results are archived at run end but never auto-imported by the next clean run; sentinel,
  startup recovery and TDR protections remain fully active in both modes. Development validation
  runs MUST use clean run mode until the algorithm stabilizes.

## Condemnation ledger + vertical Apply repair (P0+P1, 2026-07-16)
- **Problem proven by the 2026-07-14/16 runs**: (a) a gate failure at the margin pair excluded the
  whole clock — the 07-14 run sank 1920→1740 through six ~38-min Endurance failures without ever
  trying the validated bins above each failure; (b) durable failure knowledge lived only in
  `safe_loop.json`, which a manual reset wiped on 07-15 — 1890@900 (Endurance fail 07-10) was
  re-attempted on 07-16 and PASSED a single ladder, one interruption away from publication.
- **P0 — `condemnation_ledger.jsonl`** (`crates/core/src/condemnation.rs`): append-only, per-GPU,
  survives every reset path (`clear_all_learning` explicitly excludes it; wire format pinned by
  test). Severities: **Rigid** (field TDR, CandidateCrash, device-lost, operator report — refuses
  `anchor <= floor`, manual rehabilitation only via an appended `rehabilitated` entry) and
  **Quarantine** (Texture/Endurance SilentError at exact-Apply — refuses strictly below; the exact
  pair stays attemptable but publishing needs TWO independent full-gate passes, or one pass under a
  strictly stronger contract). Descent 60 s failures stay operational (not ledgered). The floor is
  the UNION of the safe-loop field floor and the ledger (`ledger_refusal`), consulted by every
  confirmed preflight, the descent boundary check, profile restore and IPC Apply.
- **P1 — vertical repair** (`f2_plan_vertical_repair`): a gate failure condemns the BIN, not the
  clock. The same clock climbs the real VF curve (+1 bin on SilentError, +2 on TDR/device-lost,
  skipping condemned bins), admitted only under the PUBLICATION ceiling (`off_cap_ceiling_w`, 94%
  of cap — not the 98–99% discovery bound) using the worst honest measurement at the bin (confirmed
  p99 + peaks; unmeasured bins get a PowerRender calibration first). Budget:
  `MAX_APPLY_REPAIRS_PER_CLOCK = 2` per run — a budget, not a physical truth. Repaired pairs ALWAYS
  re-run the full exact-Apply gate: descent evidence orients power/order only.
- **Dominance pre-gate**: a candidate is skipped without spending the 25–40 min ladder only when an
  already gate-APPROVED point dominates it (≥ sustained clock, ≤ selection power); descent-only
  evidence never vetoes. Unknown candidate power fails open toward qualifying.
- **Deferred**: Godforge fast-drop (next clock born at the highest off-cap bin). Godforge's
  tie-break is lowest-power, so a lifted same-clock candidate always loses selection — doing this
  properly needs per-profile candidate overrides in synthesis; it must not ride along with the
  P0+P1 validation run. Texture v9 / Endurance reorder / gate removals are P2 (contract v18).

## Forge restart incidents require acknowledgement; field failures come from the local profile (2026-07-15)
- **Restart contract**: a running checkpoint surviving startup is evidence of an incomplete Forge.
  Reconcile it before Safe Loop consumes the boot flag. Attribute and blacklist only an exact armed
  candidate; otherwise record an explicitly unattributed incident and never infer a neighboring point.
- **Recovery contract**: stay at stock and block Forge Start, Apply and boot reapply until explicit
  operator acknowledgement. Resetting hardware is not acknowledgement, and the UI must not auto-start.
  The ordinary reset preserves the checkpoint and learning; full reset is the intentional clean-test
  operation that forgets them.
- **Field contract**: a user-confirmed real-use profile failure is stronger than a synthetic pass.
  Resolve the exact current profile pair at runtime, persist its local blacklist/incident evidence and
  invalidate the profile set. Never hard-code a GPU's known-bad coordinates in source.
- **Ownership**: the active Forge worker owns GPU mutation. A live sentinel event records the incident
  and requests cooperative stop; it does not race the worker with a second reset.
- **Evidence identity**: export only the ordered run sequence associated with the checkpoint and its
  incidents. Dirty builds are identified by a content hash in addition to HEAD.

## Qualification evidence is reproducible; Ctable, Cboost and Cmax are separate (2026-07-15)
- **Corrected premise**: the field trace did not isolate sustained BoostEdge/bin residence as the
  cause of the game TDR. Equivalent external telemetry survived in the lobby. Treat workload and
  driver-path composition as the missing variable; do not encode the old causal claim in policy.
- **Evidence contract**: qualification v17 stores build revision/dirty state, workload fingerprint,
  actual render backend, adapter/driver, checksum method and golden configuration. Current positive
  discovery, boundary and exact-Apply evidence requires both confirmed stock reset and boot-flag
  cleanup. Compatibility means old lines remain readable, not eligible.
- **Stock domain**: Ctable comes from static physical VF indices, Cboost is sampled only after a
  bounded thermal/p5 preheat converges, and Cmax is the first clock the Forge proves sustainable.
  Failure to converge, missing temperature, telemetry stall or stock throttle aborts before tuning.
- **Power policy**: p99 >=99% of the numeric limit is NearCap, <=98% is OffCap, and the interval is
  Ambiguous. Ambiguity must repeat within the bounded p99 budget and otherwise fails inconclusive.
  `power_capped_frac` is fallback only when no valid numeric limit exists.
- **Candidate Transaction**: the final usable discovery attempt arms/applies/verifies once, runs
  PowerDiscovery and the boundary qualifier under that same curve, then owns one reset/boot-flag
  cleanup. p99 rechecks close cleanly before a new attempt. Persist qualification before discovery
  because JSONL is not transactional; expose no positive/callback until both records and cleanup are
  proven. DeviceLost retains the boot flag, and reset/clear/blacklist-save/persistence failures dominate
  any otherwise positive dwell.
- **Qualifier**: MixedGame interleaves BoostEdge, TextureRop and PowerRender within one frame/submit;
  BoostEdge/MixedGame integrity checks run GPU-side every 16 frames with accumulated mismatches. This
  removes the every-frame full-copy/wait distortion without discarding known-answer coverage.
- **Sentinel**: a Rust thread cannot safely cancel a stuck GPU worker. The canary therefore owns its
  call synchronously; a timeout may not abandon a live context. Canary and Event Log atomically claim
  recovery ownership so one episode cannot trigger two concurrent GPU mutations. Its TextureRop
  baseline remains execution-local, so the canary detects returned stochastic/self-consistency
  failures but is not represented as a stock-known-answer or game-correctness oracle.
- **DX11 exact-Apply gate**: after Texture, run a native offscreen Direct3D 11 render for 5 min before
  TransitionShock and Endurance. Capture a deterministic stock checksum first, select an NVIDIA DXGI
  adapter explicitly, require the same adapter LUID at the candidate, poll GPU completion with a
  750 ms bound, and treat missing/ambiguous coverage as inconclusive. This expands driver/API coverage
  without changing descent or shortening any existing gate. `1845@862` remains local field evidence,
  never a source-coded blacklist coordinate.

## v13: absolute NVML max-clock ceiling for every F2 dwell AND Apply (2026-07-06)
- **Problem**: the anchored plateau caps are per-point offsets relative to the base V/F curve, which
  the driver shifts with temperature — the full 2026-07-06 run measured p5/p95 = label **+15/+30 MHz**
  on every pair. The delivered regime was ambient-dependent, the calm profile "1770@856" effectively
  ran ~1800@~856 when cool (operator ground truth: 1800@868 is game-unstable, 1800@875 is the
  hand-validated point), and the operator's 1800@875 was displaced into an unselected regime rung by
  the v12 relabeling.
- **Decision**: set `nvml_gpu::lock_core_clock_max_mhz(target)` (min=210, max=target — the
  `nvidia-smi -lgc` equivalent, absolute in MHz, immune to the thermal shift) after every verified
  anchored VF write: in the dwell motor (`RealF2Ops::apply_positive_offset`) and in the one-shot
  Apply (`apply_anchored_undervolt`). Fail-closed: ceiling failure fails the apply; every reset path
  already releases the lock (`reset_to_stock`, `gpu_apply::reset`). Measured point == labeled point
  == applied point. Classifier: sustained p95 > target + 15 ⇒ Inconclusive (ceiling didn't hold —
  never Stable, never boundary knowledge). The v12 regime lift is REMOVED; the strict p95
  reconciliation stays as a dormant fail-closed net. Contracts bumped: discovery 4→5,
  qualification 11→12 (all shifted-regime evidence quarantined; full re-forge required).
- **Distinct from the rejected pin**: the earlier "no NVML clock pin" decision rejected the RIGID
  pin (min=max) / voltage lock, which removes power management and TDRs under a power cap. The
  max-only ceiling keeps full downward elasticity and stays adopted.
- Plan/gates: `docs/clock-lock-v13-plan.md`.

## Thermal slowdown disqualifies an exact-Apply dwell only if it dropped the clock (2026-07-04)
- **Problem**: a full F2 run finished with ZERO applicable profiles. The power-bound top point
  (1935 MHz @ 956 mV, pinned at the 200 W cap) returned `ExactApplyInconclusive`: three HighFPS
  dwells each set NVML `thermal_throttled` (a memory-junction hotspot at only ~67-69 C core) while the
  card actually ran avg 1953-1957 MHz, p5 >= 1935, no silent error, coverage 8/8. The old guardrail
  treated any thermal-slowdown flag during exact-Apply as fatal, discarding evidence where the card
  demonstrably HELD the qualified point.
- **Decision**: a thermal-slowdown flag invalidates exact-Apply *stability* evidence only when the
  slowdown actually backed the card OFF the point — i.e. sustained clock (p5) sagged below target
  beyond the existing 30 MHz `F2_CLOCK_DROP_TOL_MHZ`. When the card held >= target despite the flag,
  the hard VF point was exercised, so the dwell/observation is trusted. Implemented in two layers that
  must agree: the dwell classifier (`classify_f2_stress_dwell`, `ApplyQualification` arm) and the two
  Apply-qualification publish gates (`apply_qualification_p99_at_anchor` /
  `current_apply_qualification_p95_clock_at_anchor` via `apply_qual_reading_trustworthy`). Fails closed
  when the sustained clock is unknown.
- **PowerDiscovery stays strict**: power calibration (`PowerDiscovery` classifier arm,
  `f2_power_measurement_usable`, `current_discovery_observation_at_anchor`) keeps the unconditional
  `!thermal_throttled` rule — a throttled sample understates the V<->W map, a different and real
  corruption. Only Apply-qualification stability is relaxed.
- **Safety (audited SAFE, twice)**: publish aggregation is max-only, so admitting a held-throttled
  reading can only RAISE published p99 wattage (never understate → profile never presented cooler than
  reality) and RAISE p95, which makes regime reconciliation demand *more* voltage, not less. Triad
  completeness, reset-clean and boot-flag gates are untouched. No new path lets an under-qualified
  profile reach apply/persist.
- **Compatibility**: no discovery contract, Safe Loop, reset-to-stock or IPC change. New core const
  `F2_APPLY_CLOCK_HOLD_TOL_MHZ = 30` mirrors service `F2_CLOCK_DROP_TOL_MHZ = 30` (hand-synced, doc
  cross-referenced). Regression-covered in `f2_observation.rs`; core 78/0, service 355/0.
- **Status**: code + tests complete, NOT hardware-tested. A controlled rerun is expected to validate
  HighFPS on the first dwell, advance HighFPS->Texture->Transitions, and publish 1935 @ 956 mV even
  when the hotspot recurs.

## Forge ETA separates best remaining time from a conservative total ceiling (2026-07-03)
- **Decision:** keep `estimated_remaining_ms` as the live best remaining estimate and publish
  `estimated_total_upper_ms` as a separate absolute wall-time ceiling. The UI must not label the best
  estimate as a deadline or recreate backend timing constants.
- **Refinement:** before Cmax, publish no upper total because the inclusive 90% physical domain is not
  trustworthy yet. Once the first sustainable Cmax is known, publish that exact domain and recompute
  after each target, calibration gap and selected exact-Apply pair.
- **Conservative work model:** a frontier candidate may consume PowerRender plus every current v7
  boundary pattern; each missing exact Apply bin reserves up to three p99 attempts; Standard/Long
  reserve up to three unique exact-Apply pairs with three five-minute patterns each.
- **Compatibility:** all new progress fields are optional/defaulted. Legacy and interrupted payloads
  show a refining estimate rather than a fabricated maximum. Terminal progress stores actual elapsed
  time as the final upper total.
- **Trade-off:** the ceiling can begin deliberately high and tighten sharply after Cmax/synthesis.
  Retries or newly exposed p95 support may still raise it; it is a transparent operating estimate,
  not a hard deadline.

## F2 profile identity follows sustained p5 electrical regime (2026-07-02)
- **Decision**: tolerate one 15 MHz physical bin of target/p5 variance. Beyond that, p5 owns the
  electrical regime. The candidate must carry at least the maximum measured Apply anchor through the
  nearest target at/above p5; otherwise it is removed before synthesis.
- **Profile behavior**: removing an under-anchored alias makes Godforge resolve to the canonical
  higher-regime point and makes Brokkr's/Deep Calm fall to the next efficient self-consistent target.
  Power is never interpolated and no synthetic point is created.
- **Evidence inheritance**: Standard/Long require current A+B boundary evidence for the candidate and
  the supporting p5 regime. A rejected or inconclusive exact Apply blocks lower-anchor aliases of that
  same regime before re-synthesis.
- **Published power**: after exact-Apply qualification, each selected profile publishes the larger of
  its confirmed PowerRender p99 and the p99 observed across its approved FSGL3 A+B pair. Selection
  remains based on homogeneous PowerRender frontier evidence; the card cannot understate a larger
  sustained peak already measured by its deployability soak. Restored v6 snapshots refresh from the
  append-only observations.
- **Contract**: qualification v6 invalidates v5 profile selections. Target, p5, p95 and p99 remain
  separate facts; the ten-minute exact-Apply soak runs only after regime reconciliation.

## F2 deployability requires long qualification at the exact Apply pair (2026-07-02)
- **Decision**: boundary qualification and post-margin deployability are separate claims. After
  synthesis, every unique selected `(target, Apply VF bin)` runs FSGL3 A+B for five minutes per
  pattern. This introduced qualification v5; v6 retains the soak and adds regime reconciliation.
- **Rationale**: adding voltage can move GPU Boost into a higher sustained-clock regime. A boundary
  pass at a lower bin therefore cannot be inherited by the `+12 mV` Apply point.
- **Inconclusive debt**: after any inconclusive attempt, that pattern needs two consecutive clean
  passes. One retry can no longer erase the weak-coverage signal.
- **Failure semantics**: reset-clean rejection excludes only the exact candidate and re-synthesizes
  from measured alternatives. It does not become a monotone frontier failure. Hard device, reset,
  write and verification failures still abort under Safe Loop.
- **Identity contract**: retain configured target, measured average, sustained p5 and sustained p95
  separately. Profile watts remain exact-bin sustained p99. F2 Apply requires the additive,
  versioned exact-Apply qualification seal, so restored v4 points fail closed.

## F2 uses prediction plus asymmetric adaptive search (2026-07-01)
- **Decision**: use compatible same-GPU discovery-v4 history first, then the non-increasing isotonic
  trend of the last 3–4 qualified clocks, to suggest a boundary. Start one physical bin above it.
  Predictions are scheduling hints only and never become observations, qualification or profile data.
- **Contradiction guard**: if historical, previous-clock and trend suggestions span more than 25 mV,
  keep the established sequential warm start. A compatible historical offset may serve only as the
  cross-run baseline for the writer's existing +15 MHz progression gate.
- **Power-bound stride**: while confirmed p99 is at 99%+ of cap, use the p5 deficit to request 4 bins
  at >=90 MHz, 2 bins at 45–89 MHz, otherwise 1. The selected jump must also stay within 25 mV and
  the positive-offset step cap; otherwise it shrinks automatically.
- **Asymmetric recovery**: a reset-clean off-cap failure reached by a jump creates a shallower-safe /
  deeper-failed bracket. Midpoints are tested only above the known failure. Once a point is approved,
  descent becomes adjacent and the known failed bin terminates the boundary without another dwell.
- **Unchanged**: p99 consensus and Apply-bin backfill, thermal invalidation, FSGL3/goldens, Safe Loop,
  Leva 1 qualification/margin behavior and +12 mV Apply remain the same.

## F2 frontier and profiles use apply-bin sustained p99 power (2026-07-01)
- **Decision**: keep the existing textured `PowerRender`. Do not replace it with compute-only
  `POWER_SHADER`, which previously underloaded board power relative to the render/game regime.
- **Measurement contract**: preserve `power_w` as steady-state mean, `power_p99_w` as sustained
  high-percentile power and `max_power_w` as the raw highest post-ramp sample. p99 uses nearest-rank
  over all retained samples; `n < 100` explicitly falls back to raw max and empty input remains absent.
  Discovery contract v4 excludes v3 positives/power-bound resume evidence. An anomalous adjacent-bin
  p99 in the same p5 regime repeats the exact bin up to three total attempts; two must agree and the
  highest measured p99 is retained. No consensus is ineligible, with no interpolation.
- **Profile contract (superseded for stability by the 2026-07-02 decision)**: apply the unchanged +12 mV margin first, then calibrate power and sustained p5
  from the exact physical apply bin. If warm-start pruning skipped that exact target/bin pair, run a
  supervised discovery-only PowerRender backfill there and apply the same v4 p99 consensus. Do not
  rerun FSGL3 under the original v4 assumption that the higher-voltage Apply bin was safer. F2
  selection uses p99 for R and MHz/W.
- **Boundary contract**: `ClockDrop` at 99%+ of the numeric cap by p99 becomes
  `PowerBoundClockDrop` and continues descent even after a prior sustained point; off-cap clock drop
  remains a boundary. A `Validated` point still at cap also continues descent and cannot launch FSGL3.
  Power-bound observations are calibration telemetry, not qualification evidence.
- **Thermal contract**: software/hardware thermal slowdown invalidates the discovery measurement as
  `Inconclusive`; it never marks the voltage unstable.
- **UI contract**: cards say “sustained p99” and explicitly state it is not a hard power limit.
  Legacy payloads fall back to raw peak/mean; raw peak remains available as a separate diagnostic.
- **Historical scope**: FSGL3 A+B goldens, Leva 1 recovery, early-stop and +12 mV application policy
  were unchanged here. Later qualification contracts superseded v4 deployability.

## F2 uses a margin boundary and treats supervised TDR as learning (2026-06-30)
- **Decision**: qualification compares like-for-like FSGL3 heavy-phase p5 telemetry, separated by A/B
  pattern. A relative fall beyond 30 MHz after two stable references is a reset-clean `ClockDrop`,
  not a crash. Aggregate mixed-workload p5 remains excluded.
- **Ambiguity**: `Inconclusive` retries the same point twice with a 1.5× dwell and then skips only the
  current clock. The Forge continues, but cannot emit `finished` or unlock Apply without qualified
  profiles. Hard recovery-integrity failures still abort.
- **Recovery accounting**: exact `f2_undervolt_probe` TDR/Unknown boot flags blacklist and recede
  without consuming the normal-use Safe Mode threshold. Unrelated bugchecks and all non-Forge phases
  keep the existing threshold. Startup recovery is the single crash-accounting authority.
- **Resume policy**: the initial Forge action is consent to resume the same interrupted run. Resume is
  attempted once when the UI reconnects, not as an unconditional background stress run at service
  boot. Manual Stop is not resumable.
- **Apply policy**: the learned boundary and applied point are distinct. Apply requests +12 mV and
  snaps upward to an exact valid VF bin; both boundary and effective margin are visible in additive
  IPC fields.
- **Pre-hang gate**: record the 300 ms missing-valid-NVML-sample signal, but do not reset hardware from
  the sampler thread. Active mitigation requires calibrated evidence and cooperative cancellation.
- **Status**: code-complete, no hardware run. Leva 2 dispersion/bisection stays gated.

## FSGL3 uses stock goldens and verifies every rendered frame (2026-06-30)
- **Problem**: FSGL2 sampled framebuffer checksums about every 250 ms and compared against the first
  frame of the same segment. Known low-average-power TextureRop/MixedGame instability could therefore
  escape qualification and later TDR in a game.
- **Decision**: keep `PowerRender` discovery unchanged and make FSGL3 A+B the Standard/Long
  interleaved qualifier. Capture power, boost and texture/ROP REDUCE3 goldens at stock with three
  fresh `GpuCtx` instances, then compare every rendered frame on-GPU. Goldens are per-run and never
  persisted; non-deterministic stock capture aborts Forge.
- **Transient contract**: FSGL3 emphasizes TextureRop/MixedGame and repeats six heavy frames followed
  by a 4 ms gap. This intentionally probes the first post-gap frame while preserving the existing
  eight telemetry phases and one-heavy-render-per-context invariant.
- **Compatibility**: FSGL1/FSGL2 patterns, REDUCE self-reference cadence, discovery, compute KATs and
  IPC payloads are unchanged. Historical evidence remains readable.
- **Apply gate**: `F2_QUALIFICATION_CONTRACT_VERSION = 4`. Only current FSGL3 A+B `Pass` evidence
  counts toward deployable profiles; older strengths/contracts remain provisional.
- **Trade-off**: per-frame copy+REDUCE3 adds GPU queue pressure. Polling remains bounded every three
  frames; if the supervised hardware gate exposes TDR pressure, the next mitigation is a small verify
  ring rather than weakening coverage.
- **Status**: code-complete; workspace build/check/tests, UI build, clippy and diff checks pass.
  Hardware was not run. Before the first trial, clear persisted Forge state and verify rejection of
  1920 MHz @ 912 mV and 1935 MHz @ 918 mV.

## FSGL2 becomes the default Standard/Long qualifier (2026-06-30)
- **Problem**: a boundary could pass PowerRender and two FSGL1 passes but still TDR in a game. FSGL1 was
  useful as a discovery/fronteira filter, but it was still too close to a soak pattern and not enough as
  a deployable stability contract.
- **Decision**: keep PowerRender as measurement/characterization only, pause FSGL1 in the Standard/Long
  runtime path, and use FSGL2 as the default interleaved per-bin qualifier: pattern A 60 s + pattern B
  60 s. If either FSGL2 pass fails, the candidate bin is recorded as bad and the algorithm keeps the
  previous FSGL2-qualified physical bin as the boundary.
- **Workload contract**: FSGL2 uses different deterministic profile order/emphasis (BoostEdge,
  HeavySpike, TextureRop, ComputeBurst, IdlePulse, MixedGame) and records per-phase metrics:
  phase/pattern/pass, duration, frames, checksums, compute checks, clock percentiles/residency,
  power/cap fraction, temperature, failure phase and coverage status. BoostEdge power-cap masking or
  weak phase contrast makes the result `Inconclusive`, not `Pass`.
- **Apply gate**: `F2_QUALIFICATION_CONTRACT_VERSION = 3`. Apply counts only current-contract FSGL2
  `Pass` evidence and requires distinct patterns A+B. FSGL1-qualified, discovery-only, legacy and old
  contract evidence may guide discovery but cannot unlock Apply.
- **Future gate**: if FSGL2 proves more reliable as the default, a later FSGL3 can be designed as the
  final doubt-breaker instead of reintroducing FSGL1 in front of FSGL2.
- **Status**: code-complete, no hardware run in this session. No-hardware validation passed with
  targeted FSGL2 tests, workspace Rust tests/checks, UI build, `clippy` and diff whitespace checks.
  Supervised stock/conservative FSGL2 validation is still required before Forge-long testing.

## Cmax descent interleaves qualification per VF bin (2026-06-29)
- **Problem**: per clock, discovery descended `PowerRender` PAST the first sustained point to the
  deepest PowerRender-survivable bin, THEN qualified that deepest bin with the failure-seeking
  `FailureSeekingGameLoop`. PowerRender tolerates more than the failure-seeking qualifier (and than real
  games), so the deepest PowerRender point is often too aggressive — qualifying it risks a TDR, and the
  descent below the bin that ultimately qualifies is wasted. The old code then climbed back UP one bin
  at a time on qualification failure (`f2_discovery_decision` returned `ContinueVoltage` on every
  post-sustain `Validated`; qualification ran on `deepest_good_candidate` with upward back-off).
- **Decision (operator-approved, "completa, sem teto")**: interleave the two workloads. For Standard/Long
  the per-clock loop now stops the `PowerRender` descent at the FIRST sustained (under-cap `Validated`)
  bin, qualifies THAT bin with the full N passes, and only then descends one real VF bin lower —
  `PowerRender` there measures its power and gates whether to even attempt qualification. Each deeper bin
  is fully qualified before going lower; the first qualification failure stops the descent and keeps the
  last qualified bin as the boundary. The heavy qualifier therefore never runs more than ONE bin below an
  already-proven point, so an over-aggressive synthetic discovery point can no longer TDR during
  qualification. Fast (no qualification) keeps the old descend-to-PowerRender-floor behavior (provisional).
- **Why no upward back-off anymore**: descending top-down and qualifying each bin means the bin one step
  up is already qualified when a deeper bin fails — there is nothing to climb back to. The only
  not-qualifiable case is the first sustained bin failing (the most stable under-cap point), which is a
  genuine fail-closed for that clock.
- **Downstream is unchanged and already correct**: a failed qualification records an `is_bad()`
  observation at that bin, so `first_bad_for_target` bounds the frontier and `last_discovery_good_for_target`
  selects the deepest QUALIFIED bin automatically. Locked by a new core test
  (`interleaved_qualification_failure_selects_shallower_qualified_point`). `synthesize_forge_profiles`,
  `learned_frontier`, Cmax/90%-floor, Safe Loop arm/verify/reset per dwell, and resume/warm-start are
  untouched.
- **Trade-off (accepted)**: more qualification dwells per clock (N passes × each qualified bin) → longer
  Standard/Long. The initial ETA estimate under-counts and self-corrects upward as deeper bins qualify.
  Justified by Stability/Safety over Performance.
- **Status**: code-complete; `nidavellir-core` 69 + `nidavellir-service` 319 tests pass; clippy no new
  warnings. NOT yet re-run on hardware with the new flow — a supervised Standard run is the recommended
  next check. The change is localized to `run_confirmed_f2_clock_discovery` (+ new helper
  `qualify_anchored_candidate`) in `crates/service/src/gpu_undervolt.rs`.

## Reset releases the Safe Mode latch; clean restart ≠ crash; deep reset (2026-06-29)
- **Problem**: once Safe Mode tripped, it could never be cleared. `safe_mode`/`consecutive_crashes`
  live in `safe_loop.json`, but `gpu_apply::reset` (the only caller is `ResetGpuTuning`) only reset
  hardware + boot-flag + applied profile and **never rewrote the record**. `safe_mode` was a one-way
  latch — nothing in the codebase set it `false` (not even `mark_validated`). The UI's Needs Attention
  branch then hid the forge controls, leaving "no option," and the latch survived reboots. Worse, every
  clean reboot while latched re-ran `EnterSafeMode` and **incremented `consecutive_crashes`**, and a
  manual PC restart during an armed boot-flag was indistinguishable from a crash — both inflated the
  streak.
- **Decision (Fix A)**: `gpu_apply::reset` now also clears the recovery latch via
  `SafeLoopRecord::clear_recovery_latch()` (safe_mode→false, consecutive_crashes→0, state→idle) while
  PRESERVING learning (blacklist, last_validated, crash_log) and the F2 observation frontier. So the
  existing "Reset all" button finally releases Safe Mode. No UI change required.
- **Decision (Fix B)**: re-entering Safe Mode on a *clean* boot is a new `RecoveryAction::RemainSafeMode`
  that stays hands-off **without** incrementing the crash streak. `EnterSafeMode` (which increments) is
  now reached only via the armed-flag threshold trip — an actual crash.
- **Decision (Fix C)**: the service writes a one-shot `clean_shutdown.txt` marker on graceful
  Stop/Shutdown. Startup recovery consumes it once; an armed boot-flag + marker is treated as a clean
  interruption (disarm, no crash counted). A real crash leaves no marker, so the parachute still fires.
  Fail-closed: a missing marker counts as a crash.
- **Decision (deep reset)**: new additive IPC `ResetGpuTuningFull` does everything `ResetGpuTuning` does
  AND wipes all learning (Safe Loop record → default incl. blacklist, `f2_observations.jsonl`,
  `gpu_knowledge.json`) for users who want a true clean slate. Normal Reset all preserves learning; the
  deep reset is the rare destructive option (UI control requested from Codex; see contract 2026-06-29).
- **Status**: code-complete; `nidavellir-core` 68 + `nidavellir-service` 319 tests pass; clippy no new
  warnings. NOT hardware-tested — a supervised check of the stuck-Safe-Mode → Reset all path is the
  recommended next step. An independent `nidavellir-safety-auditor` pass on Fix C is recommended before
  commit.

## F2 qualification freshness and recovery reset (2026-06-29)
- **Decision**: Standard/Long may reuse prior observations as hints, but must not qualify a boundary
  that exists only as old `prior_good` evidence. Qualification now requires the current run to
  rediscover a candidate with `PowerRender` before the FailureSeekingGameLoop can validate it.
- **Rationale**: a stronger/newer qualifier exposed that old aggressive points could TDR during
  validation. Those points should be discarded by discovery first, not promoted directly into the
  qualification workload.
- **Recovery**: `ResetGpuTuning` is an emergency recovery path, not a normal competing tuning run. It
  no longer waits for the service-wide start/apply lease; it best-effort stops marked-running work,
  resets to stock, clears the Safe Loop flag, removes the visible `forge_state.json` checkpoint and
  returns the F2 Forge handle to idle after a confirmed reset. It intentionally does not erase the
  automatic F2 observation history.
- **Worker robustness**: the live F2 Forge worker catches panic/unwind and marks progress idle with
  Apply locked, so a TDR/interrupted stress path cannot leave `running=true` forever.

## F2 discovery and qualification use orthogonal, versioned workloads (2026-06-29)
- **Decision**: keep the existing steady, eight-instance textured `PowerRender` as the exclusive F2
  discovery workload. Cmax, near-power-limit behavior, p5 and `ClockDrop` remain based on one
  homogeneous load, preserving the meaning of the learned frontier.
- **Qualification**: Standard/Long reset/reapply passes use a deterministic
  `FailureSeekingGameLoop` with PowerOpening, BoostEdge, HeavySpike, TextureRop, ComputeBurst,
  IdlePulse, MixedGame and PowerClosing. The loop crosses render, ROP/texture, compute and idle→spike
  transitions; every phase contributes checksum/coverage evidence.
- **Evidence versioning**: `f2_observations.jsonl` distinguishes discovery vs qualification evidence.
  Current Apply qualification counts only current-contract qualification passes; legacy/discovery
  positives can seed search but cannot unlock Apply. `Pass`, `Fail` and `Inconclusive` coverage are
  explicit; inconclusive coverage does not become a bad-boundary veto.
- **Classification/backoff**: crash, device loss, checksum divergence and unstable results remain
  fail-closed. Aggregate p5 from the mixed qualifier cannot produce `ClockDrop`; discovery still owns
  the 30 MHz p5 boundary. A rejected qualification automatically backs off one physical VF bin upward,
  runs fresh `PowerRender` discovery there, then restarts all qualification passes.
- **Compatibility**: Fast remains discovery-only/provisional; pass counts and durations are unchanged;
  no IPC or frontend field changes. No manual bad-point registry or operator prior is encoded.

## F2 qualification contract — Fast provisional, 90% domain, bounded warm-start fallback (2026-06-28)
- **Decision**: Fast remains full-frontier discovery but is preview-only. Standard adds two independent
  60 s reset/reapply passes per discovered boundary; Long adds three 120 s passes. A failed pass moves
  one physical VF bin upward and restarts the full qualification count.
- **Apply gate**: additive `profiles_qualified` defaults false. Both UI and service block F2 Apply until
  all selected profiles meet the confidence and confirmation requirements. Starting any new hardware
  run clears qualification because new evidence may invalidate the prior boundary.
- **Deep Calm domain**: extend the measured frontier from 95% to 90% Cmax, matching its policy floor.
- **Cross-clock reuse**: start the next clock one physical bin above the previous minimum stable anchor;
  retain the last power-bound ClockDrop as fallback. A rejected/no-candidate warm-start retries the same
  target from the fallback instead of declaring that clock unsustainable.
- **Time model**: discovery stays 10 s per candidate. Fresh-GPU estimates are Fast 20–30 min, Standard
  55–75 min, Long 90–120 min; learned-state resume can shorten them.

## F2 partial learning is durable; instability is not a crash (2026-06-28)
- **Decision**: append every dwell observation before publishing its progress event and checkpoint
  partial `PowerSweepProgress`; a service interruption restores an inspectable `interrupted` run.
- **Decision**: `SilentError`/`Unstable` ends the current voltage descent and blacklists that point,
  but does not increment `consecutive_crashes` after a confirmed reset. Only DeviceLost/TDR counts as
  a crash and retains recovery state.
- **Decision**: seed the next lower clock from the previous target's last power-bound ClockDrop
  (falling back to its first sustainable voltage). This keeps one conservative overlap while skipping
  redundant high-voltage dwells.
- **Tradeoff**: total steps and ETA are estimates because Cmax and pruning become exact during the
  run; the UI labels them accordingly and never parses technical log text for state.

## F2 integrated Cmax/frontier algorithm — physical-bin sweep, no arbitrary step cap; modes change evidence only (2026-06-28)
- **Decision**: preserve the intended integrated search instead of adding a separate preliminary Cmax
  pass. Start at the highest real clock, descend that clock's real voltage anchors while it remains
  power-bound, and reuse those dwells as discovery evidence. If no voltage sustains once power falls
  below 99% of cap, move to the next clock. The first sustained target is Cmax.
- **Per-clock terminal rule**: before sustain, a power-bound clock drop is search progress; an off-cap
  drop, silent error, or instability ends that clock. After any validated point, the first
  `SilentError`, `Unstable`, or `ClockDrop` is the lower-voltage boundary. `DeviceLost`, reset failure,
  arm/apply/verify failure, or untrustworthy recovery aborts the whole Forge. TDR is not sought.
- **No step budget**: autonomous discovery plans the full physical VF domain (`usize::MAX` is only a
  no-budget planner sentinel). The real curve/floor and first terminal detection bound the run.
  Explicit `--steps N` is still available as a manual operator boundary, not an algorithmic cap.
- **Outer frontier**: characterize every real clock from Cmax through the last bin at or above 90% of
  Cmax. No definitive synthesis/persistence occurs unless the whole range completes.
- **Mode contract**: Fast, Standard, and Long never change which clocks/voltages are eligible. They
  change only evidence depth: Fast 10 s discovery/provisional; Standard 10 s discovery + 2×60 s
  qualification; Long 10 s discovery + 3×120 s qualification.
- **Persistence/identity**: append each candidate immediately; scope learning and forge state to the
  physical GPU UUID; resume below prior good/power-bound observations or reuse a complete bracket.
- **Safety architecture**: one in-process service-wide GPU lease across starts/applies/reset; checked
  modern-VF reset; Safe Loop flag armed before writes and retained for `DeviceLost`/reset failure;
  partial runs cannot replace known-good profiles.
- **Alternative rejected**: a separate max-clock-only pass would duplicate hazardous dwells and throw
  away useful undervolt evidence. A fixed 3-step guard was also rejected because it prevented reaching
  the actual boundary; fail-closed detection and the physical VF floor are the correct bounds.
- **Hardware status**: code/tests only. The next step is a supervised real Forge run, separately
  authorized because it can trigger TDR/reboot.

## F2 PHASE 2 — Apply path wired to F2 anchored undervolt; F1 KEPT (not removed); code-complete, not HW-tested (2026-06-27)
- **Decision**: implement Phase 2 of the F2 pivot — make `ApplyPowerGodforge/Brokkrs/DeepCalm` actually
  APPLY the F2 anchored undervolt. Before this, the forge produced F2 profiles but the apply path was still
  attached to F1 flatten-down (`apply_core`→`apply_vf_ceiling`, the WRONG op for an undervolt point), so
  Phase 1 had GATED apply behind a refusal. Phase 2 removes the refusal and routes apply to the F2 writer.
- **Scope call (operator)**: F2 is the main algorithm, but **F1 is NOT removed in this pass** — it stays the
  live apply path for legacy `is_undervolt==false` payloads, and removing it would be a large, separately-
  reviewable change. Advisory recorded: `synthesize_forge_profiles` + `ForgePolicy` are SHARED by F2 and must
  stay; the now-dead F1 apply/forge code is retained for a future Phase 3 cleanup. This keeps the safety-
  critical apply change small and reviewable (the project's surgical-change rule).
- **Design (reuse the proven F2 motor, do not duplicate)**: a one-shot apply
  `gpu_undervolt::apply_anchored_undervolt(target_mhz, anchor_mv)` reuses the SAME primitives the confirmed
  `RealF2Ops` motor uses (read live VF base → `select_anchor_bin` → `apply_bounded_anchored_positive_offset`
  with prev_offset=0 → `verify_anchored_positive_offset`). FAIL-CLOSED: only `AnchoredRaiseVerified` leaves the
  curve applied; any missing anchor / writer rejection / non-verified verdict resets to stock, confirms every
  touched bin reads ~0, and returns Err with nothing applied. `apply_vf_ceiling*` / F1 / the verifier are
  untouched.
- **Persistence + reapply-on-boot**: `AppliedProfile` gains `undervolt: Option<UndervoltApply{target_mhz,
  anchor_mv}>` (`#[serde(default)]` → legacy `gpu_applied.json` loads as `None`). `apply_and_persist_undervolt`
  mirrors the F1 `apply_and_persist` lifecycle (arm Safe Loop boot-flag → write → persist → clear flag after
  the 8 s survival window; a crash leaves it armed → not re-applied). `reapply_on_boot` branches on the
  descriptor: F2 RE-DERIVES the anchored curve from the LIVE VF table each boot (never replays a stale
  absolute curve); F1 path byte-identical. Why store target+anchor (not the raw plan): the live curve can
  shift between sessions, so the deterministic axes are re-resolved against the current table. Reapply
  requires the exact validated anchor voltage; if that bin is absent it fails closed rather than silently
  selecting a lower-voltage (deeper) anchor.
- **Apply axes**: resolved from the forge point via `undervolt_apply_params` — `target_clock_mhz ?? clock_mhz`,
  `vf_table_voltage_mv ?? voltage_mv` (deterministic forge fields preferred; measured fallback for legacy
  points). The router `apply_forge_profile` keys on the STRUCTURED `prog.is_undervolt` (never text).
- **Safety invariants preserved**: fail-closed write+verify+reset; boot-flag armed before the write; conservative
  caps + clock-ceiling = stock boost top (never overclock, never CLI-widenable); NO auto-apply (apply stays the
  explicit user step); reversible via the existing GPU reset (already zeros the modern VF curve). F1 untouched.
- **Validation (no hardware)**: cargo check clean; tests core 61 / nvapi 38 / service 300; clippy zero new
  warnings. NOT run: any apply / VF write / `--confirm` / hardware.
- **Next**: independent `nidavellir-safety-auditor` pass on the diff, then ONE supervised manual apply on the
  rig (forge F2 → Apply → confirm verify+persist+reapply-on-boot; reset clears). Phase 3 =
  retire/repurpose the dead F1 code + fold Fast/Long modes into F2 depth.

## FORGE PIVOTS TO F2 UNDERVOLT — F1 flatten-down cannot differentiate a power-bound card; F2 can (proven −43 W) (2026-06-27)
- **HW finding (2 supervised runs, RTX 3060 Ti)**: the live button's F1 multi-clock forge (Option A +
  knee-seeking) COLLAPSES on this card. The card is hard-pinned at its **200 W power limit**; F1 flatten-down
  only caps FREQUENCY at/above a voltage ceiling, but the card runs at ~990 mV — below every ceiling placed —
  so lowering the ceiling 1150→1031 mV changed NOTHING: every frontier point stayed `pcf=1.000`, ~200 W,
  ~1790–1808 MHz. Phase-B knee-seeking descended 17 probes to 1031 mV and hit a `LiveMismatch` →
  `SoftUnverified` just shy of the knee. **Not a budget bug** — physics: lowering a frequency ceiling cannot
  lower power when the card is already at its power limit. Clean exit: no TDR/crash, GPU reset to stock,
  nothing persisted.
- **Decision (operator call)**: the forge's PRIMARY method becomes **F2 anchored undervolt**, not F1
  flatten-down. F2 holds the clock at a LOWER VOLTAGE and drops power directly — proven on THIS exact GPU:
  **1800 MHz @ 875 mV = 157 W vs the 200 W power-bound point (−43 W, same clock)** — a real Godforge/Brokkr's/
  Deep Calm spread. F1 stays valid only for cards where it CAN differentiate (not power-bound).
- **Not a rebuild — F2 building blocks exist + HW-proven (reuse, don't reinvent)**:
  - motor: `run_confirmed_f2_multi_step` (gpu_undervolt.rs:1362) — anchored write→verify→dwell→reset→clear
  - ladder: `run_anchored_ladder_sweep` (gpu_undervolt.rs:2657) — descend voltage per clock for min-stable
  - synthesis bridge: `learned_frontier` → `frontier_to_points`/`to_power_sweep_point` (f2_observation.rs:371/394)
    → the SAME `synthesize_forge_profiles` (gpu_power_sweep.rs:1284) the button already uses
  - F2 writer (Phase 2 apply): `apply_bounded_anchored_positive_offset` (gpu-nvapi)
- **The real GAP (verified)**: F2 is wired ONLY to the CLI `undervolt-probe` — the F2 writer is called from a
  SINGLE site (gpu_undervolt.rs:1792, the CLI motor). The BUTTON runs F1 (`run_power_sweep`→build_frontier), and
  the Apply IPC (`ApplyPowerGodforge/Brokkrs` → `apply_power_profile` → `apply_core`) writes **F1**
  `apply_vf_ceiling` (gpu_apply.rs:99). So F2 was finalized as a CLI CAPABILITY but NEVER wired into the
  button or the apply path. The missing work is WIRING, not rebuilding.
- **Big plus — F2 brings the cross-run LEARNING/MEMORY F1 lacked**: the F2 observation store
  (`f2_observations.jsonl`) records every sweep candidate across runs; `learned_frontier` (last_good/first_bad/
  bracket per clock) accumulates; the descent resumes from the deepest prior VALIDATED point; confidence grows
  with `validations_at_best` (and the `--validation-passes` opt-in / future IDLE). This RESOLVES the IDLE-learning
  gap flagged in Option A (F1 was per-run telemetry only).
- **Phased plan (each HW-validated before the next)**:
  - **Phase 1 (DONE + pushed via `e4bd006`)** — `measure_multiclock_undervolt_forge`: reuse the F2 ladder/motor over a few
    hardware-relative candidate clocks → records observations → `learned_frontier` → `frontier_to_points` →
    `synthesize_forge_profiles` → 3 differentiated profiles → validate each pick → persist `forge_state.json`.
    Apply stays GATED/refused for F2 profiles (today's apply does F1 flatten-down — wrong for undervolt). Safe,
    self-contained: the UI shows real differentiated profiles as **Discovered** and disables Apply whenever
    `PowerSweepProgress.is_undervolt == true`; legacy F1 Apply remains unchanged.
  - **Phase 2** — F2 apply path: wire `apply_bounded_anchored_positive_offset` into the Apply IPC (Safe Loop
    arm/verify/persist/reapply-on-boot). Riskiest piece → own safety audit + HW run.
  - **Phase 3** — polish: fold Fast/Long modes into F2 depth, reapply-on-boot, retire/repurpose
    the F1 button path.
- **Git**: branch synced to `origin/master` (`e60a6f7` = Codex UI + Fast/Long modes `3c82e96`). The abandoned F1
  knee-seeking commit (`cc8710a`, F1-specific, moot under F2) was dropped in the diagnosis session (reflog-
  recoverable there); nothing of value lost — the modes are on master and the insight is recorded here.

## F1b live-forge BUTTON MODES — Fast / Standard / Long; Standard byte-identical; `validation_passes` delivered as a bounded MODE (2026-06-26)
- **What**: implemented DEFERRED #1 (two button modes) for the live multi-clock forge. Three modes around
  the proven button — FAST (quick discovery), STANDARD (unchanged default), LONG (everything up-front).
  Implemented + validated + committed (`3c82e96`, pushed to master); no hardware run yet (one supervised
  test of the button pending).
- **Decision — keep Standard byte-identical, ADD Fast/Long (3 modes, not a 2-mode replacement)**: the plain
  `StartPowerSweep` still runs the exact just-HW-validated path (24 probes / per-target 3 / one 35 s ceiling
  soak), pinned by a new test. Rationale: do NOT degrade or alter a freshly hardware-validated default;
  additive modes are strictly safer than mutating the proven one. Alternatives rejected — replacing the
  button with only Fast/Long would change the proven default's behavior; shortening the FAST soak would
  weaken the per-pick safety floor (the fail-closed 35 s ceiling soak must run ≥1× in EVERY mode).
- **Knobs (named consts; hardware-relative PROBE counts, NOT fixed MHz)**: FAST `12/2/1`, STANDARD `24/3/1`,
  LONG `40/4/3` (max_probes / max_probes_per_target / ceiling-soak passes). Passes clamped to a defensive
  `POWER_SWEEP_MAX_VALIDATION_PASSES=5`. LONG's repeated soak is the multi-clock analogue of F2's
  `--validation-passes`: it lets a deep point earn in-session confidence; any failed pass DROPS the pick
  (fail-closed), so extra passes can only REJECT, never widen exposure.
- **IPC — additive**: two NEW unit methods `StartPowerSweepFast` / `StartPowerSweepLong`; `StartPowerSweep`
  unchanged (= Standard). No payload/field change → no contract break. This realises the `validation_passes`
  "IPC parameter when the Forge action is wired" the 2026-06-23 entry anticipated — delivered as a BOUNDED
  mode rather than a free-form integer (smaller, safer surface; values can't be widened by the caller). UI
  toggle requested from Codex in `docs/contracts/ui-backend.md`.
- **Safety**: `apply_vf_ceiling_monotone` / Safe Loop / `reset_to_stock` / verifier / the probe+soak motor
  all untouched; no auto-apply; persist only when `godforge.is_some()`. FAST reduces exposure; LONG is a
  longer supervised run of the SAME bounded fail-closed motor (global `max_probes` stays the hard cap), no
  new risk class. Recommend an independent `nidavellir-safety-auditor` pass before any confirmed LONG run.
- **Validation**: cargo check clean; `core 59 / nvapi 38 / service 293` (+1) tests pass; clippy zero new
  warnings. Files: `crates/core/src/ipc.rs`, `crates/service/src/{gpu_power_sweep.rs, ipc_server.rs}`,
  `docs/contracts/ui-backend.md`.
- **Next**: optional independent safety audit → one supervised LONG hardware run (confirm multi-pass ceiling
  validation end-to-end) → DEFERRED #2 (IDLE / cross-run multi-clock confidence).

## F2 multi-clock profile package — Brokkr's 0.95 + descending ladder (Caminho B) + confidence opt-in; "906 vs 868" is the CONFIDENCE GATE, not a margin (2026-06-23)
- **What**: three approved backend changes toward the v0.5 multi-clock profile frontier — implemented +
  validated + safety-audited, NOT yet committed (awaiting operator approval). No hardware run.
- **Key finding (answers "why applied 906 not the 868 the sweep reached")**: `synthesize_forge_profiles`
  selection is **voltage-agnostic** — it picks points by clock/power/p5/**confidence**; `vf_table_voltage_mv`
  is the deterministic apply axis, NOT a selection input. The 868→906 gap is the **Wilson confidence gate**
  (`confidence_threshold = 0.85`): a once-validated point has confidence ~0.21 and is filtered out; the
  deepest point that earned enough repeat confirmations wins. It is NOT a fixed mV safety margin.
- **Part 1 — Brokkr's floor 0.98 → 0.95** (`ForgePolicy::balanced`): the efficiency knee may now sit up to
  5% below Godforge (was 2%) for larger watt savings; Deep Calm stays 0.90, confidence gate stays 0.85.
  Selection-only; authorizes no voltage/clock. Three floor-boundary tests decoupled to an explicit 0.98
  literal; new test pins the 0.95 default.
- **Part 2 — Caminho B descending ladder** (`ladder_target_descent_bounds`, gpu_f2_sweep): `run_anchored_ladder_sweep`
  is now direction-aware. DESCENDING targets start the descent at the prior clock's last-good (a CEILING) with
  the full BASE hardware floor, so each lower clock reaches its OWN deeper min-V; ASCENDING/first keeps today's
  prior-as-floor behavior byte-for-byte. The confirmed loop chains the freshly-discovered last-good forward via
  `prev_good`. (The old ascending-floor ladder over-floored a descending list — Caminho B fixes that.)
- **Part 3 — confidence opt-in `--validation-passes N`** (default 1, hard cap `F2_MAX_VALIDATION_PASSES=20`):
  an opt-in that re-validates ONLY the deepest validated point up to N-1 extra times in ONE session (reuses the
  safe arm→apply→verify→dwell→reset motor + per-pass Safe Loop/blacklist precheck; stops on any non-Validated;
  records one observation per pass so `validations_at_best`/confidence accumulate). Default 1 = strict no-op =
  today's behavior. Lets a deep point EARN the 0.85 gate in one longer session instead of waiting days/runs —
  WITHOUT lowering the gate. Mode 1 (accrue over runs) kept intact; idle auto-validation = FUTURE.
- **UI**: contract request for Codex (`docs/contracts/ui-backend.md`, 2026-06-23): profiles from the multi-clock
  frontier, Brokkr's 95%, honest collapse (Brokkr's ≡ Godforge on power-limited GPUs), confidence-is-a-gate-not-a-
  margin messaging, "Build confidence now" opt-in (default OFF), idle-validation future.
- **Validation**: cargo check clean; tests gpu-nvapi 38 / core 59 / service 292 pass (8 added, 1 fixed); clippy no
  new warnings. Independent safety audit (nidavellir-safety-auditor): **GO**, all 8 items PASS — re-validation
  reuses the safe motor + is bounded/fail-closed, descending uses the base floor under the unchanged planner
  gates, no apply/persist/promote, `apply_vf_ceiling_monotone`/F1 untouched, ForgePolicy selection-only. Non-
  blocking: a cosmetic synthesized stop_reason on a failed extra pass (no safety effect); the over-cap gate is
  correct-by-inspection but only tautologically tested.
- **State**: no `--confirm`, no hardware, no profile apply/persist/promotion, no commit/push. Observation store
  unchanged (8 records / last_good 962 mV).
- **Next**: operator review → commit/push; then a SUPERVISED confirmed descending multi-clock ladder (anchored at
  the validated top) to populate the frontier so the classifier can differentiate the three profiles; later, wire
  `validation_passes` + the F2 frontier into the live Forge IPC (still classifier-preview today).

## F2 target sweep — LEARNED OFFSET HORIZON implemented (+210 abs / +15 step); dry-run shows step-budget is today's binding limit; hardware run HELD (2026-06-22)
- **What**: implemented the target-sweep-specific progressive absolute-offset horizon the prior entry called
  for (its "separately-reviewed algorithm change … NOT a cap widening"). Commit `c40a78d`
  `feat(service): add f2 target sweep learned offset horizon`, pushed to `origin/master`.
- **Design**: new `TARGET_SWEEP_HORIZON_MAX_MHZ = +210` constant + `PositiveOffsetLimits::target_sweep_learning_horizon(floor, ceiling)`
  in gpu-nvapi — abs +210, per-step STILL +15 (the critical difference from `manual_prior`, which widens BOTH
  caps for a one-shot known point). Only the `--auto-sweep` dispatch builds it; default/ladder/manual-prior keep
  `conservative` (+30/+15). +210 is reachable ONLY by accumulating validated chained +15 steps, each gated by a
  prior Validated outcome + clean reset + cleared boot flag. NOT a global cap widening.
- **Hard-cap rationale (+210)**: lets the descent reach a low-voltage bin ~200 MHz below target; stays strictly
  below the manual-prior +250 (autonomous discovery stays more conservative than an operator-asserted point);
  explicit, bounded, constant, never CLI-widenable. The per-step +15 + chained validation is the real safety
  mechanism; the abs cap is a hard backstop. effective_mhz = base+offset = target, so the larger abs cap can
  never authorize a clock above the stock-boost-top ceiling, nor bypass the hardware floor.
- **Validation**: cargo check clean; tests pass — `nidavellir-gpu-nvapi` 38, `nidavellir-core` 59,
  `nidavellir-service` 284 (8 new horizon tests); clippy adds ZERO new warnings. Independent safety audit
  (nidavellir-safety-auditor): **GO**, all 11 checklist items PASS — no global widening, manual-prior isolated,
  per-step +15 preserved, both caps still fail-closed, `apply_vf_ceiling_monotone`/F1 untouched, no single +210
  jump (~14 validated steps), confirmed sweep still bounded by `F2_CONFIRMED_MAX_STEPS=3`, no profile persist.
- **Dry-runs (no --confirm)**: default progressive still `abs +30 / per-step +15 (constants — NOT CLI-widenable)`
  (unchanged); manual-prior still `+250 (DEFAULT discovery cap stays +30 — unaffected)` (unchanged); `--auto-sweep`
  now shows `abs +210 MHz (TARGET-SWEEP LEARNING HORIZON …)`, resumes from prior validated 962 mV/+30, and PLANS
  6 candidates continuing below 962: #4 962/+45, #5 956/+45, #6 950/+60 (each step Δ ≤ +15).
- **MATERIAL FINDING — today's binding limit is the STEP BUDGET, not the +30 cap**: the prior entry attributed
  the 968 mV saturation to the +30 absolute cap, but this session's live curve has THREE bins within +30 near
  the top (981/+15, 975/+15, 968/+30). A confirmed run executes only the first `F2_CONFIRMED_MAX_STEPS`=3
  candidates AND the descent restarts from the curve top (981 mV) each run, so it would reach only **968 mV** —
  shallower than the 962 mV frontier — and would NOT advance discovery this session. Raising the absolute cap
  correctly unblocks the PLANNER (candidates #4–#6 now exist) but does not, by itself, make the confirmed run
  reach them. The safety auditor independently flagged the same (its C1).
- **Decision — HARDWARE RUN HELD** (operator choice): no confirmed sweep was run. A TDR/reboot-risk operation
  that only re-validates already-known-good points (981/975/968) without advancing the frontier is poor value;
  safety-first favors holding. State untouched: no `--confirm`, no VF write, no Safe Loop arm, no profile
  apply/persist/promotion; observation store still 8 records / `last_good 962 mV / first_bad None`.
- **Next recommended task**: a SCOPED, separately-reviewed follow-up so the confirmed sweep RESUMES ITS DESCENT
  START near the validated baseline (skip already-validated shallow bins) — then the deep candidates (962/+45,
  956, 950) fall within the 3-step budget and the horizon actually advances the frontier in one supervised run.
  Alternative: a bounded multi-clock LADDER over 1815/1830.

## F2 target sweep 1800 MHz — second confirmed chained run; frontier saturated at the +30 absolute cap (~962–968 mV) — PASS (2026-06-22)
- **What**: a third confirmed official target sweep (second run of the chained-descent build) at HEAD
  `01b97ca`: `undervolt-probe --target-mhz 1800 --auto-sweep --confirm`. One confirmed command, operator
  present, no second run.
- **Result — PASS** (exit 0): **3/3 Validated**, `CompletedAllPlanned`. #1 981 mV/+15 (avg/p5 1815, 191 W),
  #2 975 mV/+15 (avg 1803/p5 1800, 198 W), #3 968 mV/+30 (avg/p5 1815, 193 W). All RaiseVerified + dwell
  Stable; reset + boot-flag cleared for all 3; no TDR/DeviceLost/Unstable/ClockDrop. `first_bad None`,
  frontier updated, ended safe.
- **Key finding — the conservative sweep is now ABSOLUTE-CAP-BOUNDED**: this session's static VF-table read
  sat slightly higher (boost top 1935 vs 1950 MHz), so the deepest bin reachable within the **+30 absolute
  cap** was **968 mV/+30**; the next bin needs +45 → `offset +45 exceeds the absolute cap +30 — fail closed`.
  The chained baseline (resume from the prior validated 962 mV/+30) only relaxes the PER-STEP cap, never the
  ABSOLUTE cap, so it cannot push below ~962 mV. `last_good` therefore stays **962 mV** (the prior run's
  deeper point in the full store) — the 1800 MHz official frontier has reached its conservative floor and
  re-confirms cleanly across sessions. Going deeper would need a separately-reviewed algorithm change (e.g.
  cumulative multi-run offset beyond a single +30 step), NOT a cap widening.
- **Cleanup correct**: `gpu_applied.json`/`boot_flag.json` ABSENT after; `forge_state`/`gpu_knowledge`/
  `heartbeat`/`safe_loop` byte-identical (no persist/apply/promote, no knowledge mutation, no new blacklist);
  `f2_observations.jsonl` 5→8 (7 validated + the 1 preserved no-write abort). git clean.
- **Next recommended task**: the 1800 MHz conservative frontier is saturated at the +30 cap; pivot to a
  bounded LADDER over additional targets (e.g. 1815/1830) to build the real multi-clock frontier — supervised,
  one confirmed run at a time — rather than re-running 1800.

## F2 target sweep — observation-aware CHAINED DESCENT + first full-descent hardware run (1800 MHz @ 962 mV) — PASS (2026-06-22)
- **What**: the planner refinement the PASS-PARTIAL run called for, then its first confirmed hardware run.
  `undervolt-probe --target-mhz 1800 --auto-sweep --confirm` at HEAD `fcdf04d`. One confirmed command,
  operator present, no second run.
- **The bug**: the confirmed motor applied each candidate's offset from STOCK (+0) after `reset_to_stock`, so
  a candidate needing +30 was rejected by the +15 per-step cap even when it was only +15 above an already
  VALIDATED point — the PASS-PARTIAL stop at candidate #2.
- **The fix — observation-aware chained same-target descent** (`feat(service): refine f2 target sweep descent
  baseline`): the confirmed motor measures each candidate's per-step increase against the **last validated
  offset** — the prior candidate THIS run (the motor only reaches candidate `i` after `i-1` validated), or for
  candidate 0 the deepest prior VALIDATED same-target/same-GPU **observation** (cross-run resume), or 0 when
  none. The **ABSOLUTE +30 cap still bounds every candidate's absolute offset**; only the per-step REFERENCE
  moves from stock to the last validated point. Pure helpers: `validated_descent_baseline` (core),
  `chained_prev_offset` (service). gpu-nvapi cap functions were already parameterized by `prev_offset_mhz`, so
  the writer, `apply_vf_ceiling_monotone`, the verifier, and the manual-prior (+250) cap are all UNCHANGED.
- **Why it is safe**: a no-write `AbortedBySafetyGate`/`RejectedByPlanner` record is never `Validated`, so it
  can never become a baseline, a `first_bad`, or a blacklist entry — the prior 968/+30 abort does NOT block
  replanning. A baseline is only as deep as a point that ALREADY validated on this hardware, so chaining can
  never authorize an absolute offset beyond +30. Default progressive, manual-prior, F1/build-frontier, and the
  ladder's voltage-FLOOR policy are untouched (the `--steps`/ladder confirmed paths share the within-run
  advancement — the same bug fix — but seed no cross-run baseline).
- **Result — PASS** (exit 0): **3/3 candidates Validated**, `CompletedAllPlanned`. #1 975 mV/+15 (avg 1803,
  p5 1770, 198 W), #2 968 mV/+15 (avg/p5 1800, 190 W), #3 **962 mV/+30** (avg/p5 1800, 191 W) — the exact +30
  point that ABORTED in the PASS-PARTIAL run now validates via the chained +15 delta. New min stable voltage
  **962 mV** (was 975); `first_bad None`, frontier updated, ended safe. No TDR/DeviceLost/Unstable/ClockDrop.
- **Cleanup correct**: `reset_to_stock_ok` + `boot_flag_cleared` true for all 3; `gpu_applied.json` /
  `boot_flag.json` ABSENT after; `forge_state.json` / `gpu_knowledge.json` / `heartbeat.txt` / `safe_loop.json`
  byte-identical (no persist/apply/promote, no knowledge mutation, no new blacklist). 3 observations appended
  (store 2→5); the 2 prior records (incl. the old no-write abort) preserved as history.
- **Next recommended task**: extend the validated 1800 MHz frontier with a bounded LADDER (multiple targets)
  to build the real multi-clock frontier (F1b/F2 convergence), still supervised, one confirmed run at a time.

## F2 target sweep — FIRST official hardware run (1800 MHz @ 975 mV validated) — PASS-PARTIAL (2026-06-22)
- **What**: the FIRST bounded hardware run of the OFFICIAL F2 target sweep (progressive anchored descent,
  NOT manual-prior): `undervolt-probe --target-mhz 1800 --auto-sweep --confirm` on the freshly-built debug
  binary at HEAD `8dbd296`. One confirmed command, operator present, no second run.
- **Result — PASS-PARTIAL** (exit 0): candidate **#1 Validated** (anchor **975 mV**, base 1785, **+15 →
  1800 MHz**; verify **RaiseVerified**; dwell **Stable** avg/p5 **1815 MHz**, **191 W**, `silent_error=false`);
  candidate **#2 aborted_by_safety_gate** — a benign planner fail-closed (per-step **+30 > +15** cap), **no
  VF write** (`verifier/dwell = not_run`, watts null). `last_good=975 mV`, `first_bad=None`, bracket=None,
  frontier updated. No TDR / DeviceLost / Unstable / ClockDrop / reboot.
- **Cleanup correct**: `reset_to_stock_ok=true` and `boot_flag_cleared=true` for BOTH candidates; ended safe
  (reset). `gpu_applied.json` / `boot_flag.json` ABSENT after run; `forge_state.json` / `gpu_knowledge.json`
  / `heartbeat.txt` byte-identical (no profile persisted/applied/promoted, no knowledge mutation);
  `safe_loop.json` content unchanged (`safe_mode=false`, no new blacklist). 2 observations appended to
  `f2_observations.jsonl` (first official observation file).
- **Algorithm observation (NOT changed this task)**: because every candidate restarts from stock (+0) and the
  per-step cap is +15 MHz, only the candidate whose real base is within +15 of the target (1785 → +15) is
  reachable in one supervised step; the deeper-anchor candidates (base 1770, +30) self-abort via the cap. So
  the 1800 MHz progressive sweep effectively validates a single point per run, and the descent below 975 mV
  is not explorable under the current single-step-from-stock + +15 cap design.
- **Next recommended task**: a planner refinement so the official sweep can descend past the first reachable
  anchor (e.g. carry the prior validated offset as the next step's baseline for SAME-TARGET descent, or widen
  the per-step cap for descent only), then re-run the 1800 MHz sweep to actually bracket the minimum stable
  voltage. Algorithm change only — separate, reviewed task.

## F2 discovery/learning algorithm — observation store + target sweep + ladder sweep + learned frontier IMPLEMENTED (not yet HW-validated) (2026-06-22)
- **What**: the four-block F2 discovery/learning algorithm. Code + tests + docs only — **no hardware, no
  `--confirm`, no VF write, no profile apply/persist/promote** in this task. Checkpoints `0df6179` (store +
  target sweep) and `cb125b6` (ladder + learned frontier).
- **Block 1 — observation store** (`crates/core/src/f2_observation.rs`, pure + testable): `F2Observation`
  DTO (full per-attempt outcome) + serializable `F2ObsMode`/`F2ObsVerifier`/`F2ObsDwell`/`F2ObsOutcome`;
  `F2ObservationStore` = APPEND-ONLY JSONL at `default_data_dir()/f2_observations.jsonl` (mirrors the
  SafeLoopStore path/serde conventions but accumulates across runs; BOM-tolerant; skips malformed lines).
  This is LEARNING data only — NOT profile persistence. Pure queries: `last_good_for_target` (lowest
  validated), `first_bad_for_target` (highest failure), `bracket_for_target` (Vmin in (first_bad,
  last_good]), `is_known_bad` (conservative downward), `learned_frontier`.
- **Block 2 — target sweep** (`undervolt-probe --auto-sweep`): autonomous same-target minimum-stable-
  voltage discovery via the OFFICIAL progressive anchored descent (conservative +30/+15 caps — NOT
  manual-prior), reusing `plan_anchored_undervolt_descent` + `run_confirmed_f2_multi_step`. Bounded by
  `F2_CONFIRMED_MAX_STEPS` (ignores `--steps`; fail closed; no infinite search). The confirmed path records
  one observation per executed candidate (`record_target_sweep`) and reports the discovered last-good /
  first-bad / bracket. Dry-run plans + previews + writes nothing.
- **Block 3 — ladder sweep** (`undervolt-probe --ladder-sweep --targets a,b,c`): runs a target sweep per
  target IN ORDER. A lower target's discovered last-good is used ONLY as a conservative descent FLOOR for
  higher targets (`ladder_target_floor`) — it NEVER assumes the lower voltage holds the higher clock (the
  descent still validates top-down). The ladder STOPS on a safety failure (`ladder_should_continue` =
  reset-clean); a normal bad candidate stops only that target.
- **Block 4 — learned frontier + classifier bridge**: `learned_frontier` emits one `F2FrontierEntry` per
  target (best anchor / offset / sustained / watts / confidence / first_bad / bracket / counts).
  `to_power_sweep_point` bridges each entry to the canonical `(PowerSweepPoint, confidence)` the EXISTING
  classifier consumes (F2 apply axis = the LOWER anchored bin; non-power-bound; `stable=true`).
  `classify_f2_frontier_summary` (gpu_power_sweep) runs the SAME `synthesize_forge_profiles`
  (`ForgePolicy::balanced`) READ-ONLY and previews Godforge / Brokkr's Best / Deep Calm — **no new
  scoring, and no profile selected/applied/persisted/promoted**.
- **Confidence**: `frontier_confidence(validated_count)` is a simple monotone heuristic (one clean
  validation clears the 0.85 balanced gate; repeats raise toward 0.99) — deliberately DECOUPLED from the
  F1b Wilson trial model so F2 learning does not touch profile scoring.
- **Instability is learning data**: `Unstable`/`ClockDrop`/`VerifierFailed` that reset cleanly are recorded
  as bad points (they bracket Vmin) but are NOT safety failures; only `ResetFailed`/`CrashOrRecovery` are
  safety failures that stop a sweep/ladder (`is_safety_failure`).
- **Untouched**: default progressive + manual-prior behavior (the new branches gate on their flags and
  return before the default dispatch); F1/build-frontier; `apply_vf_ceiling_monotone`; Safe Loop;
  `reset_to_stock`; the verifier; `synthesize_forge_profiles` (reused, not modified). v1 stays GPU-only —
  CPU/RAM tuning explicitly deferred; no UI work.
- **Validated (no hardware)**: `cargo test` core 56/0, service 278/0 (incl. F1/build-frontier), nvapi 33/0;
  clippy clean of new code. Dry-runs: `--auto-sweep` (1800) plans the official descent (975/968/962) and
  writes nothing; `--ladder-sweep --targets 1800,1815,1830` plans all three with the conservative-prior
  policy + learned-frontier + classifier preview; default `--steps 3`, manual-prior, and auto-sweep stay
  distinct; `f2_observations.jsonl` / `boot_flag.json` / `gpu_applied.json` ABSENT after every dry-run.
- **Next recommended task**: the FIRST bounded hardware run of the official F2 target sweep
  (`undervolt-probe --target-mhz 1800 --auto-sweep --confirm`, operator present) — NOT another manual
  validation. Manual-prior remains a dev/known-GPU shortcut only.

## F2 MANUAL-PRIOR anchor mode — FIRST confirmed hardware validation (1800 MHz @ 875 mV, +210) — PASS (2026-06-21)
- **One supervised confirmed run** (operator present, ONE confirmed command, no second):
  `undervolt-probe --target-mhz 1800 --start-mv 875 --steps 1 --manual-prior --confirm` on the
  freshly-built binary at commit `34581d0`. **First real MANUAL-PRIOR anchored VF write.**
- **Result: exit 0, outcome `Validated`.** No TDR / black-screen / reboot / DeviceLost / Unstable /
  ClockDrop / silent error; the machine stayed responsive.
- **Candidate (live curve)**: target **1800 MHz**, anchor bin **875 mV**, base **1590 MHz**, offset
  **+210 MHz** → 1800; **26** higher-voltage bins capped DOWN to 1800 (max flatten **-150 MHz**), 18
  already at/below target, **43** lower bins elastic. Offset within the manual-prior cap (+250) and below
  the +250 per-step cap; effective clock 1800 ≤ ceiling 1950.
- **Motor (end-to-end)**: startup recovery ran first ("clean boot, nothing to restore") → Safe Loop
  armed BEFORE the write → `apply_bounded_anchored_positive_offset` (manual limits) → anchored verify
  **`AnchoredRaiseVerified`** (`verifier result = Some(RaiseVerified)`) → dwell **Stable** (avg **1815
  MHz**, p5/sustained **1815 MHz**, **157 W**, `silent_error=false`) → `reset_to_stock` ran + CONFIRMED
  stock (all written bins cleared) → boot flag cleared after the clean reset. Not blacklisted. **No
  profile persisted, applied, or promoted** (Validated reported only; `last_validated` stays null).
- **Undervolt benefit vs the 975 mV run**: same 1800 MHz held at **875 mV** draws **157 W** vs **183 W**
  at 975 mV — ~**26 W** lower for the same clock. avg==p5 (1815) confirms the plateau caps prevent boost
  above 1800; the +15 over target is within the 15 MHz verifier tolerance.
- **State after run**: `boot_flag.json`/`gpu_applied.json` absent; `safe_loop.json` **byte-identical**
  (sha256 `40D4DE38…`, mtime-only touch; `consecutive_crashes=1`, blacklist unchanged, `safe_mode=false`);
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; repo tree clean.
- **Manual-prior is an explicit dev/known-GPU shortcut**; the official unknown-GPU behavior remains
  progressive anchored descent with conservative caps (+30/+15). **Clocks above 1800 at 875 mV are NOT
  assumed** — they must still be discovered progressively.
- **Next recommendation (no second confirmed run in this task)**: either descend below 875 mV for 1800
  (find the minimum stable voltage), or begin progressive discovery for 1815+ without assuming 875 mV.

### Implementation record (pre-hardware)
- **What**: an OPT-IN `--manual-prior` path for `undervolt-probe` that anchors at an operator-provided
  `--start-mv` using a SEPARATE, larger bounded positive-offset cap. It exists ONLY to validate a KNOWN
  manual point faster on the current dev GPU (e.g. `1800 MHz @ 875 mV`). It is NOT the default and NOT
  for unknown GPUs. Code + tests + docs; **no hardware run, no `--confirm`, no VF write** in this patch.
- **Default unchanged**: the official/autonomous unknown-GPU behavior remains progressive anchored
  descent with the conservative caps (+30 abs / +15 per-step). Manual-prior branches BEFORE the default
  dispatch (`run_undervolt_probe`, gated on `args.manual_prior`) and never widens the default caps.
- **Separate cap**: `F2_MANUAL_PRIOR_MAX_POSITIVE_OFFSET_MHZ = 250` (service), surfaced via
  `PositiveOffsetLimits::manual_prior(floor, ceiling, max)` (gpu-nvapi) which widens ONLY the offset caps
  (abs + per-step both = max, since manual-prior is single-step) while floor / clock-ceiling / real-bin /
  sanity checks stay EXACTLY as `conservative`. Still fail-closed: an offset above the cap is REFUSED,
  never clamped; the stock clock ceiling still caps the effective clock (can never overclock).
- **Why 250**: the known point `1800 @ 875` needs ~+210 MHz (live base at the 875 mV bin is 1590 MHz). 250
  covers it with margin yet stays a hard, fail-closed bound. The default +30 cap correctly REFUSES +210.
- **Planner** (`plan_manual_prior_undervolt`): resolves `--start-mv` to the nearest real VF bin at/below
  it (via `select_anchor_bin`), reuses `plan_anchored_undervolt` with the manual limits (so it inherits
  every anchored fail-closed rule), and reports the selected bin / base / required offset even on refusal.
  Anchored semantics intact: anchor raised to target, higher-voltage bins capped DOWN to target, lower
  bins elastic.
- **Confirmed gate** (`confirmed_manual_prior_refusal`): REQUIRES `--start-mv`, then delegates to the
  shared `confirmed_f2_refusal` with the manual limits — inheriting the `--steps 1` single-step gate, the
  Safe Mode / armed-flag / crash-threshold gates, the candidate bound re-checks, and the blacklist check.
  The confirmed branch reuses the validated single-step motor (`run_confirmed_f2_step` / `RealF2Ops`) with
  `limits: manual_limits`; one anchored candidate only; never persists/applies/promotes a profile.
- **Untouched**: F1/build-frontier and `apply_vf_ceiling_monotone` (its positive-offset refusal guard is
  intact), Safe Loop, `reset_to_stock`, the verifier (`verify_anchored_positive_offset`), the blacklist,
  power-limit/TDP/clock-lock. The gpu-nvapi diff adds only the `manual_prior` constructor.
- **Dry-run `1800 @ 875` (read-only, live curve)**: mode ANCHORED + MANUAL-PRIOR; warning "uses
  user-provided prior; not the default unknown-GPU discovery path"; selected **875 mV** bin, base **1590
  MHz**, required **+210 MHz**, manual cap **+250 MHz**, within bounds **YES**, 26 higher bins capped (max
  flatten **-150 MHz**), 43 lower elastic, plan self-check **AnchoredRaiseVerified**; explicit no-op /
  no-write + no-persist/apply/promote lines. Default progressive `1800 --steps 3` remained **unchanged**
  (975 / 968 / 962 mV).
- **Validation (no hardware)**: `cargo check` clean; `cargo test -p nidavellir-service` 269/0 (incl. F1 +
  12 new manual-prior tests); `cargo test -p nidavellir-gpu-nvapi` 33/0. Focused adversarial safety review
  of the diff: no blockers (default unchanged, fail-closed cap, F1 untouched, gates intact).
- **Clocks above 1800 at 875 mV are NOT assumed** — they must still be discovered progressively.
- Hardware validation status: **PASS** — see the validation record at the top of this section
  (`undervolt-probe --target-mhz 1800 --start-mv 875 --steps 1 --manual-prior --confirm`, outcome
  `Validated`). One confirmed run only; no profile persistence/apply/promotion.

## F2 ANCHORED multi-step descent — bounded same-target probing IMPLEMENTED (not yet HW-validated) (2026-06-21)
- **What**: a controlled, bounded, SAME-TARGET ANCHORED multi-step descent for `undervolt-probe`. For one
  target (e.g. 1800 MHz) a confirmed run executes a SHORT sequence of anchored candidates from
  safer/higher voltage to lower voltage, stopping at the first real failure and preserving the last good
  point. Code + tests + docs only — **no hardware was run, no `--confirm`, no VF write** in this patch.
- **Why**: the first confirmed run proved ONE anchored point (1800 @ 975 mV, +15) but not the minimum stable
  voltage for 1800. The descent walks voltage down at the same target to find where it stops holding.
- **Scope (fail-closed)**: single target only; anchored mode only; `--simple` stays single-step. No
  multi-target automation, no autonomous crash-seeking, no profile persistence/apply/promotion, no
  power-limit/TDP/clock-lock change. F1/build-frontier, `apply_vf_ceiling_monotone`, Safe Loop,
  `reset_to_stock`, and the verifier gates are **untouched**.
- **Step cap**: `F2_CONFIRMED_MAX_STEPS = 3`. Confirmed mode enforces its OWN cap
  (`confirmed_f2_multi_refusal`): `--steps` must be `1..=3`, else FAIL CLOSED. The read-only dry-run may
  preview a longer plan (`plan_anchored_undervolt_descent` honors the requested `--steps`), but the
  confirmed branch never executes more than the cap. `--steps 1` keeps the previously hardware-validated
  single-step path byte-for-byte (`confirmed_f2_refusal` + `run_confirmed_f2_step`).
- **Design (reuse, not duplication)**: the descent planner is the anchored analog of
  `plan_undervolt_probe` (descend bins, chain `prev_offset` so the +15 per-step cap bounds how fast the
  undervolt deepens, stop at the first anchored-plan rejection). The orchestrator
  `run_confirmed_f2_multi_step` drives the SAME validated per-candidate motor (`run_confirmed_f2_step`) via
  a candidate-cursor trait `F2MultiStepOps` (`select(i)` re-checks Safe Loop + blacklist BEFORE each
  write). It CONTINUES only on a stably-`Validated` candidate (dwell stable + reset confirmed + boot flag
  cleared) and STOPS immediately on any other result, never attempting a deeper candidate.
- **Per-candidate sequence (unchanged motor)**: check Safe Loop + blacklist → arm boot flag → apply the
  bounded anchored curve → `verify_anchored_positive_offset` → dwell once → `reset_to_stock` → clear boot
  flag ONLY after a confirmed reset. A reset that cannot be confirmed RETAINS the boot flag (fail closed).
- **Stop reasons** (`F2MultiStopReason`): `CompletedAllPlanned`, `VerifierFailed`, `Unstable`,
  `DeviceLost`, `ClockDrop`, `ResetFailed`, `Blacklisted`, `ArmFailed`/`ApplyFailed`, `NoMoreCandidates`.
- **New ClockDrop signal**: `F2DwellOutcome::ClockDrop` — a STABLE (no crash/error) dwell whose sustained
  (p5) clock sags below `target − F2_CLOCK_DROP_TOL_MHZ` (30 MHz) is reclassified as a clock drop: the
  undervolt did not hold the clock under load → stop. It resets + (clears on confirmed reset) + never
  validates + does NOT blacklist (not a crash/instability to record). Additive: the single-step motor's
  Stable/Unstable/DeviceLost behavior is unchanged (the 975 mV point dwelt at p5 1815 ≥ 1800 → Stable).
- **Validation done (no hardware)**: `cargo check`/`cargo test -p nidavellir-service` (256 pass, incl.
  F1/build-frontier + single-step + 12 new multi-step tests), `cargo test -p nidavellir-gpu-nvapi` (33
  pass), clippy introduced no new categories. Read-only dry-run `--target-mhz 1800 --steps 3` planned 3
  anchored candidates (975 mV +15 → 968 mV +30 → 962 mV +30, stop = step budget), preflight OK, explicit
  no-op line; `--help` and `--steps 1` (single-step) output unchanged; `boot_flag.json`/`gpu_applied.json`
  absent after the dry-runs.
- **Intended NEXT hardware validation (one confirmed run, operator present, stop after first non-stable)**:
  `undervolt-probe --target-mhz 1800 --steps 3 --confirm`. No profile persistence/apply/promotion. The
  multi-step path is NOT yet hardware-validated.

## F2 ANCHORED undervolt — FIRST confirmed hardware validation (2026-06-21) — PASS
- **One supervised confirmed run** (operator present, ONE confirmed command, no second):
  `undervolt-probe --target-mhz 1800 --steps 1 --confirm`. This is the FIRST real ANCHORED positive-offset
  VF write on hardware — the anchored confirmed branch from `747a11b` (HEAD = origin/master = `747a11b`,
  tree clean). A fresh worktree-local binary was built first (`cargo build -p nidavellir-service`) because
  `target/debug/nidavellir-service.exe` was ABSENT in the worktree; the built binary mtime is newer than `747a11b`.
- **Preflight gates all passed**: working tree clean; `gpu_applied.json` absent; `boot_flag.json` absent;
  `safe_mode=false`; `boot_flag_armed=false`; `consecutive_crashes=1` (< 3); the planned anchored point was NOT
  blacklisted (preflight `blacklisted_points=0`; the 4 existing blacklist entries are all far from 1800 MHz @
  975 mV / +15). Help printed usage only (no hardware read/plan/mutation); dry-run was mode **ANCHORED** with
  exactly ONE anchored candidate and the no-op line (no Safe Loop arm, no apply, no dwell, no VF write).
- **Confirmed result: exit 0, outcome `Validated`.** No TDR, no black-screen, no reboot, no DeviceLost, no
  Unstable, no silent error.
- **Exact anchored candidate (live curve at confirm time)**: target **1800 MHz**, anchor voltage bin **975 mV**,
  anchor base clock **1785 MHz**, positive offset **+15 MHz** → effective 1800 MHz. **27** higher-voltage bins
  capped DOWN to 1800 (max flatten **-150 MHz**), 1 already at target, **59** lower bins left elastic. Offset
  within both caps (per-step +15, absolute +30; constants, not CLI-widenable). NB: the earlier read-only dry-run
  reported the anchor as 981 mV / 25 capped / -135 / 61 elastic; the confirmed run read the live curve fresh and
  the anchor that needed +15 to reach 1800 sat at the 975 mV bin (981 mV was already at base 1800 → capped +0).
- **Execution sequence (the safety-critical motor, end-to-end)**: Safe Loop boot flag armed BEFORE the VF write
  → F2 bounded ANCHORED writer (`apply_bounded_anchored_positive_offset`) applied the full curve →
  `verify_anchored_positive_offset` passed (**`AnchoredRaiseVerified`**; summary `verifier result = Some(RaiseVerified)`)
  → single dwell **Stable** (avg clock **1815 MHz**, p5/sustained **1815 MHz**, **183 W**, `silent_error=false`) →
  `reset_to_stock` ran and CONFIRMED stock (all written bins cleared) → boot flag cleared after the clean reset.
  Point NOT blacklisted. **No profile persisted, applied, or promoted** ("validated" reported only, never written
  to Safe Loop `last_validated`).
- **Post-run state (read-only verify)**: `boot_flag.json` absent; `gpu_applied.json` absent; `safe_loop.json`
  **byte-identical** (sha256 unchanged; mtime touched only by the arm→clear cycle — `safe_mode=false`,
  `consecutive_crashes=1`, blacklist still 4 entries, `last_validated=null`); `forge_state.json` /
  `gpu_knowledge.json` / `heartbeat.txt` unchanged; git tree clean; HEAD still `747a11b`.
- **Boost constrained vs the prior SIMPLE F2 run** — this is the key result: the simple positive-offset run boosted
  ELASTICALLY above target (avg **1868**, p5 **1845**, **199 W**; avg > p5). The anchored run pins a flat plateau at
  the target (avg **1815** = p5 **1815**, **183 W**; ~**16 W** lower). avg==p5 confirms the higher-voltage caps
  prevent boost above the target; the +15 MHz over 1800 is within the verifier's 15 MHz tolerance.
- **Decision recorded**: the F2 ANCHORED-undervolt HARDWARE path is now PROVEN at one bounded point. This confirms
  the classic undervolt SHAPE on real hardware — `target MHz @ anchor mV`, higher-voltage bins capped to prevent
  boost above target, lower bins elastic — and that the motor (**arm → write → verify → dwell → reset → clear**) is
  recoverable. It is the first result that directly supports the intended METHOD: map the stable voltage limit for a
  chosen clock, repeat across clocks, then synthesize Godforge / Brokkr's Best / Deep Calm from real curve points.
  It does NOT yet prove the MINIMUM stable voltage for 1800 MHz — only that one bounded anchored point holds.
- **Next direction (do NOT immediately run another confirmed command before this record is committed)**: a bounded,
  still-supervised, same-target **MULTI-STEP** anchored probe at 1800 MHz that descends voltage until verifier fail,
  instability, clock drop, voltage floor, or budget — using the same Safe Loop / verification / reset guarantees.
- **Scope of this entry**: docs/continuity only (`decisions.md`, `handoff.md`, `memory.md`). No code edits, no tests,
  no further hardware. The validation it records ran exactly one debug build, one `--help`, one dry-run, and ONE
  confirmed run.

## F2 ANCHORED undervolt is now the intended path for classic undervolt probing (2026-06-21)
- **Decision**: move F2 from a single positive offset at one VF bin to a true CLASSIC anchored
  undervolt point. For a chosen target clock + voltage bin, the planner now: (1) RAISES the anchor bin
  to the target with a bounded positive offset; (2) CAPS/flattens every HIGHER-voltage bin DOWN to the
  same target (offset ≤ 0; already-at/below-target bins stay 0 — never raised); (3) leaves LOWER-voltage
  bins elastic (offset 0). The result is the point `target MHz @ anchor mV with boost above target
  prevented`. **ANCHORED is the DEFAULT mode**; `--simple` retains the original single-bin descent for
  comparison/diagnostics.
- **Why**: the first confirmed F2 hardware run (`78ecfc7`, below) proved the positive-offset MOTOR but
  was NOT anchored — the GPU still boosted ABOVE the nominal 1800 MHz target (dwell avg 1868, p5 1845).
  For the classic undervolt method, `1800 MHz @ 981 mV` must be tested as an anchored curve point, not
  one raised bin with the rest of the boost curve free. The anchored plan caps the whole plateau so the
  card cannot exceed the target during the test.
- **Isolation / safety**: this is a NEW, separate, fail-closed path. It does NOT touch or relax
  `apply_vf_ceiling_monotone` (build-frontier flatten-down) or the F1 path. New symbols:
  `plan_bounded_anchored_positive_offset` / `apply_bounded_anchored_positive_offset` /
  `AnchoredPositiveOffsetPlan` (gpu-nvapi); `verify_anchored_positive_offset` /
  `AnchoredOffsetVerification::AnchoredRaiseVerified` (gpu_verify); `plan_anchored_undervolt` /
  `anchored_plan_lines` / `UndervoltMode` (gpu_undervolt). The anchor raise REUSES the bounded single-bin
  planner, so it inherits every floor/offset/per-step/clock-ceiling fail-closed rule. The planner REJECTS
  (never clamps) a non-real anchor, an empty/foreign/non-sane curve, a target above the clock ceiling, an
  offset above the absolute/per-step cap, any positive offset outside the anchor, a higher-voltage bin
  left above target, or a non-monotone lower bin above target.
- **Anchored verifier**: confirms the anchor was raised to target (within tol), every higher-voltage bin
  sits at/below target + tol, and NO bin outside the anchor carries a positive offset. Verdicts:
  `AnchoredRaiseVerified` / `AnchorRaiseIncomplete` / `AnchorOverRaise` / `HigherBinAboveTarget` /
  `UnexpectedPositiveOffset` / `Unverifiable`. The simple `verify_positive_offset` and the F1 flatten-down
  verifier are UNCHANGED.
- **Confirmed branch** (NOT executed this patch): in anchored mode it writes ONE anchored curve plan
  (anchor + plateau caps + elastic zeros) via the anchored writer, verifies with the anchored verifier,
  and confirms EVERY written bin reads ~0 after `reset_to_stock`. Still single-step only (requires
  `--steps 1`), still arms Safe Loop before the write, still resets on every post-arm exit, still clears
  the boot flag only after a confirmed reset, still no persistence/apply/promotion.
- **Dry-run validated read-only** on hardware (`--target-mhz 1800 --steps 1`, no `--confirm`): anchor
  **981 mV base 1785 +15 → 1800** (the same candidate the prior confirmed run used), **25** higher-voltage
  bins capped DOWN to 1800 (max flatten **-135 MHz**), 2 already at target, **61** lower bins elastic,
  `plan self-check = AnchoredRaiseVerified`, no-op (no arm/apply/dwell/VF write). No Safe Loop mutation.
- **Hardware NOT yet validated for anchored mode.** First future anchored validation: `--target-mhz 1800
  --steps 1` (one candidate, operator present, no second confirmed run). NOT multi-step yet.

## F2 true-undervolt — FIRST confirmed hardware validation (2026-06-21) — PASS
- **One supervised confirmed run** (operator present, ONE confirmed command, no second):
  `undervolt-probe --target-mhz 1800 --steps 1 --confirm`. This is the FIRST real positive-offset VF write
  on hardware — the confirmed F2 branch implemented in `78ecfc7` (top of branch; HEAD = origin/master =
  `78ecfc7`, tree clean). A fresh worktree-local binary was built first (`cargo build -p nidavellir-service`)
  because `target/debug/nidavellir-service.exe` was ABSENT; the built binary mtime is newer than `78ecfc7`.
- **Preflight gates all passed**: working tree clean; `gpu_applied.json` absent; `boot_flag.json` absent;
  `safe_mode=false`; `boot_flag_armed=false`; `consecutive_crashes=1` (< 3); the planned point was NOT
  blacklisted (preflight `blacklisted_points=0`; the 4 existing blacklist entries — offsets 255/855, 300/937,
  330/925 and freq 1755/862 — are all far from 1800 MHz @ 981 mV / +15). Help printed usage only (no hardware
  read/plan/mutation); dry-run showed exactly ONE candidate with the no-op line (no Safe Loop arm, no apply, no
  dwell, no VF write).
- **Confirmed result: exit 0, outcome `Validated`.** No TDR, no black-screen, no reboot, no DeviceLost, no
  Unstable, no silent error.
- **Exact candidate**: target **1800 MHz**, voltage bin **981 mV**, base clock at bin **1785 MHz**, positive
  offset **+15 MHz** → effective 1800 MHz. Within both caps (per-step +15, absolute +30; constants, not
  CLI-widenable).
- **Execution sequence (the safety-critical motor, end-to-end)**: Safe Loop boot flag armed BEFORE the VF write
  → F2 bounded positive-offset writer (`apply_bounded_positive_offset`) applied the offset → `verify_positive_offset`
  passed (**`RaiseVerified`**) → single dwell **Stable** (avg clock **1868 MHz**, p5/sustained **1845 MHz**,
  **199 W**, `silent_error=false`) → `reset_to_stock` ran and CONFIRMED stock (offset cleared) → boot flag cleared
  after the clean reset. Point NOT blacklisted. **No profile persisted, applied, or promoted** ("validated" is
  reported only, never written to Safe Loop `last_validated`).
- **Post-run state (read-only verify)**: `boot_flag.json` absent; `gpu_applied.json` absent; `safe_loop.json`
  **byte-identical** (sha256 unchanged; mtime touched only by the arm→clear cycle — `safe_mode=false`,
  `consecutive_crashes=1`, blacklist still 4 entries, `last_validated=null`); `forge_state.json` /
  `gpu_knowledge.json` / `heartbeat.txt` unchanged; git tree clean; HEAD still `78ecfc7`.
- **Decision recorded**: the F2 true-undervolt HARDWARE path is now PROVEN at one bounded positive-offset point.
  This validates the safety-critical motor (**arm → write → verify → dwell → reset → clear**) on real hardware and
  proves it is recoverable. It does NOT yet prove an optimal undervolt profile — it proves the minimum hardware
  path is viable and clean. The dwell clock landing ABOVE 1800 MHz (1868 avg) is EXPECTED and acceptable: this
  probe does not lock the GPU clock, so the card still boosts per curve/power behavior; the verifier's
  `RaiseVerified` confirms the +15 raise on the 981 mV bin.
- **Next direction (do NOT immediately run another confirmed command)**: the next implementation/design step
  should be one of — (1) a controlled, still-bounded/supervised F2 MULTI-STEP probe for the same target;
  (2) explicit `--start-mv` for confirmed single-step probing if not already supported; (3) result recording /
  Forge Knowledge for validated F2 candidates WITHOUT profile promotion. The first true optimization step should
  search the lower-voltage limit around the 1800 MHz target using the same Safe Loop / verification / reset
  guarantees.
- **Scope of this entry**: docs/continuity only (`decisions.md`, `handoff.md`, `memory.md`). The only commands
  this validation ran were one debug build, one `--help`, one dry-run, and ONE confirmed run — no code edits, no
  tests, no further hardware.

## F2 confirmed single-step branch — IMPLEMENTED, not executed (2026-06-20) — no hardware run
- **Decision**: implement the FIRST real confirmed F2 hardware branch (the `--confirm` path of
  `undervolt-probe`), but do NOT execute it. Single-target, single-step only. The branch is isolated
  behind a trait so its fail-closed state machine is unit-tested with a mock (no hardware), while the
  real executor wires to the validated NVAPI writer + dwell + reset.
- **Confirmed state machine** (`run_confirmed_f2_step` over the `F2Ops` trait, in `gpu_undervolt.rs`):
  arm Safe Loop boot flag → apply ONE bounded positive offset (the F2 writer
  `apply_bounded_positive_offset`, NOT `apply_vf_ceiling*`) → verify the write (offset-presence;
  idle GetStatus freq passed as `None`) → dwell once → `reset_to_stock` on EVERY exit path → clear the
  boot flag ONLY after a CONFIRMED reset. Outcomes: ArmFailed / ApplyFailed / VerifyFailed / Unstable /
  DeviceLost / ResetFailed / Validated.
- **Cleanup / boot-flag policy** (the critical invariants, all unit-tested):
  - reset is attempted after any post-arm exit; the real `reset_to_stock` re-reads the bin offset and
    returns `Ok` ONLY when it confirms ~0 — an unreadable or non-zero readback FAILS CLOSED → flag
    RETAINED. (F2 must NEVER leave a positive offset applied after exit.)
  - boot flag cleared ONLY when reset is confirmed; on DeviceLost (crash/TDR) the flag is RETAINED so
    startup recovery blacklists + recedes; on a failed reset the outcome becomes `ResetFailed` and the
    flag is retained.
  - DeviceLost and Unstable record the point in the Safe Loop blacklist (crash knowledge); only a
    Stable dwell + confirmed reset yields `Validated` — and "validated" is REPORTED only, never written
    to Safe Loop `last_validated` (so nothing auto-reapplies). No profile persist/apply/promotion.
- **Confirmed preflight** (`confirmed_f2_refusal`, pure, fail-closed): refuses unless `--steps 1`
  (single-step only); not in Safe Mode; no boot flag already armed; `consecutive_crashes` <
  `SAFE_MODE_CRASH_THRESHOLD` (=3); a candidate exists; the candidate is within all offset/floor/clock
  bounds (defensive re-check); and the intent is not blacklisted (checked against BOTH the 3-axis F2
  intent and the 2-axis freq/vf_bin point, matching build-frontier regions). `run_undervolt_probe_cmd`
  runs startup recovery first on `--confirm` (mirrors build-frontier); on refusal NO hardware is touched.
- **CLI help fix**: `undervolt-probe --help` / `-h` now short-circuits in `run_undervolt_probe_cmd`
  BEFORE any hardware read / plan / Safe Loop access, printing usage (all flags + an explicit
  `--confirm` may-write-VF / operator-supervision warning).
- **F1 untouched**: `apply_vf_ceiling_monotone` and the build-frontier algorithm are unchanged; the only
  `gpu_power_sweep.rs` edits are ADDITIVE/visibility — `reset_to_stock` → `pub(crate)` and new
  `pub(crate) single_load_dwell()` / `SingleDwell` adapters that reuse the validated `load_and_measure`.
  No power-limit/TDP/clock-lock change. Dry-run output unchanged except the footer + help.
- **First future run** (operator present, no second confirmed run):
  `undervolt-probe --target-mhz 1800 --steps 1 --confirm`.
- **Hardware: STILL NOT VALIDATED.** No `--confirm` executed in this task; no VF write; no Safe Loop
  mutation (the confirmed branch — the only mutating path — was never run). Validation: `cargo check`
  clean; `cargo test -p nidavellir-service` 228/0; `cargo test -p nidavellir-gpu-nvapi` 25/0; dry-run +
  `--help` exercised read-only.

## F2 true-undervolt foundation — first isolated planner/verifier/probe (2026-06-20) — pure, no hardware
- **Direction**: begin F2 (true undervolt) as a SEPARATE, ISOLATED path from F1/build-frontier. F1's
  `build-frontier` deliberately FLATTENS DOWN: `apply_vf_ceiling_monotone` refuses `desired_offset_mhz > 0`
  and the flatten-down verifier treats any clock above target as an overshoot failure. True undervolt is the
  opposite operation — it must RAISE a lower-voltage bin's clock (a bounded POSITIVE offset) so the target
  clock holds at a lower voltage. Because positive offsets are safety-critical, F2 cannot reuse the F1 writer
  or verifier; it gets its own bounded, fail-closed symbols. Read-only safety audit verdict was
  `READY WITH REQUIRED SAFETY CHANGES`; this is the first of those changes.
- **What landed (no hardware, dry-run-first)**:
  - **gpu-nvapi** (`crates/gpu-nvapi/src/lib.rs`): pure `plan_bounded_positive_offset` + windows-gated
    `apply_bounded_positive_offset`, with `PositiveOffsetPlan` / `PositiveOffsetLimits` and conservative
    constants `POS_OFFSET_MAX_MHZ = +30` (absolute cap) and `POS_OFFSET_STEP_MAX_MHZ = +15` (per-step cap).
    These are NEW symbols — `apply_vf_ceiling_monotone` and the F1 flatten-down path are UNCHANGED.
  - **Verifier** (`crates/service/src/gpu_verify.rs`): pure positive-offset-aware
    `verify_positive_offset` → `PositiveOffsetVerification` (RaiseVerified / RaiseIncomplete / OverRaise /
    Unverifiable). The intended raise is the SUCCESS case here (it does NOT use the flatten-down overshoot
    veto); it still rejects an unintended over-raise above target + tolerance. The existing flatten-down
    verifier (`classify_curve` / `eval_ceiling_evidence`) is untouched.
  - **F2 module** (`crates/service/src/gpu_undervolt.rs`, NEW): pure `plan_undervolt_probe` search skeleton
    (descend real voltage bins, compute the bounded positive offset each bin needs to hold the focus target,
    stop at the first bound/floor violation), pure `undervolt_preflight` (Safe Loop read-only refusal),
    and the windows `run_undervolt_probe` entry (dry-run plan; the `--confirm` branch fails closed).
  - **CLI** (`crates/service/src/main.rs`): new `undervolt-probe` subcommand. DRY-RUN by default; `--confirm`
    is parsed but REFUSED in this patch (no hardware). Flags: `--target-mhz`, `--start-mv`, `--steps`.
- **Fail-closed safety rules (planner)**: reject an empty / foreign / non-sane base curve; reject a non-real
  VF bin (index not on the curve); reject a bin below the hardware floor; reject offset ≤ 0
  (positive-offset-only); reject offset above the absolute cap; reject a per-step delta above the per-step
  cap; reject a planned clock above the conservative clock ceiling. NEVER silently clamp an unsafe offset —
  always return an explicit `Err`, and return the plan BEFORE any write. The offset bounds are CONSTANTS, not
  CLI-widenable, so the operator cannot loosen them.
- **Scope of F2 v1 (deliberately small)**: ONE focus target only; small bounded positive offset only; no
  persistence / apply / promotion; no multi-target loop; no autonomous crash-seeking. In confirmed mode
  (future) it must stop on the first crash / TDR / instability / verifier failure.
- **Unchanged / NOT touched**: `apply_vf_ceiling_monotone`, the validated F1/build-frontier flatten-down
  writer + verifier, Safe Loop, boot flag, `reset_to_stock`, blacklist, last-known-good fallback, power-limit
  / TDP / clock-lock. F2 dry-run reads Safe Loop state READ-ONLY and never mutates it.
- **Not implemented yet (confirmed hardware path)**: arming the boot flag before a positive write; clearing it
  only after a clean dwell + reset; leaving enough state for recovery/blacklist on crash; last-known-good
  fallback; the actual confirmed one-step write+dwell+verify+reset loop. Left as explicit TODO/design comments
  in `gpu_undervolt.rs` — the `--confirm` branch fails closed until that lands.
- **Hardware: BLOCKED.** No confirmed run, no VF write, no profile apply/persist, no Safe Loop mutation in
  this task. Next step: a dry-run review of `undervolt-probe`, THEN (only after review) a first supervised
  one-step confirmed F2 validation.

## F1c bounded-tail — confirmed hardware validation PASS, then tail-richness follow-up (2026-06-20)
- **Confirmed run (2026-06-20) of the bounded tail (`8667bf0`) — verdict PASS.** Command
  `build-frontier --confirm --max-targets 7 --max-probes 45 --max-probes-per-target 3 --safe-start-cap 1075
  --bind-seeking --power-bound-knee-seeking --phase-b-probes 24`. Exit 0; no TDR/crash/reboot; `reset_to_stock`
  ran; `gpu_applied.json`/`boot_flag.json` absent; state files byte-identical (safe_loop.json mtime-only);
  monotone writer `positive_offsets=0` throughout. Phase A collapsed (6 pts pcf 1.0); plateau 1800; Phase B
  focus 1800, **started at 1056 mV (below the 1062 Phase-A floor, skipped 1075/1068/1062)**, crossed the knee
  (pcf 1.000 at 1012 → **0.215 at 1006 mV**), **continued past the first off-cap point to 1000 mV (pcf 0.224),
  captured 2 useful off-cap points**, stopped `KneeTailComplete`. **Synthesis became `differentiated`** (2
  useful / 14 power-bound), no longer `POWER-BOUND COLLAPSE`. The fix works end-to-end.
- **Remaining issue → this follow-up**: the two captured tail points (1006 & 1000 mV) were BOTH ~199 W (the
  knee hugs the power cap here), so Godforge / Brokkr's / Deep Calm all coincided at ~1811 MHz @ 1006 mV /
  199 W — "differentiated" (not collapse) but a THIN frontier. Need MORE useful below-knee points (deeper,
  where power actually drops) to separate the three profiles.
- **Decision (2026-06-20)**: enrich the Phase-B tail. Raise the internal richness bounds
  `PHASE_B_MIN_USEFUL_POINTS` 2 → **4** (decoupled from the synthesis collapse threshold
  `MIN_USEFUL_FRONTIER_POINTS`, which STAYS 2) and `PHASE_B_POST_KNEE_TAIL_BINS` 3 → **5**. Phase B now keeps a
  bounded tail until 4 useful off-cap points OR 5 post-knee bins. Still opt-in / default OFF; no new CLI flag;
  `--phase-b-probes` (24) and global `--max-probes` (45) remain the external bounds. Failure / instability /
  verifier / floor / budget stops keep precedence (checked before the tail logic).
- **Unchanged safety surfaces**: monotone writer, verifier gates, Safe Loop, `reset_to_stock`, floor/cluster
  selection, persistence/knowledge writes, power-limit/TDP/clock-lock. Phase A, synthesis, and bind-seeking
  untouched. The dry-run plan auto-reports the new "≥ 4 useful points or ≤ 5 post-knee bins" tail target.
- **Validation (no hardware)**: see the implementation commit's `cargo check` / `cargo test` results.
- **Hardware**: one confirmed validation authorized for this follow-up to see whether power drops below the
  knee and the three profiles separate (same flags). Non-goals unchanged.

## F1c follow-up — Phase B captures a bounded below-knee TAIL (commit 8667bf0, 2026-06-16) — pure, no hardware
- **Driver: FIRST confirmed knee-seeking hardware run (2026-06-16) — verdict PASS-PARTIAL.** Command
  `build-frontier --confirm --max-targets 7 --max-probes 45 --max-probes-per-target 3 --safe-start-cap 1075
  --bind-seeking --power-bound-knee-seeking --phase-b-probes 24`. Exit 0; no TDR/crash/reboot; `reset_to_stock`
  ran; `gpu_applied.json`/`boot_flag.json` absent; state files byte-identical (safe_loop.json mtime-only);
  monotone writer, `positive_offsets=0` throughout. Phase A collapsed (7 pts pcf 1.0, ~199 W); plateau 1785;
  Phase B focus 1785, **started at 1056 mV (below the 1062 Phase-A floor — skipped 1075/1068/1062)**, descended
  1056→1025, and at **1025 mV pcf dropped 1.000→0.437** (a STEEP knee, crossing 0.95 and 0.50 in one 6 mV bin)
  → `LeftPowerRegime` fired. **Real knee ≈ 1025 mV** (~100 mV higher than the ~930 mV estimate; budget was NOT
  the limiter — reached in 6 bins).
- **Problem found:** Phase B stopped at the FIRST off-cap point, so only **1** useful point was captured →
  synthesis still (correctly) reported `POWER-BOUND COLLAPSE`, Godforge/Brokkr's/Deep Calm best-effort/not
  differentiated. The **stop policy**, not the budget, was the limiter.
- **Decision (2026-06-16):** change `descend_phase_b` to capture a BOUNDED below-knee tail. After the knee
  crossing (first `pcf < POWER_BOUND_FRAC` point — the synthesis off-cap definition), keep descending until
  `PHASE_B_MIN_USEFUL_POINTS` (= `MIN_USEFUL_FRONTIER_POINTS` = 2) useful off-cap points OR
  `PHASE_B_POST_KNEE_TAIL_BINS` (= 3) post-knee bins, then stop cleanly as new
  `BracketStop::KneeTailComplete`. With ≥ 2 useful points the existing synthesis differentiates; with 1 it
  keeps the honest collapse. Replaces the old first-off-cap (`pcf ≤ BIND_CAP_FRAC`) `LeftPowerRegime` stop in
  Phase B (bind-seeking's `LeftPowerRegime` is unchanged).
- **Safety precedence PRESERVED:** crash / abort / global drain / verifier failure / instability are checked
  BEFORE the tail logic and stop immediately — the tail never descends through an unverified or unstable
  probe; hardware floor / `--phase-b-probes` / global `--max-probes` still bound it.
- **Unchanged:** Phase A, synthesis, bind-seeking, and the whole safety chain (monotone writer, verifier
  gates, Safe Loop, `reset_to_stock`, floor/cluster selection, persistence/knowledge writes, power-limit/TDP/
  clock-lock). Opt-in / default OFF; no new CLI flag. Dry-run plan states the bounded-tail policy.
- **Validation (no hardware):** `cargo check -p nidavellir-service` clean (0 warnings); `cargo test`
  **203 passed / 0 failed** (8 new: steep-knee captures a tail not one point; stop-at-enough-useful;
  post-knee-bin bound; verifier-fail / instability / `--phase-b-probes` / global-`--max-probes` / floor each
  stop the tail; 3 updated). No dry-run / `--confirm` / hardware.
- **Hardware: STILL BLOCKED.** Next is a NEW dry-run-only review confirming the bounded-tail plan output,
  before any further confirmed run. Non-goals unchanged: no power-limit/TDP, no clock-lock, no safety-chain
  change, no persistence/apply, no same-config rerun.

## F1c follow-up — Phase B continues BELOW Phase-A floor (commit 9f35ec0, 2026-06-16) — pure, no hardware
- **Decision (2026-06-16)**: act on the dry-run-only review finding. The `0ef4e68` Phase B re-started from
  the cap, so on this card's fine-grained VF curve (~6–7 mV/bin) `--phase-b-probes 12` reached only
  ~1006 mV — it re-probed the inert top bins Phase A already covered (1075/1068/1062) and stopped ~75 mV
  ABOVE the estimated ~930 mV knee. Commit `9f35ec0 fix(service): start power-bound phase b below phase a
  bins`. Scope: `crates/service/src/gpu_power_sweep.rs` (+ in-file tests). Pure — no hardware.
- **What changed**: Phase B now CONTINUES below the deepest bin Phase A retained for the focused target.
  Two pure helpers — `phase_a_deepest_bin(frontier, target)` (the target's deepest retained Phase-A VF bin;
  in the collapse trigger every Phase-A probe is stable so this equals the deepest probed) and
  `phase_b_start_below(descent, floor)` (the highest REAL bin strictly below it) — pick the Phase-B start so
  every probe lands on a new, deeper real bin. Fallbacks: no retained Phase-A point for the target (not
  probed / dropped) → safe-start cap; Phase A already at the hardware floor → Phase B skipped cleanly (no
  deeper bin, no unbounded behavior). Dry-run plan adds a `knee start` line.
- **Unchanged**: Phase A, `descend_phase_b` (its contract is unchanged — only the start voltage the
  orchestrator passes it changed), `synthesize_forge_profiles`, the whole safety chain (monotone writer,
  verifier, Safe Loop, `reset_to_stock`, floor/cluster, persistence, power-limit/TDP/clock-lock). Feature
  stays opt-in / default OFF; global `--max-probes` remains the master cap; no new CLI flag.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean (0 warnings); `cargo test`
  **195 passed / 0 failed** (5 new: `phase_a_deepest_bin`, `phase_b_start_below`, start-below-floor
  integration with probe instrumentation, no-history safe-start fallback, floor-reached clean skip; existing
  collapse test updated — knee now at index 3 since the inert top bins are skipped).
- **Hardware: STILL BLOCKED.** Next step is a NEW dry-run-only review confirming the improved plan output
  (the `knee start` line + a deeper effective reach), before any confirmed validation. Note: budget SIZING
  is still the operator's call — with ~6–7 mV bins, crossing a ~930 mV knee from a ~1062 mV Phase-A floor
  needs ~20+ Phase-B probes; this patch makes each probe count, it does not change the default budget (12).
- **Non-goals (unchanged)**: no power-limit/TDP, no clock-lock, no safety-chain change, no persistence/apply,
  no same-config confirmed rerun.

## F1c power-bound knee-seeking — two-phase prototype IMPLEMENTED (commit 0ef4e68, 2026-06-15) — pure, no hardware
- **Decision (2026-06-15)**: implement the design-audit direction `NEED DEEPER POWER-BOUND DESCENT` as an
  OPT-IN two-phase knee-seeking prototype. Commit `0ef4e68 feat(service): add power-bound knee-seeking
  phase`. Scope: `crates/service/src/gpu_power_sweep.rs` (+ in-file tests) and two CLI flags in
  `main.rs`. Pure change — no hardware, no `--confirm`, no dry-run, no VF write.
- **Why the shallow collapse was NOT terminal proof**: the validated `0996769` run walked only the top
  ~13 mV (first-pass bins `[1075, 1068, 1062]` under `--safe-start-cap 1075 --max-probes-per-target 3`) —
  ~130 mV ABOVE the card's real operating voltage (~1810 MHz draws ~930 mV). `apply_vf_ceiling_monotone`
  only caps bins at voltage ≥ ceiling_mv, so a 1062–1075 mV ceiling was physically INERT (clock pinned at
  the cap regardless of target/bin: 1800@1068→1812, 1830@1062→1814). pcf stayed 1.000 because the descent
  never reached the knee, NOT because no frontier exists. (The prior "not scheduler depth / not per-target
  cap — 1755 went deeper to 1062" reading mistook 1062 mV — the 3rd bin — for "deep"; it is ~100 mV above
  the knee.)
- **What landed (opt-in, default OFF)**:
  - **Phase A** = the existing single-pass `build_frontier` descent — extracted VERBATIM into
    `run_target_descents` so the single-pass path is byte-for-byte unchanged (proven by the unchanged
    `build_frontier` / per-target-cap / warm-start / bind-seeking tests).
  - **Phase B** (only after a Phase-A power-bound collapse): `detect_plateau_clock` (median power-bound
    clock), `select_phase_b_target` (lowest candidate ≥ plateau), `descend_phase_b` (deep descent on ONE
    focused target, recording the FULL trajectory, descending THROUGH the knee and stopping cleanly at
    `pcf ≤ BIND_CAP_FRAC` / budget / floor / failure), `detect_power_bound_knee` (first pcf crossing below
    `POWER_BOUND_FRAC`). Merge Phase A + Phase B and re-synthesize via the EXISTING
    `synthesize_forge_profiles`: a crossed knee differentiates (Godforge = highest sustained off-cap clock);
    no knee preserves the honest `PowerBoundCollapse`.
  - **Flags**: `--power-bound-knee-seeking` (valueless, default OFF) + `--phase-b-probes N` (default None →
    `FRONTIER_PHASE_B_PROBES = 12`). Global `--max-probes` stays the MASTER cap; `--phase-b-probes` only
    bounds the focused descent depth; `--phase-b-probes 0` fails closed. Dry-run plan prints the mode.
- **Profile semantics (prototype)**: Godforge becomes the knee region (highest sustained off-cap clock),
  NOT the highest requested clock; Brokkr's / Deep Calm come from the below-knee tail via the existing
  policy. Full profile-policy refinement (knee-margin, multi-target tails) is deferred — documented in
  comments/tests, not overfit here.
- **Unchanged safety surfaces** (diff audited — no protected symbol modified): monotone static-base writer,
  verifier gates, Safe Loop, `reset_to_stock` (still runs after every build, both paths), hardware-derived
  floor / cluster selection, per-target cap, warm-start default OFF, profile persistence / knowledge writes,
  power-limit / TDP / clock-lock.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean (0 warnings); `cargo test -p
  nidavellir-service` **190 passed / 0 failed** (new F1c: plateau median, target selection + fallback, knee
  transition/detection, Phase-B crosses-knee/budget-bounded/stays-saturated, two-phase OFF==single-pass,
  collapse→differentiates, no-knee→honest-collapse, differentiated-skips-Phase-B, global-cap bounds both
  phases, `--phase-b-probes` fail-closed, plan lines, CLI parse). No dry-run / `--confirm` / hardware.
- **Hardware: STILL BLOCKED.** Next step is a SEPARATE dry-run-only review of the new Phase-B plan output
  (no `--confirm`). A confirmed run is justified only after that review and must be a bounded knee-seeking
  shape (one focused target descended deep past ~930 mV), NOT a same-config rerun of the known collapse.
- **Non-goals (unchanged)**: no power-limit / TDP change; no clock-lock; no safety-chain removal; no
  profile persistence/apply; no same-config confirmed rerun; no broad adaptive target regeneration yet.
- **Scope**: code + tests in `gpu_power_sweep.rs`, CLI flags in `main.rs`. No version bump.

## F1b power-bound collapse classification — FIRST confirmed hardware validation (commit 0996769, 2026-06-15) — PASS
- **Validation (2026-06-15)**: one supervised confirmed run of `0996769` (docs `4880153`), operator present;
  worktree HEAD = `origin/master` = `4880153`, tree clean. Fresh worktree-local binary (built after `0996769`,
  not the stale main-repo target). Confirming dry-run gate passed first (regime-only wording, threshold
  `power_capped_frac <= 0.50`, clock arm retired, warm-start OFF, no-op safety line). Command:
  `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075
  --bind-seeking`. **Exit 0; ~5.7 min.**
- **Safety: PASS.** No TDR / crash / driver reset / black-screen / reboot. `reset_to_stock` ran ("GPU restored
  to stock; no profile applied or persisted"); GPU back at stock/idle. After: `gpu_applied.json` /
  `boot_flag.json` absent; `safe_loop.json` stayed idle/disarmed (`safe_mode:false`, no new crash/blacklist
  entry; only mtime touched by startup recovery); `forge_state.json` / `gpu_knowledge.json` / `heartbeat.txt`
  byte-unchanged; working tree clean. Every probe `write_mode=monotone_static`, `positive_offsets=0`; no
  overshoot veto.
- **Run mechanics**: 19 probes / 17 measured dwells; global `--max-probes 21` not exhausted; **6 of 7 targets
  characterized**. Target **1920 dropped** on a benign verifier `LiveMismatch` at the start bin (verifier path
  worked, no crash, run-variance — not patch-related); target **1890** hit a later `LiveMismatch` and kept its
  deepest verified bin. Targets descended to 1062/1068 mV. All dwells **PowerLimited**, `power_capped_frac=1.000`,
  ~199 W, clocks ~1784–1825 MHz.
- **Algorithm/reporting honesty: PASS.** No `BoundBinding` wording, no `reason=Clock` anywhere. **Clock-arm
  retirement validated**: probes whose avg clock sat within 30 MHz of target (e.g. 1800 @ 1068 → avg 1812;
  1830 @ 1062 → avg 1814) would have FALSE-bound under the retired v2 Clock arm — they correctly did **not**
  bind and descended to `PerTargetCap`. **`LeftPowerRegime` validated negatively**: evaluated on every eligible
  probe, correctly returned `bound=false reason=None`, and **no** target stopped by `LeftPowerRegime` (none had
  `power_capped_frac <= 0.50`). **`PowerBound`/`PowerBoundCollapse` validated positively**: all 6 retained
  points marked `[power-bound]`; output reported `6 power-bound / 0 useful`; explicit diagnostic
  *"power-bound collapse — cannot build a differentiated VF frontier under this workload/regime"*; frontier
  classes = `POWER-BOUND COLLAPSE (best-effort, NOT a differentiated VF frontier)`. Synthesis collapsed
  Godforge / Brokkr's / Deep Calm to the SAME best-effort point (1815 MHz / 199 W, R=0.00), confidence stayed
  0.21, all explicitly flagged not-differentiated — **no fake differentiated frontier presented.**
- **Verdict: PASS** (safety/mechanics PASS, reporting honesty PASS). The physical frontier is still not useful
  under this workload/regime because the RTX 3060 Ti is pinned at the ~199 W power cap — which the tool now
  reports honestly instead of fabricating profiles.
- **Caveats**: `LeftPowerRegime` was validated **negatively only** (it did not false-fire under pcf=1.000); a
  positive `LeftPowerRegime` stop still needs a workload/target regime where pcf drops ≤ 0.50. The 1920
  `LiveMismatch` is benign run-variance, not patch-related. No useful frontier diversity appeared.
- **Decision / next direction**: **accept the patch behavior; keep hardware BLOCKED for this same
  configuration.** Do NOT repeat the same confirmed run; do NOT increase the per-target cap as the next step;
  do NOT tune power limit / TDP / clock lock yet. The next move is a **design decision**, one of: (a) a workload
  that does not saturate the ~199 W cap; (b) candidate-target generation below the observed power-bound plateau;
  or (c) a dedicated design pass for how Nidavellir should present "cannot differentiate under this
  workload/regime."
- **Scope**: docs/continuity only. One dry-run + one confirmed run; no code/test change; no further hardware.

## F1b power-bound collapse classification — IMPLEMENTED (commit 0996769, 2026-06-15) — pure, no hardware
- **Decision (2026-06-15)**: implement the first post-audit simplification (the SIMPLIFY direction recorded
  below). Commit `0996769 fix(service): classify power-bound frontier collapse`. Scope:
  `crates/service/src/gpu_power_sweep.rs` ONLY. Pure change — no hardware, no `--confirm`, no dry-run.
- **What changed**:
  - **Retired bind-seeking's Clock arm.** `classify_binding` is now regime-only: a target binds (stops early)
    ONLY when it has LEFT the power-limited regime (valid `power_capped_frac <= BIND_CAP_FRAC = 0.50`). The
    clock-near-target arm false-binds on a power-bound card (the cap, not the descent, sets the achieved
    clock). Removed `BIND_OVERSHOOT_MHZ`, `BindThresholds.overshoot_mhz`, `BindReason::Clock`; the start-bin
    eligibility guard is retained.
  - **Renamed `BracketStop::BoundBinding` → `LeftPowerRegime`** — the early stop now has one honest meaning.
  - **First-class power-bound classification** (`POWER_BOUND_FRAC = 0.95`): pure `is_power_bound_frac` /
    `is_power_bound_point` / `useful_frontier_points` / `frontier_power_bound_collapse`. A stable, verified,
    pcf-saturated dwell is a VALID raw bracket but NOT a useful clock-frontier point. Missing/invalid pcf is
    NOT marked power-bound (fail open for classification) yet still fails CLOSED for regime binding.
  - **Collapse-aware synthesis**: `synthesize_forge_profiles` excludes power-bound points from differentiated
    selection; with < 2 useful (non-power-bound) points it returns a FLAGGED best-effort and logs
    *"power-bound collapse — cannot build a differentiated VF frontier under this workload/regime"* (new
    `ForgeProfiles.power_bound_excluded` / `power_bound_collapse`). Keys on pcf saturation, so the jittery
    ~1798–1819 MHz @ pcf 1.0 plateau is caught where exact-distinct-clock detection missed it. With NO
    power-bound points the legacy path is byte-for-byte unchanged.
  - **Reporting**: `run_build_frontier` RESULT now prints per-point `pcf` (+ a `[power-bound]` tag) and a
    `frontier classes : N useful / M power-bound — synthesis differentiated|POWER-BOUND COLLAPSE` summary.
- **Unchanged safety surfaces** (diff audited — no protected symbol added/removed): monotone static-base
  writer, verifier gates, Safe Loop, `reset_to_stock`, hardware-derived floor / cluster selection, per-target
  cap, warm-start default OFF, profile persistence / knowledge writes, power-limit / clock-lock.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  **173 passed / 0 failed** (new: power-bound threshold + invalid; all-power-bound collapse incl. jittery
  1798/1811/1819; collapse flagged + best-effort; mixed frontier synthesizes from useful; legacy path
  unchanged; clock-near-target does NOT bind under saturation; regime binds at pcf ≤ 0.50; invalid pcf fails
  closed). No dry-run / `--confirm` / hardware.
- **Hardware: STILL BLOCKED.** Pure code/test patch. A confirmed run is justified only AFTER reviewing the new
  classification/reporting behavior in a fresh dry-run; the same-config rerun remains not recommended.

## build-frontier / F1b algorithm audit — verdict SIMPLIFY (2026-06-15, read-only, pre-implementation)
- **Decision**: record the conclusion of a read-only algorithm audit of the F1b build-frontier curve
  exploration BEFORE writing any code, so the next patch has a clear north star. **No code, no tests, no
  hardware, no `--confirm` were run** for this audit — inspection of `crates/service/src/gpu_power_sweep.rs`
  and the continuity docs only. Verdict: **SIMPLIFY CURRENT DIRECTION** (not a redesign, not a full rollback).
- **North star reaffirmed** (the intended simple dwell-descent the code should keep implementing):
  1. find the relevant max core clock / top sustainable target;
  2. start at the max safe voltage / safe VF ceiling;
  3. dwell trying to hold the target core clock;
  4. descend the real VF voltage bins while sustainable;
  5. stop each target on an **explicit stop reason** — unstable, verifier failure, crash/abort/budget drain,
     voltage floor, per-target cap, **or power-bound regime/collapse**;
  6. move to the next lower target; 7. build a real stable frontier;
  8. synthesize Godforge / Brokkr's Best / Deep Calm **only from meaningful (non-power-bound) frontier data**.
- **The skeleton is still valid.** Discovery → descent → synthesis matches the north star. The drift is
  concentrated in **bind-seeking / `BoundBinding`** semantics.
- **Load-bearing — KEEP (do not touch)**: hardware-derived floor; cluster selection / sane-core VF filtering
  (`derive_core_seed`, `select_core_cluster`, `sane_core_points`); real-bin descent (`derive_descent`,
  `descend_target`); per-target probe cap; the typed hard/soft stops (`SoftUnverified`/`SoftUnstable`/
  `HardFailure`/`Aborted`/`BudgetExhausted`/`CleanFloor`/`PerTargetCap` — they gate B2 fallback, carry-forward
  eligibility, and abort); the confidence gate + best-effort fallback structure in `synthesize_forge_profiles`;
  monotone static-base writer; verifier gates; Safe Loop; `reset_to_stock`; no profile persistence/knowledge
  write during build-frontier.
- **Conclusion on bind-seeking**: `BoundBinding` is the **wrong combined abstraction**. It mixes (a) a **bad
  Clock arm** that *false-binds* under power cap — on a power-pinned card the cap, not the voltage descent,
  sets the achieved clock, so "avg clock within 30 MHz of target" is an illusion — with (b) a **useful Regime
  arm** (`power_capped_frac <= 0.5`) that legitimately indicates the card left power-limited behavior. The v2
  start-bin guard was useful and validated (it killed the v1 start-bin false bind) but it did **not** solve the
  physical frontier collapse.
- **Evidence (confirmed v2 hardware run, commit `bf02971`, 2026-06-15)**: all dwells stayed power-limited —
  `power_capped_frac=1.000`, ~199 W flat, achieved clocks ~1798–1819 MHz, synthesis confidence 0.21, profiles
  collapsed to one ~1800 MHz / 199 W point. The Clock arm bound (`reason=Clock`); the Regime arm never fired
  (pcf saturated). Therefore the remaining issue is **regime / power-bound collapse, not scheduler depth and
  not per-target probe count** (1755 descended deeper to 1062 mV and still produced ~1811 MHz / 199 W).
- **Decisions to act on (next patch)**:
  - Stop treating a `Clock` bind as sufficient evidence of useful VF-bound behavior when `power_capped_frac`
    is saturated; **retire or neutralize bind-seeking's Clock arm** (vetoing it under pcf saturation is the
    minimal step; deleting it — and with it `bind_eligible`, the overshoot threshold, and the v1/v2 split — is
    the simpler/cleaner one).
  - **Keep the useful regime signal**, reclassified as something honest like **`LeftPowerRegime`** (a clean,
    informative early stop: the descent reached a voltage where the card is no longer power-pinned — a real VF
    point).
  - Add a first-class **`PowerBound` / `PowerLimitedPlateau` / `PowerBoundCollapse`** classification (north-star
    stop 5e, currently unnamed).
  - **Strengthen `synthesize_forge_profiles`**: power-bound samples are **valid raw brackets but NOT useful
    clock-frontier diversity**. Today the collapse detector keys on exact-distinct clocks, so a jittery
    ~1798–1819 MHz plateau reads as ~6 "distinct" clocks and the warning never fires — synthesis silently emits
    a falsely-differentiated frontier even with bind-seeking OFF. Re-key collapse detection on **pcf
    saturation** and emit an explicit diagnostic: *"power-bound collapse — cannot build a differentiated VF
    frontier under this workload/regime."*
- **Power-limited sample treatment**: valid bracket = **yes**; useful clock-frontier point = **no** when
  `power_capped_frac` is saturated; synthesis = **raw input yes, but excluded from differentiated profile
  selection**; collapse diagnostic = **yes, the primary signal**. Mark, don't discard (the point already
  carries `power_capped_frac`, and it remains a valid warm-start seed).
- **Next safest patch (pure / mostly pure, `crates/service/src/gpu_power_sweep.rs`)**: add/rename the stop
  classifications (`VoltageFloor`, `DepthCap`, `LeftPowerRegime`, `Unstable`, `VerifyFailed`, `Crashed`,
  `Aborted`, `BudgetDrained`, `PowerBound`/`PowerBoundCollapse`); strengthen `synthesize_forge_profiles`
  (detect the pcf-saturated plateau, don't treat jittery ~1800 MHz clocks as a real differentiated frontier,
  emit the explicit diagnostic); add unit tests over synthetic samples; **do not touch the hardware-writing
  path.** Optionally add read-only power-headroom telemetry.
- **Explicit non-goals**: no confirmed hardware run; no power-limit/TDP changes; no clock-lock changes; no
  target-generation redesign yet; no warm-start default change; no per-target cap change; no Safe Loop / reset
  / writer / verifier changes; no profile persistence / knowledge write change; no version bump.
- **Hardware**: **blocked** until the power-bound classification + collapse report land and a fresh dry-run
  shows the new diagnostics. Re-running the proven-uninformative config is not justified. See `handoff.md`
  (continuity) and `memory.md` (index).
- **Scope of this entry**: docs/continuity only (`decisions.md`, `handoff.md`, `memory.md`). No code/test/
  hardware command was run.

## bind-seeking F1b v2 strictness — FIRST confirmed hardware validation (commit bf02971) — mechanism PASS, frontier PARTIAL
- **Validation (2026-06-15)**: `bf02971 fix(service): tighten bind-seeking stop criteria` was confirmed on
  real hardware by a supervised run — operator present; docs at `3b8774c` (HEAD/origin/master = `3b8774c`,
  working tree clean). A **fresh worktree binary was built first** because the worktree-local
  `target/debug/nidavellir-service.exe` was absent and the only existing binary was stale (main-repo
  `target/debug`, built 2026-06-07, predating the bind-seeking feature): `cargo build -p nidavellir-service`
  → worktree binary created after the build (mtime after the marker, distinct size); the stale main-repo
  binary was **not** used; the working tree stayed clean.
- **Dry-run gate passed** (no `--confirm`): bind-seeking ENABLED; v2 strict start-bin-not-eligible note;
  thresholds `avg_clock_overshoot <= 30 MHz` + `power_capped_frac <= 0.50`; coverage-bounded scheduler;
  `max_probes=Some(21)`; `max_probes_per_target=Some(3)`; targets `[1935,1905,1875,1845,1815,1785,1755]`;
  first-pass bins `[1075,1068,1062]`; warm-start OFF; no applied-profile warning; no Safe Loop conflict
  warning; dry-run no-op line (no Safe Loop arm / apply / dwell / VF write). Confirmed:
  `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3 --safe-start-cap 1075
  --bind-seeking`.
- **Safety: PASS.** Exit 0; no TDR / driver reset / black-screen / reboot / crash. Startup recovery clean;
  `reset_to_stock` ran ("GPU restored to stock; no profile applied or persisted"). After:
  `boot_flag.json`/`gpu_applied.json` absent; `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt`
  unchanged; `safe_loop.json` idle (`safe_mode:false`), size unchanged, mtime touched by startup recovery
  only. GPU back at stock idle.
- **Probe result**: 15 probes/dwells; all 7 targets characterized. 6 stopped via **`BoundBinding`**
  (1935/1905/1875/1845/1815/1785, `probes_used=2`); 1 via **`PerTargetCap`** (1755, `probes_used=3`); none
  dropped; global `--max-probes 21` not exhausted (15/21); no `overshoot_veto`; every probe
  `write_mode=monotone_static`, `positive_offsets=0`.
- **v2 mechanism — PASS (start-bin guard).** Every 1075 mV start bin reported `eligible=false / bound=false`;
  all 7 descended to 1068 mV, 1755 to 1062 mV; earliest binding only after a real bin descent (the 6 bound at
  bin 1068, `reason=Clock`). Bind telemetry present per probe (eligible / bound / reason / avg_clock_mhz /
  p5_clock_mhz / power_capped_frac). The regime arm never fired (pcf saturated at 1.000) — binding came
  solely via the avg-clock path.
- **Physical frontier — PARTIAL (did NOT de-collapse).** All dwells PowerLimited, `power_capped_frac=1.000`
  throughout (199 W flat); achieved clocks clustered ~1798–1819 MHz, all converging to the same power-bound
  point; synthesis confidence stayed 0.21 (R=0.00); Godforge/Brokkr's/Deep Calm collapsed to ~1800 MHz/199 W.
- **Conclusion**: v2 fixed the **procedural** start-bin binding bug; the **remaining collapse is
  power/regime-related, not scheduler depth and not the per-target cap** (1755 went deeper to 1062 and still
  produced ~1811 MHz / 199 W). **Do NOT repeat the same run; do NOT bump the per-target cap as the immediate
  next action; do NOT jump straight to risky power-limit / clock-lock changes.**
- **Direction (next design, analysis first)**: add/adjust **regime-aware binding semantics**; distinguish a
  true `Clock` bind from a `PowerLimitedPlateau` / `PowerBoundCollapse`; consider **vetoing `Clock` binding
  when `power_capped_frac` is saturated near 1.0**; add explicit collapse diagnostics + power-headroom /
  power-drop telemetry. **Stop for analysis before any further confirmed hardware run.**
- **Scope**: docs/continuity only (`handoff.md`, `memory.md`, this file). One debug build + one dry-run + one
  confirmed run; no code/test change; no further hardware command.

## bind-seeking F1b v2 strictness — IMPLEMENTED + pushed (commit bf02971), hardware-validated (see entry above)
- **Decision (2026-06-15)**: tighten bind-seeking after its first supervised hardware run. Commit
  `bf02971 fix(service): tighten bind-seeking stop criteria`, pushed to `origin/master`. Scope:
  `crates/service/src/gpu_power_sweep.rs` only.
- **Why**: the v1 run was **safety/mechanics PASS but semantic PARTIAL** — v1 allowed `BoundBinding` on the
  first/start bin (1075 mV), so every viable target stopped immediately, no descent happened, and the frontier
  stayed degenerate (single-bin, ~1075 mV / ~199 W; Forge confidence ~0.21). The binding test was too
  permissive, and the p5/sustained clock signal could call a target "binding" while the average/achieved clock
  was still materially above target.
- **What changed**:
  - **Eligibility**: the start bin is never bind-eligible; a target must descend ≥1 real VF bin first
    (earliest bind = 2nd probed bin). `classify_binding` now takes an `eligible` flag and returns a
    `BindDecision`; eligibility computed by `bind_eligible(probes_before, cur_bin, start_bin)`.
  - **Clock metric**: clock binding keys off the AVERAGE/achieved clock (`avg - target <= 30`), not
    p5/sustained; p5 stays telemetry/reporting; absent/zero avg fails closed.
  - **Regime arm**: `power_capped_frac <= 0.5` kept; invalid/missing cap_frac fails closed (`valid_cap_frac`:
    NaN / <0 / >1 → no regime binding).
  - **Telemetry**: new `BindReason` + `BindDecision`; per-probe live log of eligible / bound / reason /
    avg_clock_mhz / p5_clock_mhz / power_capped_frac; dry-run reports the start-bin-not-eligible caveat.
- **Rejected/deferred (unchanged from v1)**: power-drop stop-condition still NOT a stopping rule; no
  power-limit / clock-lock changes; per-target cap semantics untouched.
- **Precedence preserved**: crash → abort → budget drain → verifier failure → dwell instability → binding →
  per-target cap → floor (only the binding arm is now gated by eligibility).
- **Safety unchanged**: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  persistence/profile apply, hardware-floor derivation, warm-start default OFF.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  169 passed; dry-run only passed (`--max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`); no hardware boundary crossed.
- **Status**: hardware-validated 2026-06-15 (see the FIRST confirmed hardware validation entry above) —
  mechanism PASS (start-bin guard), frontier PARTIAL (still power-limited / collapsed); next design work
  is regime-aware binding, not a re-run.

## bind-seeking F1b v1 — IMPLEMENTED + pushed (commit 08f745e), hardware-validated PARTIAL → superseded by v2
- **Decision (2026-06-14)**: implement the bind-seeking direction from the `5248758` run as an **opt-in,
  default-OFF** scheduler mode. Commit `08f745e feat(service): add opt-in bind-seeking to build-frontier`,
  pushed to `origin/master`. Scope: `crates/service/src/gpu_power_sweep.rs` + `crates/service/src/main.rs`.
- **Why**: `--max-probes-per-target` (`5248758`) fixed budget *distribution* but not *binding/differentiation*
  — shallow near-stock bins (1075/1068 mV) are non-binding on the power-capped 3060 Ti, so profiles collapsed
  to one point. Walking a fixed bin count cannot tell a useful (binding) point from a non-binding one. The
  fix: per target, **keep descending while stable but non-binding; stop at the first BINDING point**.
- **Binding signal v1 — Clock + regime** (`classify_binding`, pure): a verified + dwell-stable probe binds iff
  EITHER `sustained - target <= BIND_OVERSHOOT_MHZ (30)` (sustained = p5 else avg) OR `power_capped_frac <=
  BIND_CAP_FRAC (0.5)`. Constants `BIND_OVERSHOOT_MHZ=30`, `BIND_CAP_FRAC=0.5`.
- **Rejected for v1 — power-drop stop-condition**: deliberately excluded; it needs top-power reference
  tracking and adds state/risk. Clock + regime is sufficient and self-contained. Power-drop may be logged as
  telemetry later, but is NOT a stop rule in v1. (Alternatives considered earlier: log-only classifier — too
  weak; redefining the per-target cap — overloads validated semantics; power-limit/clock-lock — out of scope.)
- **Scheduler**: new `BracketStop::BoundBinding` — CLEAN (`is_hard_failed()==false`), carry-forward eligible
  when it has a `lowest_verified_mv`. Binding is evaluated ONLY on a verified+stable sample, AFTER every
  failure arm, so precedence is unchanged: crash/hard-failure → aborted → global budget drained →
  verifier-failure/unverified → dwell-unstable/silent-error → **binding** → per-target cap / floor.
- **Interactions / invariants**: `--max-probes` remains the hard global cap; `--max-probes-per-target` remains
  the per-target attempt/depth cap (bind-seeking can stop earlier); **warm-start remains default OFF**. Safety
  boundaries unchanged: monotone static-base writer, verifier gates, Safe Loop, `reset_to_stock`,
  hardware-derived floor, persistence/profile apply — none touched.
- **Validation (no hardware)**: `cargo check -p nidavellir-service` clean; `cargo test -p nidavellir-service`
  **165 passed / 0 failed** (classifier boundaries, failure precedence, BoundBinding carry-forward, dry-run
  reporting). Dry-run only (no `--confirm`): `--max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking` → exit 0, `bind-seeking: ENABLED`, thresholds + caveat printed,
  warm-start OFF, no Safe Loop arm / apply / dwell / VF write.
- **Hardware validation: NOT yet done for `08f745e`.** Next step (separate, operator-present): clean
  confirming dry-run, then `build-frontier --confirm --max-targets 7 --max-probes 21 --max-probes-per-target 3
  --safe-start-cap 1075 --bind-seeking`. **No hardware commands were run in this implementation or docs pass.**
- **Scope of this entry**: docs/continuity only (`handoff.md`, `memory.md`, this file).

## F1b `--max-probes-per-target` — FIRST confirmed hardware validation (commit 5248758) — coverage PASS, profile PARTIAL
- **Validation** (2026-06-13): the per-target probe cap (`5248758 feat(service): add per-target probe cap to
  build-frontier`) was confirmed on real hardware by a supervised run — operator present, after a clean
  confirming dry-run with no plan drift (HEAD/origin/master `5248758`; `47f39be`/`f90981d`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 14 --max-probes-per-target 2 --safe-start-cap 1075`
  — **warm-start OFF**.
- **Safety: PASS.** Exit 0; ~4 min; no TDR / driver reset / black-screen / reboot / crash. Startup recovery
  clean; Safe Loop armed→cleared **per probe** (idle); `reset_to_stock` ran. No persistence:
  `boot_flag.json`/`gpu_applied.json` absent before AND after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged; `safe_loop.json` content/size unchanged (idle, `safe_mode` false), mtime touched
  only, **no new blacklist/crash entry**. GPU back at stock idle.
- **Coverage: PASS (the fix works).** 13 dwells across **all 7 targets** (vs the prior 34-on-1935 depth-first
  run). 6 targets stopped via **`PerTargetCap`** (`probes_used=2`, bins 1075 + 1068 mV); global
  `--max-probes 14` **not exhausted** (13 used). The cap prevented one target from draining the budget — the
  precise goal of Option B.
- **1905 dropped** at probe 1: `LiveMismatch`, `overshoot_veto=true`, `eff_cov=0.963` — conservative verifier
  rejection (neighbors 1935 `NoDownCapNeeded` and 1875 `VerifiedCurve` passed), not a fault. Every probe
  `write_mode=monotone_static`, `positive_offsets=0`; verdicts `NoDownCapNeededCeiling` (1935) +
  `VerifiedCurve` (1875–1755). No writer/verifier/Safe-Loop/reset/persistence regression. Shallow band only
  (1075/1068 mV); did not touch 875/868/862/856/850.
- **Profile goal: PARTIAL.** Achieved clocks clustered **1832–1867 MHz**, power **194–199 W**; lower targets
  did not yield distinct clock/power. Live plateau ~1890 MHz, `overshoot` grew 1875:+30 → 1755:+135 → the
  near-stock flatten does not govern the achieved clock. Godforge/Brokkr's/Deep Calm collapsed to
  **1860 MHz / 194 W** (target 1755); FORGE confidence 0.21 (single-trial → best-effort).
- **Key conclusion**: **shallow near-stock coverage at 1075/1068 mV is non-binding on this hard power-capped
  RTX 3060 Ti.** `--max-probes-per-target` solved budget *distribution*, not *binding/differentiation*. The
  collapse cause is the power-limited / non-binding-ceiling regime (consistent with the prior bin-based-floor
  run), not the scheduler.
- **Direction — bind-seeking F1b (next design):** do NOT repeat these flags, do NOT bump the per-target cap to
  3, do NOT enable warm-start, do NOT jump to power-limit/clock-lock changes as the immediate next step.
  Instead, per target: **continue descending while the point is stable but non-binding; stop when it actually
  BINDS, fails the verifier/dwell, or hits the global/per-target cap.** Goal = first useful (binding) point
  per target, not deepest voltage. **No further hardware commands were run.**
- **Scope**: docs/continuity only (`handoff.md`, `memory.md`, this file). No code/test/IPC/hardware change in
  this pass.

## Build-frontier bin-based floor — FIRST confirmed hardware validation (commit f90981d) — PASS (safe), partial characterization
- **Validation** (2026-06-13): the hardware-derived / bin-based descent floor (`f90981d`) was confirmed on
  real hardware by a supervised, bounded run — operator present, after a clean dry-run on a fresh debug
  build (HEAD/origin/master include `c99dbf1`+`f90981d`; `23b70c4`/`8503182` present;
  `gpu_applied.json`/`boot_flag.json` absent; `safe_loop.json` idle/`safe_mode:false`):
  `build-frontier --confirm --max-targets 7 --max-probes 34 --safe-start-cap 1075 --warm-start-brackets`.
  `--max-probes 34` was chosen so the descent reaches **868 mV** (one real bin below the old 875 mV floor)
  but stops BEFORE **862 mV** — a historical reboot-zone / blacklisted bin.
- **Safety: PASS.** Exit 0; no TDR / driver reset / black-screen / reboot / crash. Startup recovery clean;
  Safe Loop armed → cleared (idle); `reset_to_stock` ran ("GPU restored to stock; no profile applied or
  persisted"). No persistence: `boot_flag.json`/`gpu_applied.json` absent before AND after;
  `forge_state.json`/`gpu_knowledge.json`/`heartbeat.txt` unchanged; `safe_loop.json` byte-identical
  (idle, `safe_mode` false, size unchanged) — mtime touched at run start only, **no new blacklist/crash
  entry**. GPU back at stock idle.
- **Coverage**: 34 hardware dwells, **all spent on target 1935** (ceilings 1075→868 mV). Reached 875 and
  868 mV; **did not reach 862 mV** (no `ceiling_mv=862` line — the 35th scheduler step hit
  `BudgetExhausted` before any write/dwell). Targets 1905/1875/1845/1815/1785/1755 were NOT physically
  characterized (budget exhausted on the hardest target). Warm-start carry-forward observed (B1: 1935 from
  cap, `warm_started=false`; B2: 1905 inherited `start_mv=893` = 868 + 25 mV). Every probe
  `write_mode=monotone_static`, `positive_offsets=0`, `down_caps=0`, no `overshoot_veto`, all
  `NoDownCapNeededCeiling`, `eff_cov=1.000`.
- **Key interpretation / limit of the result**: this proves it is **safe to WRITE the static VF ceiling
  down to 868 mV and descend the bin sequence** — it does **NOT** prove core stability when the core is
  forced to RUN at 868 mV. The card stayed **power-limited (~198 W)** the whole descent, so the ceiling
  was **non-binding** (NoDownCapNeededCeiling everywhere; the power-governed operating point sat at/below
  each ceiling). Frontier point 1935 → **1839 MHz @ 868 mV vf_bin / 198 W**. **PASS for the first
  bin-based floor validation; partial/insufficient for profile synthesis** (single sustainable clock
  1800 MHz → FORGE confidence 0.21, profiles collapse identical). The historical 862/855 mV reboot-zone
  blacklist is offset-keyed (offsets 255/300/330; freq 1755 @ 862) — a DIFFERENT regime from this
  pure-ceiling, zero-offset, power-limited descent.
- **Direction**: do NOT jump to `--max-probes 40`. `--max-probes 35` could deliberately touch 862 mV for
  pure reboot-zone boundary mapping (operator present; the 862 blacklist entry keyed `freq=1755` would not
  match a 1935-target ceiling → Safe Loop is the backstop, not prevention), but it does not produce useful
  profiles. **Primary next step: pivot to F1b / multi-clock characterization and/or make the ceiling
  actually BIND (e.g. raise the power limit) before descending deeper** — deeper ceilings on a
  power-limited card add reboot-zone exposure for ~zero gain. **No further hardware commands were run.**
- **Scope**: docs/continuity only (`handoff.md`, `memory.md`, this file). No code/test/IPC/hardware change
  in this pass.

## Build-frontier floor is hardware-derived / bin-based — shipped (commit f90981d)
- **Decision** (2026-06-13): remove the hardcoded active **875 mV** descent floor from `build-frontier`.
  The lower bound is now **discovered from the GPU's real VF / core-cluster voltage bins** — the lowest
  real graphics-core bin (`seed.cluster_v_min_mv`), not a fixed constant. `FRONTIER_LOWEST_SAFE_MV` is
  deleted from active code; no replacement fixed floor (no 825/800).
- **Bin-based descent**: `FrontierDescent` carries `bins_desc` (real descending VF bins);
  `derive_descent` builds it from `CoreSeed.cluster_bins_mv`; `descend_target` walks **real bins only**
  and never invents 25 mV requested voltages outside the curve. Warm-start maps its margin to the
  **conservative real bin at or above** the requested margin target, and **never starts below the
  previous `lowest_verified_mv`** (B1). `--max-probes` remains the global exposure cap;
  `--warm-start-brackets` stays default OFF; no new CLI flag.
- **Fail-closed**: an empty/underivable bin domain aborts **before any hardware write** (no fallback to
  a fixed floor on a confirmed run). Dry-run now prints the hardware-derived floor, the exact descent
  bin sequence, the real bin count, and the worst-case dwell count.
- **Scope**: only `crates/service/src/gpu_power_sweep.rs`. **Unchanged**: monotone static-base VF
  writer, verifier gates, Safe Loop, `reset_to_stock`, persistence, profile apply. `cargo check` clean;
  `cargo test -p nidavellir-service` 142 passed. Pushed to `origin/master`.
- **Historical validity**: prior `1755 @ 875 mV` validations (NoDownCapNeededCeiling) remain valid for
  that point; they are **not** an active floor. Future runs may descend **below 875** where real bins
  exist and `--max-probes` allows.
- **Operational warning**: **no hardware run of `f90981d` yet.** The descent may now go **below the
  historical ~855 mV reboot zone**. First real runs MUST be bounded (`--safe-start-cap` / `--max-probes`),
  the operator present and able to reboot, and the dry-run hardware floor + bin sequence reviewed before
  any `--confirm`.

## F1b warm-start voltage-bracket carry-forward — shipped + hardware-validated (commits 23b70c4, 6f2f061)
- **Decision** (2026-06-13): ship a **generic** warm-start voltage-bracket carry-forward scheduler
  primitive for ordered hardest→easiest core-clock voltage descents, behind an opt-in CLI flag
  **`--warm-start-brackets` (default OFF)**. An easier target reuses the previous harder target's
  verified + dwell-stable bracket as its descent start (`lowest_verified_mv + 1 step`), skipping
  dominated high-voltage probes. NOT Godforge-specific; first adopter is build-frontier/F1b. Commit
  `23b70c4 feat(service): add warm-start bracket carry-forward` on `origin/master`.
- **Safety constraints (verified by unit tests AND live logs):**
  - **B1** — seed only from a verified (`VerifiedCurve`/`StockEquivalentCeiling`/
    `NoDownCapNeededCeiling`) AND dwell-stable bin; never from verify-only-unstable, unverified,
    crash/abort, or budget drain; never start below the previous `lowest_verified_mv`.
  - **B2** — the verifier/ceiling axis is NOT monotone in clock: a warm-started first probe that
    fails apply/verify falls back ONCE to `safe_start_cap` and descends normally; never on
    drain/crash/abort; never recurses; the target is not dropped.
  - **B3** — every target gets ≥1 verified-or-fallback-exhausted probe; never dropped solely because
    the inherited warm-start probe failed verify/apply.
- **Hardware validation (2026-06-13) — PASS.**
  `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075 --warm-start-brackets`:
  exit 0; no TDR/reboot; Safe Loop armed/cleared; `reset_to_stock` ran; no persistence
  (`boot_flag.json`/`gpu_applied.json` absent after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged); GPU back at stock idle. **33 probes**, all 7 targets produced points.
  **B2 exercised**: 1905 inherited an optimistic 900 mV start from 1935's boost-top NoDownCapNeeded
  bracket, failed verify (`LiveMismatch`, `overshoot_veto=true`), fell back once to cap 1075, target
  preserved. Frontier preserved — `1755 @ 900` (`NoDownCapNeededCeiling`, plateau 1665..1755,
  overshoot 0) and `1755 @ 875` (`NoDownCapNeededCeiling`, plateau 1620..1755, overshoot 0,
  ≈1755 MHz @ 875 mV ≈176 W) re-validated; every probe `write_mode=monotone_static`,
  `positive_offsets=0`. Residual non-1755 single-bin 15 MHz overshoot persists (safe verify-axis
  early stop); FORGE low confidence (0.21) unrelated.
- **Efficiency**: 33 probes vs 32 baseline (≈ flat) but **−5 vs the equivalent from-cap descent (38)**
  for an identical frontier (net of one B2-fallback probe). Modest on this RTX 3060 Ti because mid
  targets stop early on verify-axis residual overshoot regardless of start voltage. **Keep default
  OFF** until more runs justify flipping it.
- **Observability follow-up** (`6f2f061 feat(service): surface build-frontier scheduler logs`):
  log-only — `run_build_frontier` now emits the scheduler `result.log` (bracket carry / warm-start /
  fallback / probes_used) before the synthesis `result.profiles.log`, deduping shared lines (pure
  `ordered_frontier_logs` helper + 2 unit tests). No tuning behavior changed. Closes the validation
  finding that bracket telemetry existed but was invisible in CLI output.
- **Next (later, optional)**: 1–2 more warm-start runs; benign-zero-only (NoDownCapNeeded)
  bracket-seeding refinement (so a boost-top bracket doesn't mis-seed the next sub-boost target — the
  1905 case); broader frontier/profile confidence work. **Do NOT mix with profile persistence yet.**
- **Scope**: docs/continuity in this pass (`handoff.md`, `memory.md`, this file). Code already
  shipped/pushed in `23b70c4` + `6f2f061`.

## F1b Phase 2B.2-c: monotone static-base VF ceiling writer — hardware-validated (commit 8503182)
- **Validation** (2026-06-12): the monotone static-base VF ceiling writer (`8503182 feat(service):
  add monotone static-base VF ceiling writer`, on `origin/master`) was confirmed on real hardware by
  a supervised run `build-frontier --confirm --max-targets 7 --max-probes 40 --safe-start-cap 1075`
  (fresh debug build at `8503182`; clean bounded dry-run first; user present). This validates the
  writer behind the b.1 → c.0 → 8b6e105 (legit-zero diagnostics) → 91119e1 (NoDownCapNeeded rescue)
  → 8503182 chain.
- **Result**: exit 0; no TDR/reboot; Safe Loop armed+cleared; `reset_to_stock` ran; no persistence
  (`boot_flag.json`/`gpu_applied.json` absent after; `forge_state.json`/`gpu_knowledge.json`/
  `heartbeat.txt` unchanged); GPU back at stock idle. All 32 probes used `write_mode=monotone_static`
  with `positive_offsets=0` (static-base-anchored monotone-down offsets only).
- **The writer fixes the boost-top overshoot.** Primary case `1755 @ 900 mV`: previously plateau
  1755..1845 with `overshoot_veto=true` → `LiveMismatch`; now plateau 1665..1755, overshoot=0, veto
  not triggered → `NoDownCapNeededCeiling` (pass). The run then reached `1755 @ 875 mV` and verified
  (`NoDownCapNeededCeiling`, overshoot=0, plateau 1620..1755, ~19 s dwell, ≈1755 MHz @ 875 mV
  ≈179 W). Matches the monotone-writer safety-audit expectation.
- **Caveats (not blockers)**: a few non-1755 probes at low ceilings still carry a single-bin 15 MHz
  overshoot (`overshoot_veto=true`); FORGE synthesis reported low confidence (best 0.21, single-trial
  Wilson) — unrelated to the writer fix. **Next**: design warm-started voltage-bracket reuse for
  F1b/Godforge; keep build-frontier non-persisting (do not mix with persistence / profile apply yet).
- **Scope**: docs/continuity only (`handoff.md`, `memory.md`, this file). No code/test/IPC/hardware
  change in this pass.

## F1b Phase 2B.2-c.1: stock-equivalent ceiling verification for boost-top targets
- **Trigger** (2026-06-11): the FIRST bounded supervised `build-frontier --confirm` run
  (`--max-targets 1 --max-probes 6 --safe-start-cap 1075`, Fable-5-audited, user present) completed
  SAFELY (no TDR/reboot; Safe Loop armed+cleared per probe; reset-to-stock fired; no persistence;
  GPU back at stock) but produced **0 frontier points**: the only probe (target=1935 = the stock
  cluster boost top, ceiling 1075 mV) was rejected `LiveMismatch` at `offsets=20/27,
  plateau=1935..1935, overshoot=0`. Root cause: a flatten whose target equals the stock boost top
  legitimately needs ZERO offset on bins already at target, so the ≥90% offset-presence gate
  under-counts — the ceiling effect was present but unprovable by presence alone.
- **Decision**: add a NARROW stock-equivalent acceptance path, not a weaker global gate. New pure
  `is_stock_equivalent_ceiling` (`gpu_verify.rs`) consulted ONLY when the unchanged
  `classify_curve` gate says `LiveMismatch`. Accept only when ALL hold: (1) caller-supplied stock
  boost top present and target AT or BELOW it by ≤ tol — DIRECTIONAL `target ≤ top && top − target
  ≤ tol_mhz` (15 MHz); a target ABOVE the stock top is an overclock, rejected even within tol
  (replaces the original symmetric `abs_diff`); (2) every expected bin's offset readable;
  (3) NO bin reads above target (overshoot rejected even within tol); (4) every bin within tol
  below target; (5) every ZERO-offset bin reads EXACTLY at target — offset 0 means GetStatus shows
  the unmodified stock base and a correct flatten writes `target − base`, so `base == target` is
  the only valid explanation for a missing offset. Result carried as service-internal
  `LiveCeilingEval.stock_equivalent` (+`stock_equivalent_bins` diagnostic); **`CurveVerification`
  (IPC) is untouched** and `state` still reports the normal verdict.
- **Why it does not weaken safety**: the path demands MORE evidence than the normal gate (per-bin
  frequency agreement + zero-overshoot + full accounting of every bin), not less; a silently-failed
  apply leaves below-top bins at their stock base → violates (4)/(5) → rejected; GetStatus idle
  noise can only cause a false REJECT (fail closed). `VerificationFailed` (unreadable/empty) is
  never rescued. Below-boost targets with weak offsets stay `LiveMismatch`.
- **Data flow**: `eval_ceiling_evidence`/`classify_live_ceiling` gain `stock_top_mhz: Option<u32>`.
  `verify_applied_curve` (VerifyAppliedProfile IPC) passes `None` → classification byte-identical.
  `real_probe_step` passes `Some(seed.stock_boost_max_mhz)` and accepts `VerifiedCurve ||
  stock_equivalent`, logging the accepted branch distinctly as `verify=StockEquivalentCeiling`.
- **Scope**: `gpu_verify.rs` + `gpu_power_sweep.rs` only. No IPC/contract/core/`apps/ui`/Safe-Loop/
  reset/abort/`gpu_apply`/planning change, no hardware run in this patch. `cargo check` clean;
  service **109/109** (+11 stock-equivalent tests: the exact first-run reproduction, plateau-miss /
  any-overshoot / below-boost / zero-offset-not-exact / no-stock-top / unreadable-offset rejects,
  the fully-degenerate all-zero-at-target accept, tol-boundary accept/reject on both the bin-freq
  and target-vs-top gates, and the directional above-top reject), core 46/46. **`--confirm` remains
  gated on explicit user approval; re-run the bounded dry-run first.**

## F1b Phase 2B.2-c.0: first-run limiter flags for build-frontier
- **Decision** (2026-06-08): bound the first supervised hardware run so it validates the pipeline
  without the full 84-dwell plan. Added `build-frontier` flags: `--max-targets N` (truncate to the
  top N targets), `--max-probes N` (hard-stop total probe executions), `--safe-start-cap MV` (lower
  the descent start to the cap when below the derived cluster top).
- **Semantics**: dry-run + confirmed both honor the flags; defaults (no flags) preserve the full
  plan. `--safe-start-cap` never raises above the derived top and never goes below the crash floor.
  FAIL CLOSED on absurd values (max-targets/max-probes = 0; cap ≤ crash floor; non-numeric/missing).
  `--max-probes` short-circuits remaining probes (no hardware), then the run resets to stock + clears
  the Safe Loop flag (no auto-apply, no `forge_state`/`gpu_knowledge` writes).
- **Pure helpers**: `FrontierLimits` / `validate_limits` / `apply_frontier_limits` (gpu_power_sweep);
  `parse_frontier_limits` (main.rs). Dry-run prints a `limits` line + the capped dwell budget.
- **Scope**: `gpu_power_sweep.rs` + `main.rs` only. No IPC/contract/core/`apps/ui`/Safe-Loop/
  `gpu_apply`/`nvml_gpu`/Phase-3/11D change, no hardware. `cargo check` clean; service 95/95 (+7),
  core 46/46. **Dry-run QA** (`--max-targets 1 --max-probes 6 --safe-start-cap 1075`, stock, no
  --confirm, no state writes): targets=[1935], descent 1075→875 mV (9 bins), 6 dwells (~120 s,
  capped). `--confirm` remains forbidden.

## F1b Phase 2B.2-b.4: derive safe_start from the stock core VF cluster (refines b.3)
- **Decision** (2026-06-07): b.3's generic guard rejected absurd values (7001 MHz, 1237 mV) but
  still let `safe_start` come from the global max of *all* sane points — which on the 3060 Ti was
  1150 mV (the hard-cap boundary, likely a non-core point inside the generous range). b.4 adds a
  stage-2 derivation: select the actual contiguous core VF cluster and derive boost/sustained/
  safe_start from the CLUSTER TOP, never the global sane max.
- **`select_core_cluster`** (pure): sort sane points by voltage; split into contiguous runs where
  the voltage gap ≤ `CORE_CLUSTER_GAP_MV` (60 mV); pick the LARGEST run (ties → lowest voltage = the
  dense core); FAIL CLOSED if it has < `MIN_CORE_CLUSTER_POINTS` (8). Isolated high-voltage points
  above the cluster top are reported as rejected outliers.
- **Diagnostics**: the dry-run now prints raw/retained/rejected counts, rejected extremes, the
  selected core-cluster mV+MHz range, outliers-above count, the stock reference (from cluster top),
  the safe_start source, and a WARNING when a profile appears applied (`gpu_apply::load_applied()`).
- **b.3 generic hard guards retained** (freq [500,3500] MHz, voltage [600,1150] mV); b.4 only changes
  WHICH sane point becomes safe_start.
- **Scope**: `gpu_power_sweep.rs` + docs only. No IPC/contract/core/`apps/ui`/Safe-Loop/`gpu_apply`/
  `nvml_gpu`/Phase-3/11D change, no auto-reset, no hardware. `cargo check` clean; service 88/88,
  core 46/46. **Stock dry-run QA pending the user's manual reset; `--confirm` remains forbidden.**
  Future: NVML `max_clock_info(Graphics)` could corroborate boost (frozen `nvml_gpu.rs` this patch).

## F1b Phase 2B.2-b.3: graphics-core sanity-domain seeding guard (safety fix)
- **Decision** (2026-06-07): the first `build-frontier` dry-run revealed the seeding derived
  candidate clocks + safe_start from the UNFILTERED global max over `read_vf_curve_modern()`, which
  includes non-core / memory-domain points → a bogus plan (targets 7001..6311 MHz, safe_start
  1237 mV). The dry-run gate correctly blocked it (zero hardware). Fix: never seed from the global
  max; seed ONLY from sane graphics-core points and FAIL CLOSED otherwise.
- **Guard** (pure, in `gpu_power_sweep.rs`): `sane_core_points` keeps points with freq ∈ [500, 3500]
  MHz and voltage ∈ [600, 1150] mV; `derive_core_seed` derives boost/sustained/safe_start from those
  only, records rejected-point diagnostics (rejected max freq/voltage), emits soft-limit warnings
  (freq > 3200, voltage > 1125), and returns `Err` (fail-closed) when no sane points remain or a
  derived value exceeds a hard guard. `run_build_frontier` aborts (no Safe Loop arm / apply / dwell /
  VF write) on `Err` or if any candidate target > 3500 MHz. Constants are SANITY guards, NOT tuning
  targets; a future GPU outside them fails closed and prompts a code update.
- **Result** (re-run dry-run, same card): 132 raw points → 88 sane-core retained, 44 rejected (incl.
  the 7001 MHz / 1237 mV memory points); boost~1935 MHz; targets 1755..1935; 84 worst-case dwells
  (~1680 s). safe_start landed at 1150 mV (the hard-max boundary) → flagged by the soft-max warning;
  the live curve is currently in an applied state, so a stock read would be cleaner.
- **Scope**: `gpu_power_sweep.rs` + docs only. No IPC/contract/core/`apps/ui`/Safe-Loop-behavior/
  `gpu_apply`/`nvml_gpu`/Phase-3/11D change, no auto-reset, no hardware. `cargo check` clean; service
  86/86 (+5 guard tests), core 46/46. Dry-run QA: sane plan, no state writes. **`--confirm` remains
  forbidden until the fixed dry-run is reviewed.**

## F1b Phase 2B.2-b.2: real probe closure + supervised `build-frontier` (code only, not run)
- **Decision** (2026-06-07): implement the real Windows-only probe + supervised console entry, but
  DO NOT execute the hardware path in this patch (validated by `cargo check`/tests only).
- **`real_probe_step`** (the `build_frontier` seam under `--confirm`): abort/boundary guard → snap
  `vbin` to a real VF bin (`nearest_vf_bin_at_or_above`) → arm Safe Loop → `apply_vf_ceiling` →
  read-only verify via the shared `classify_live_ceiling` (+ 11C diag log) → on not-VerifiedCurve
  reset+clear+return → `load_and_measure` dwell → clear flag → `measured_to_probe` + set `vf_bin_mv`.
  A dwell **Crash** resets to stock and sets an `abort` flag so the remaining probes short-circuit
  (run drains safely); a normal Unstable/unverified only stops THAT clock's descent.
- **`run_build_frontier(store, confirm)`**: always prints the `plan_frontier` plan. Dry-run (no
  `--confirm`) is read-only — no arm/apply/dwell/VF-write, no startup recovery. Confirmed runs
  `build_frontier` with the real probe, then ALWAYS `reset_to_stock` + clears the flag. **No
  auto-apply, no `forge_state` write, no `gpu_knowledge` write.** Console subcommand `build-frontier`
  in `main.rs`; `--confirm` runs startup recovery (parachute) first.
- **First-version conservative seeding** (operator-tunable consts; review the printed plan before a
  run): `lowest_safe_mv=875` (above the ~855 mV known reboot), 25 mV step, 30 MHz clock step, 0.90
  floor; an idle `Unconstrained` regime is clamped to `PowerLimited` (no OC on a first run);
  sustained ≈ curve top freq; per-probe confidence = 0.21 (single-trial Wilson; matures via V3).
- **Scope**: `gpu_power_sweep.rs` + `main.rs` only. No IPC/contract/core/`apps/ui`/Safe-Loop-behavior
  /`gpu_apply`/`nvml_gpu`/Phase-3/11D change. `cargo check` clean; service 81/81 (+1 arg-parse test),
  core 46/46. **Hardware path NOT executed** — supervised dry-run + `--confirm` QA is 2B.2-c
  (separately gated).

## F1b Phase 2B.2-b.1: seeding + dry-run plan + vf_bin propagation (pure prep)
- **Decision** (2026-06-07): land the pure half of 2B.2-b. Exposes the verifier's
  `classify_live_ceiling` / `LiveCeilingEval` / `CurveDiag` as `pub(crate)` (intra-crate visibility
  only — NO IPC/contract change) so the future transient-ceiling probe (2B.2-b.2) reuses ONE
  classification path. Adds pure seeding: `derive_descent(curve_bins, lowest_safe, step) ->
  FrontierDescent` (safe_start = top live bin, clamped ≥ the operator crash floor) and a read-only
  dry-run `plan_frontier(targets, &descent, dwell_ms) -> FrontierPlan` (worst-case dwell count +
  wall-time estimate + safety notice). `candidate_clocks` / `classify_regime` (Phase 1) supply targets.
- **Internal `ProbeSample.vf_bin_mv: Option<u32>`** (NOT IPC): the actually-applied snapped bin.
  `probe_to_point` now records `vf_table_voltage_mv = vf_bin_mv.or(descent vbin)`; the pure
  `measured_to_probe` leaves it `None` (the real probe fills it after the apply in 2B.2-b.2).
- **Scope**: pure prep only — NO real probe, NO `apply_vf_ceiling` / `load_and_measure`, NO
  `build-frontier` subcommand / `--confirm`, NO Safe-Loop arm/clear, NO startup-recovery wiring, NO
  forge_state / gpu_knowledge writes, NO Phase-3 / 11D / `apps/ui` / core / contract change, NO
  hardware. `cargo check` clean; service 80/80 (+7), core 46/46 (untouched). 2B.2-b.2 (real probe +
  supervised `--confirm` entry) and the hardware QA run remain separately gated.

## F1b Phase 2B.2-a: shared live-ceiling classification helper (pure refactor)
- **Decision** (2026-06-07): factor live-curve classification out of `verify_applied_curve`
  into a reusable path so the persisted-profile verifier (today) and the future transient-ceiling
  probe (2B.2-b) share ONE classifier. `classify_live_ceiling(live, ceiling_idx, ceiling_mv,
  target, tol)` (read-only; builds the offset-readback evidence at/above the bin) →
  `eval_ceiling_evidence(target, anchor_idx, &expected, tol)` (pure; runs the UNCHANGED
  offset-presence `classify_curve` gate + the 11C `compute_curve_diag`) → `LiveCeilingEval`
  bundle.
- **Behavior unchanged**: `VerifyAppliedProfile` output is byte-identical (same classifier, same
  diagnostic, same inputs) — the refactor only removes inline duplication. Offset-presence remains
  the gate; GetStatus plateau spread stays diagnostic-only; voltage never affects classification.
- **Scope**: service-internal refactor + 5 pure tests in `gpu_verify.rs`. NO real probe, NO
  `build-frontier` subcommand, NO `apply_vf_ceiling` / `load_and_measure`, NO Safe-Loop / synthesis /
  `apps/ui` / core / contract / Phase-3 / 11D change, NO hardware. `cargo check` clean; service
  73/73 (+5), core 46/46. Pure seeding helpers were NOT added (dead code until 2B.2-b). 2B.2-b
  (real probe + supervised `--confirm` entry) and the hardware QA run remain separately gated.

## F1b Phase 2B.1: pure probe-mapping prep + target_clock_mhz (no hardware)
- **Decision** (2026-06-07): land the pure, hardware-free half of Phase 2B first. Adds
  `measured_to_probe(&Measured, curve_verified, confidence) -> ProbeSample` (in
  `gpu_power_sweep.rs`) — the seam the real probe closure (2B.2) will use to feed `build_frontier`
  — performing NO hardware I/O, only a conservative interpretation of already-collected dwell data.
- **Conservative mapping rules**: a `Stable` verdict becomes `ProbeOutcome::Stable` ONLY when
  clock/power telemetry quality ≥ Medium AND a sustained-clock `p5` is present; `SilentError` /
  `Crash` (incl. a TDR / device-lost dwell → `Measured::degenerate(Crash, …)`) or weak telemetry →
  `Unstable`. `p5_clock` is preserved as the sustained-clock signal (0 / no samples → `None`);
  measured voltage uses the ramp-filtered avg and stays `None` when missing — never a fake 0.
- **Additive schema**: `PowerSweepPoint.target_clock_mhz: Option<u32>` (`#[serde(default)]`,
  backward-compatible, no schema bump) records the asked-for clock vs `clock_mhz` (measured
  achieved). Phase 2A `probe_to_point` now stamps it; the single-clock live sweep sets `None`.
- **Scope**: pure / backend-safe only. NO real probe closure, NO `apply_vf_ceiling`, NO
  `load_and_measure` loop, NO supervised console command, NO Safe-Loop / synthesis / `apps/ui` /
  Phase-3 / 11D change, NO hardware. `cargo check` clean; service 68/68 (+7), core 46/46 (+2).
  Phase 2B.2 (real probe + supervised entry point) and the hardware run remain separately gated.

## Read-only live diagnostic for the elastic VF ceiling (Patch 11C)
- **Decision** (2026-06-06): extend the existing read-only verifier (`gpu_verify::verify_applied_curve`
  / `verify-applied`) with structured diagnostic evidence + a single live telemetry snapshot, so the
  applied-curve↔telemetry relationship is provable **without mutating GPU state**. Classifier
  semantics are **unchanged**: `VerifiedCurve` still gates on flatten-offset *presence*; measured/live
  voltage above the VF anchor never downgrades; GetStatus freq stays diagnostic only.
- **Added (pure, testable)**: `compute_curve_diag` over the same per-point evidence → first modified
  bin index/voltage, modified vs expected bin count, GetStatus freq-match count, GetStatus plateau
  min/max MHz, max target overshoot/undershoot, and 3 representative offset samples (first-modified,
  anchor, highest-voltage). Plus one read-only `LiveSnapshot` (NVAPI measured voltage + first NVML
  reading: clock/power/util/temp/limit/cap). Surfaced via additive `Option`/`serde(default)` fields on
  `ApplyVerificationStatus` and one compact `apply_verify_diag:` log line.
- **What it proves**: the flatten offsets are resident and *curve-flatten-shaped* (big `+` at the
  low-voltage anchor, `−` at the top), and how far the GetStatus plateau spreads vs target. **What it
  does NOT prove**: effective/measured voltage behavior, exact per-point offset correctness, or live
  in-game stability. The live snapshot is telemetry, not load verification.
- **Exact-offset verification deferred**: expected offset is `target − stock_base_mhz`, but per-point
  stock base is not persisted and GetStatus freq is idle-unreliable, so exact-offset classification is
  NOT implemented (would need a persisted pre-apply stock curve, or validating the GetStatus `base`
  tuple — a future patch). 11C reports the *symptom* (plateau spread / offset distribution) instead.
- **Runtime QA finding (read-only, 3060 Ti)**: `VerifiedCurve` (62/64 offsets present), but the
  diagnostic revealed `anchor_offset_khz=+255000`, `highest_bin_offset_khz=−120000`, GetStatus plateau
  **1770–1830 MHz** (overshoot 45, undershoot 15) and live snapshot `voltage=1068 mV, clock=1815 MHz,
  util=6%`. This is consistent with both (a) a genuinely curve-flatten-shaped offset set and (b) the
  open **overshoot suspect** (plateau not landing exactly on target) — but GetStatus idle noise
  (freq_match 18/64) means it is **not yet conclusive**. Confirms the diagnostic does its job: surface
  the evidence, defer the verdict to exact-offset work. No state mutated (`gpu_applied.json` mtime
  unchanged).
- **Scope**: additive IPC + log only. Files: `crates/service/src/gpu_verify.rs`,
  `crates/core/src/ipc.rs`, `docs/contracts/ui-backend.md`, docs. **No apply/classifier/Safe-Loop/
  synthesis/`apps/ui`/`nvml_gpu.rs` change; P-state + full ThrottleReasons deferred; no hardware
  writes.** `cargo check` clean; service 61/61 (+9 diag tests), core 44/44.

## Elastic VF ceiling caps frequency, not effective voltage (no hard voltage cap)
- **Decision** (Applied Voltage Behavior investigation, 2026-06-06): the canonical apply path
  (`apply_vf_ceiling`, `crates/gpu-nvapi/src/lib.rs`) writes **per-point frequency offsets** to
  every modern ClkVfPoints curve point whose VF-table voltage is **≥ the selected ceiling bin**,
  flattening them to `target_mhz`; points below the bin are left untouched (elastic). It writes
  **no voltage**, holds no rail lock, pins no clock. It therefore **caps frequency, not
  effective/rail voltage**, and **does not hard-cap measured voltage in any P-state**.
- **`vf_table_voltage_mv` (the VF/curve bin) is the deterministic apply/verify/frontier key** —
  re-derived by snapping the measured dwell voltage UP to the lowest table bin ≥ it
  (`nearest_vf_bin_at_or_above`). `measured_voltage_mv` (NVAPI `core_voltage`) and HWiNFO's
  "GPU Core Voltage" are a DIFFERENT domain (measured rail incl. load-line/droop) — telemetry +
  cross-check only. They may legitimately read **above** the VF bin (idle/2D especially, and under
  load by the VID→rail offset). **Measured ≠ the bin is EXPECTED, not a mismatch** (idle ~1.075 V
  and in-game ~0.887–0.956 V for an ~850 mV bin are normal).
- **Nidavellir must NOT imply a hard voltage cap.** "X MHz @ Y mV" reads as a rail-voltage ceiling
  the engine does not provide; prefer "1785 MHz target · 843 mV VF bin". Profile cards should
  eventually show the VF bin AND the measured-under-load voltage (avg/min/max — fields already on
  `PowerSweepPoint`) as SEPARATE values.
- **A true hard voltage cap would require the legacy voltage-lock path** (`lock_core_voltage_mv` /
  `set_vfp_locks`) — the documented TDR cause under game load. There is no "soft voltage ceiling"
  NvAPI mechanism. A hard cap is therefore **not aligned with safety-first**; Nidavellir stays on
  the elastic VF ceiling only.
- **What verification proves**: `VerifyAppliedProfile` confirms the frequency-flatten OFFSETS are
  resident (≥90% of plateau points carry a non-zero offset) plus a load axis from stored dwell
  stats. It proves **nothing about effective/measured voltage** and cannot (yet) detect a
  present-but-wrong-valued offset or a live-load plateau.
- **Open suspect (not confirmed; read-only-testable, deferred)**: offsets are computed as
  `target − base_mhz` with `base_mhz` from GetStatus at apply time, and GetStatus under-reports
  freq at idle — so a plateau applied at idle could land above `target` (consistent with observed
  ~1815–1830 MHz vs ~1785, alongside normal 15 MHz boost-bin quantization). To be confirmed by a
  future read-only live diagnostic, NOT changed here.
- **Scope**: documentation/contract only (Patch 11A) — `decisions.md`, `docs/contracts/ui-backend.md`
  (incl. a Codex wording request), `memory.md`, `handoff.md`. **No backend code, no `apps/ui`, no
  apply/verify change, no F1b Phase 2B, no hardware.** Live diagnostic (11C) and the UI copy
  implementation (Codex) are deferred.

## F1b Phase 2A: simulated multi-clock outer-loop scaffolding (no hardware)
- **Decision** (2026-06-06): prove the multi-clock loop in isolation BEFORE touching hardware.
  `build_frontier(candidate_clocks, &FrontierDescent, &ForgePolicy, probe: impl Fn(u32,u32) ->
  ProbeSample)` (in `gpu_power_sweep.rs`) drives the outer loop over candidate clocks and a
  per-target voltage-bin **descent** through an **injected probe closure** — the closure is the
  only seam to (future) hardware.
- **Loop rules**: inner descent starts at `safe_start_mv`, steps down by `voltage_step_mv`,
  **never below `lowest_safe_mv`** (the known-crash floor as a config input); keeps the deepest
  stable point; stops on first `Unstable`; stops/drops on simulated `curve_verified == false`
  (the Phase-2B Patch-A gate); drops a clock with no stable point. Outer loop allows a partial
  frontier; empty frontier → synthesis returns all-`None` (safe failure, no panic).
- **Frontier points** record `vf_table_voltage_mv` as the deterministic bin; measured voltage
  stays telemetry only. Synthesis = `synthesize_forge_profiles(&frontier, policy)`.
- **No hardware path wired**: no `load_and_measure`, no `apply_vf_ceiling`, no VF write, no GPU
  stress, no Safe Loop interaction, no real power sweep. All types `#[cfg(windows)]
  #[allow(dead_code)]` (wired in Phase 2B).
- **Validation**: `cargo check` clean; service **52/52** (8 sim tests; 3060 Ti → 1830/1815/1740
  and 4090 → 2880/2860/2700 proven *through the loop*; inner-stop, boundary, verify-fail,
  partial, collapse, no-valid all covered).
- **Phase 2B (future)**: real probe closure (apply ceiling at bin → Safe-Loop-armed dwell →
  offset-readback `VerifiedCurve` gate) behind a supervised/approval-gated entry point.
  **Phase 3** (knowledge re-keying by `(target_clock, vf_table_voltage_bin)` + global voltage-floor
  boundary) remains future work.

## F1b Phase 1: policy-driven multi-clock synthesis (pure, service-internal)
- **Decision** (2026-06-06): the three profiles are synthesized from ONE multi-clock frontier
  via centralized policy, not three independent sweeps. `ForgePolicy` (in `gpu_power_sweep.rs`)
  holds the thresholds — **Balanced default: Brokkr's ≥ 0.98 × Godforge clock, Deep Calm ≥ 0.90,
  confidence ≥ 0.85**; Conservative (0.99/0.92/0.95) and Aggressive (0.97/0.85/0.70) presets exist.
- **Selection rules** (`synthesize_forge_profiles(frontier, &ForgePolicy)`):
  - **Godforge** = highest **sustainable** clock — uses `p5_clock_mhz` when present (dip-aware),
    falls back to `clock_mhz`; ties → lowest power.
  - **Brokkr's Best** = **max R = %power_saved ÷ %clock_lost within the Brokkr's clock floor**
    (real trade: clock < Godforge, power < Godforge). Resolves the F1b-doc 4090 ambiguity:
    Brokkr's = **2860** (max R within floor), NOT 2840 — keeps Brokkr's nearest Godforge rather
    than drifting into the eco profile.
  - **Deep Calm** = max MHz/W within the Deep Calm clock floor (stays useful).
- **Measured voltage is NOT a selection axis** — selection uses clock/power/p5/confidence only;
  `vf_table_voltage_mv` stays the deterministic apply axis (per the voltage-split decision).
- **Single-clock collapse** (the old single-clock sweep's failure mode) is detected and logged;
  synthesis still returns all three profiles (no panic/empty).
- **Scope**: pure, service-internal, additive — no IPC change, no `apps/ui`, no Safe Loop, no
  hardware path. `cargo check` clean; service 44 tests (F1a assertions unchanged). **Phase 2 (real
  multi-clock measurement loop) NOT started** — needs simulated outer-loop scaffolding first, then
  a supervised/approval-gated hardware run.

## Forge action consolidation: Power Sweep is canonical; Real Sweep / Forge Everything are legacy
- **Decision** (Forge Action Consolidation audit, 2026-06-06): **`gpu_power_sweep.rs` (Power
  Sweep) is the canonical Forge GPU core path** — offset + elastic VF ceiling
  (`apply_vf_ceiling`), game-power render dwell, Safe-Loop-guarded, knowledge-bounded, **no
  voltage lock**. It is the only current safe core-optimization engine and the one F1a/F1b
  build on. Canonical apply path = `ApplyPowerGodforge/ApplyPowerBrokkrs/ApplyPowerDeepCalm`.
- **`gpu_sweep_real.rs` (Real Sweep) and `gpu_forge_all.rs` (Forge Everything) are LEGACY
  voltage-lock paths**: both call `lock_core_voltage_mv` (Real Sweep L239/L370; Forge Everything
  fixed `CORE_VOLTAGE_MV=900` L193) — the documented TDR cause under load. Real Sweep also uses
  compute (ALU) load, not game power. Forge Everything tunes VRAM around a *fixed-voltage* core,
  not a forged curve. The legacy `ApplyGodforge/ApplyBrokkrs/ApplyDeepCalm` trio (from
  `real_sweep.profiles`) belongs to this generation.
- **Legacy core paths should be hidden from normal UI** (developer/diagnostic only) and
  **scheduled for removal in a later patch** (after F1b makes the Power Sweep pipeline the whole
  story). Do NOT remove the IPC methods yet — keep them wired to avoid mid-stream build/IPC breaks.
- **VRAM tuning is a FUTURE Forge GPU pipeline step**, not an independent primary action. It MUST
  run **after** the core VF curve is forged + validated and **adapt to** that curve — it must
  never define or destabilize the core. `gpu_mem_sweep.rs` is safer than the legacy core paths
  (no core voltage lock) but today runs independently of the forged core, so it stays an Advanced
  Diagnostic until redesigned. The Gen-1 `Forge Everything` ordering (VRAM around fixed-voltage
  core) is exactly what this rule forbids.
- **Status**: audit only — no code removed, no `apps/ui` change. Frontend request recorded in
  `docs/contracts/ui-backend.md`.

## Voltage is three concepts, not one number; F1b keys on VF-table, not measured dwell voltage
- **Decision** (Sensor Quality Audit, 2026-06-05): GPU voltage must be split into
  explicitly-named, never-conflated fields:
  - `vf_table_voltage_mv` — deterministic VF-curve point voltage (NVAPI GetStatus).
    **This is the apply/ceiling key and the F1b frontier axis.**
  - `measured_voltage_mv` (avg/min/max) — what the rail actually did under dwell
    (NVAPI `core_voltage`, to be made dense + validated). **Descriptive telemetry and
    cross-check only — never an apply key.**
  - `effective_rail_voltage_mv` (future) — physical rail incl. droop, if separable;
    the only value meaningfully comparable to HWiNFO's "GPU Core Voltage".
- **F1b must NOT use measured dwell voltage as the deterministic apply/frontier key.**
  Key the frontier by **(target clock + VF-table point/index)**; attach measured dwell
  telemetry as a separate descriptive field.
- **Why**: the current sweep stores voltage as a sparsely-sampled (~480 ms) NVAPI
  string-parsed **max**, then reuses that exact number as the `apply_vf_ceiling`
  threshold (`PowerSweepPoint.voltage_mv` → `AppliedProfile.core.voltage_mv` →
  `ceiling_mv`). A noisy measured max steering a deterministic curve op is the root of
  the 837-vs-869 confusion and makes apply fidelity unprovable. Three structs already
  carry a field literally named `voltage_mv` with three incompatible meanings.
- **Corollary**: 837-vs-869 mV is consistent with undersampling + bin quantization
  (+ possible sensor-source difference) — expected semantics, NOT proof of apply
  failure. The constant ~1062 mV unfocused/desktop state is most likely a
  workload-scoped (P0/3D) ceiling leaving other states on the stock curve, compounded
  by the frequency-only flatten leaving voltage uncapped. Neither verdict is finalized.
- **Sequencing**: (1) split voltage fields + stop keying apply on measured voltage
  (must-fix); (2) richer dwell stats (min/p5 clock, voltage avg/min/max, full
  `ThrottleReasons` limiter, sample_count, timestamps, workload-context tag);
  (3) finalize Applied Curve Verification (compare VF-table plateau via GetStatus,
  table-to-table — not against measured voltage); (4) resume F1b on the cleaned axis.
- **Status**: audit recorded; no code/IPC/UI change yet. Contract additions
  (populate `GpuSensors.voltage_mv`, `dwell_quality`, `GpuApplyStatus.verification`)
  to be drafted in `docs/contracts/ui-backend.md` on approval.

## Product model: 3 profiles over a clock×power frontier (reverses "two profiles")
- **Decision**: ship **Godforge / Brokkr's Best / Deep Calm**, synthesized from a
  multi-clock power frontier:
  - Godforge = highest sustainable clock under the cap (not the advertised boost).
  - Brokkr's = max `R = %power_saved ÷ %clock_lost` vs Godforge (**NOT** max MHz/W).
  - Deep Calm = max MHz/W.
  Two orthogonal axes: the **product profile** (objective) and the **confidence
  profile** (Conservative/Balanced/Aggressive — the V2 Wilson gate, applied to all 3).
- **Reverses**: the earlier "Two profiles, not three / removed Deep Calm" and
  "Brokkr's = max MHz/W". Under the product vision (`product.md`), Brokkr's is a
  benefit/cost trade and the old Brokkr's metric (MHz/W) becomes Deep Calm.
- **Requires** a multi-clock frontier measured in real power. The validated *flatten*
  sweep is single-clock; the *lock-voltage* frontier sweep is unsafe under game load
  and uses a voltage proxy. Chosen path (A): **extend the safe flatten sweep to
  multiple target clocks** (F1b) — keeps safety + game-power + continuous knowledge.
- **Status**: F1a (pure `synthesize_forge_profiles`) implemented + unit-tested
  (reproduces Godforge 1830 / Brokkr's 1815 / Deep Calm 1740); not yet wired (F1b).

## V2 selection = Wilson-confidence gate (not score×confidence), with V1 fallback
- **Decision**: among off-cap points, pick the highest accumulated `score()` (MHz/W)
  whose stability confidence — Wilson lower bound (z=1.96) over accumulated
  trials/stable_trials — clears the active profile threshold (**Conservative .95 /
  Balanced .85 / Aggressive .70**; Balanced active, a const). If none clears it, fall
  back to the V1 strategy (best off-cap perf/watt) and log it. Never returns "no
  solution".
- **Why a gate, not score×confidence**: safety-first. "Trust ≥ X, then best
  efficiency" is predictable; a product silently trades confidence for efficiency and
  could ride a barely-tested point. Ranking among the trusted still uses `score()`.
- **Why a join, not selecting from knowledge directly**: the off-cap invariant lives
  on the per-run `PowerSweepPoint` (`power_capped_frac`); `PointStat` has no cap
  field. V2 gates the off-cap subset and joins to `know.points` by offset for
  confidence — so data collection and the `gpu_knowledge.json` schema stay untouched.
- **Reality today**: 1 trial/point → Wilson-LB ≈ 0.21 everywhere → V2 always falls
  back to V1 (the chosen point is unchanged, but the decision is now logged). The gate
  "wakes up" only as trials accumulate across runs → motivates V3.
- **Scope**: code-only inside `gpu_power_sweep.rs`; unit-tested (Wilson values +
  gate-accept + gate-fallback). Supersedes the earlier "score×confidence" phrasing.

## Brokkr's objective = max efficiency (MHz/W), not min voltage
- **Decision**: the undervolt profile maximizes performance-per-watt (the efficiency
  knee), using the stability frontier only to bound the search.
- **Why**: chasing the lowest stable mV walks toward the crash cliff; the best
  perf/watt sits before it. Consistency (off-cap) > squeezing the last millivolt.
- **Alternatives**: "deepest stable undervolt" (rejected — courts the cliff).

## No fixed safety margin — data-driven, per-GPU
- **Decision**: drop fixed MHz margins (the old `CRASH_BUFFER=60` was arbitrary).
  V1 = margin **relative to the discovered zone width** (Conservative 30%). V2 =
  **Wilson lower-bound confidence** (e.g. 0 fails/50 trials ≫ 0/1) as the gate.
- **Why**: GPU curves differ; a fixed MHz is blind to the curve. Confidence/zone-
  relative margins adapt automatically and emerge from observed data.

## Stability frontier stored by severity, accumulated per GPU
- **Decision**: `BoundaryKnowledge { highest_clean, lowest_silent_error, lowest_tdr,
  lowest_reboot }` + per-offset `PointStat { trials, failures, worst_severity, ... }`
  persisted per GPU and summed across runs.
- **Why**: a cheap SilentError ≠ an expensive HardReboot; a HardReboot is valuable
  experimental data to be remembered permanently and **never re-probed**. Brokkr's
  becomes a continuous learning system of each GPU's curve, not a one-shot tuner.

## Apply undervolt via the elastic VF ceiling (ClkVfPoints), not a rigid lock
- **Decision**: flatten the V/F curve above the chosen voltage to the target clock
  via the modern NvAPI **ClkVfPoints** family (per-point freq offsets). No hard
  voltage lock, no NVML clock pin (those are the fallback for unsupported GPUs).
- **Why**: under a ~power-cap game load, a rigid voltage lock / clock pin removes the
  card's power management and **TDRs**. The Afterburner-style flatten keeps
  elasticity. Verified: ceiling controls the live clock under load with no TDR.
- **Key facts**: the old `nvapi` crate (`SetClockBoostTable`) is rejected on 595.97;
  exact struct IDs/layouts + the read-modify-write + one-bit-mask gotchas are in
  `~/.claude/.../memory/gpu-forge-real-v031.md`.

## Dwell load must be game-representative (and repeatable)
- **Decision**: stress with the FurMark-class textured render (`run_render_stress`),
  bounded to short frames (8 overdraw instances × 96 frag-iters), polled every 3
  frames, on a fresh GpuCtx per measurement.
- **Why**: a pure-ALU compute kernel drew ~159 W and never hit the cap → wrong
  regime. The render draws ~199 W and saturates the cap like real games. A single
  oversized frame (~2 s) tripped the TDR watchdog → bound per-frame work; many short
  frames = a real game and stay repeatable across a full sweep.
- **Rejected**: a correction factor on the compute reading (user wants real sensor
  values, not a fudge).

## Validate under game power; drop the rigid clock pin in validation
- **Decision**: the arduous 35 s soak uses the same render and only the offset (no
  NVML clock pin — the cap + curve limit the clock naturally).

## Two profiles, not three
- **Decision**: ship **Godforge** (max performance, ~99% cap, OC-oriented — refined
  later) and **Brokkr's Best** (best perf/watt, off-cap). Removed **Deep Calm**.
- **Why**: Deep Calm converged with Brokkr's once OC+UV were unified.

## Profiles must stay OFF the power cap (except Godforge)
- **Why**: a capped profile dips its clock in-game; off-cap = steady clocks =
  consistency, the project's differentiator.

## Safety: Safe Loop owns crash recovery; risky runs are supervised
- Hardware writes are boot-flag-guarded and reboot-survivable. Deep exploration is
  incremental and supervised (the user must be able to reboot). Verified end-to-end:
  a reboot left no bad profile persisted; the GPU came back at stock.
