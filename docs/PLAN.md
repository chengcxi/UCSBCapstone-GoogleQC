# Capstone feasibility and revised milestones

## Verdict

**Reasonable research capstone architecture; too broad as an unconditional one-quarter delivery promise.** Rust + Stim + the existing Micro Blossom FPGA stack is a sensible combination. The quarter should deliver a fixed-topology, two-layer windowed-memory prototype first. General lattice surgery, arbitrary circuit support, and defect-adaptive topology should be explicitly gated extensions, not implicit acceptance requirements.

This review uses the project vision pasted into the conversation and inspected upstream source. No private data, board specification, team capacity, or numerical real-time target has been provided. Calendar feasibility is therefore conditional; sprints below are dependency gates, not estimated dates.

## Stack assessment

| Area | Recommendation | Reason / constraint |
| --- | --- | --- |
| Circuit validation and DEM extraction | Pinned Stim Python API, called as an offline build step | Avoid writing another circuit simulator or DEM grammar in Rust. Flatten REPEAT/SHIFT via Stim; retain separators and logical labels. |
| Graph conversion and host control | Rust | Good type safety and eventual host integration. Keep Python/JSON at the compilation boundary, not in the per-shot live path. |
| FPGA | Extend upstream Scala/SpinalHDL and its existing simulation/vendor flow | Do not port the RTL stack. Confirm board, tool versions, licenses, and resource budget immediately. |
| Software correctness baseline | A reference MWPM decoder plus non-windowed baseline | Graph loading alone is not a decoding validation. Compare logical predictions/statistical logical error rates, not identical correction paths under ties. |
| Reconfiguration | Precompiled fixed-size superset graph with bounded per-phase weight/enable/boundary updates | Changing graph size/connectivity can require new routing and synthesis. Even a superset can exceed FPGA resources; benchmark it before promising general topology changes. |

Upstream `MicroBlossomSingle` includes vertex/edge communication trees, growth bounds, offloading, layer-fusion metadata and parity reporters. Its source explicitly describes parity reporters as not runtime-changeable in the current implementation. Changing only edge weights is therefore not sufficient to claim general runtime circuit support.

## Required scope corrections

1. **Replace “any Stim-expressible circuit”** with “supported circuits producing deterministic, graphlike/decomposable DEMs under an explicit approximation policy.” Stim expressibility does not imply MWPM compatibility. Hyperedges, gauge detectors, correlated components, undetectable logical faults, negative weights and logical-label collisions need explicit handling or rejection.
2. **Resolve duplicated lattice-surgery scope.** It appears both in Sprint 1 and stretch goals. Recommend an advisor-provided, known-good surgery fixture as a later acceptance target; generating general surgery circuits stays stretch. Detector coordinates do not encode logical-operation semantics or commit ownership.
3. **Separate topology adaptation from noise reweighting.** Broken qubits/couplers generally require circuit and stabilizer changes, not simply deleting DEM edges. Start with provided valid adapted circuits; automatic defect adaptation remains stretch.
4. **Move the CPU residual-syndrome oracle ahead of RTL changes.** Freeze seam ownership, physical vs temporal boundaries, index mappings and observable accounting with hand-worked examples first.
5. **Define “real time.”** Specify syndrome round period, detector counts, arrival bandwidth, allowed backlog, end-to-end latency (including scheduling/host transfers), throughput and high-percentile latency. Mean decoder latency alone is insufficient.
6. **Make hardware access a Sprint 1 dependency.** Identify board, synthesis/simulation access and a fallback simulation deliverable early. Proprietary data must not be a critical-path dependency or committed to this repository.

## Revised sprint gates

### Sprint 1 — reproducible compiler and window semantics

- [x] Stim circuit validation / DEM extraction, flattened canonical IR, provenance.
- [x] Implement and locally compile Rust graph conversion and upstream parser-input export.
- [x] Preserve edge-to-observable labels and source mechanism IDs.
- [x] Validate local window membership; identify commit/buffer seams and outer crossings. Revalidate in Rust, enforce same-layer commit exclusivity and layer/boundary agreement.
- [x] Add global correction XOR/parity checks with multiple observables (not the seam-residual oracle).
- [x] Add generated Cargo.lock; upgrade toolchain/CI to Rust 1.99.0 and edition 2024.
- [ ] Verify on the actual 1.99 compiler (download blocked locally; older-compiler compatibility tests pass).
- [x] Load exported files with native Fusion Blossom 0.2.13 ErrorPatternReader and run reference decoding.
- [ ] Run the full pinned Micro Blossom parser and test generated hardware JSON (handoff and optional CI job implemented).
- [x] Reference-decode tiny fixtures (logical predictions) and distance-3 memory (correction-syndrome consistency).
- [ ] Add statistically meaningful memory logical-error-rate regression; current 64-shot memory test is not one.
- [ ] Implement actual window graph cutting, explicit global/local detector maps, open layer-0 temporal boundaries and closed layer-1 temporal boundaries. Preserve physical code boundaries in both layers.
- [ ] Implement CPU residual-syndrome and committed-observable oracle, including seam-crossing chains and XOR cancellation. Explicitly test odd parity, start/end-of-experiment windows and multiple observables.
- [ ] Decide lattice-surgery fixture scope and document the meaning of all coordinates/phase labels.

**Exit:** a tiny seam-spanning fault is reconstructed by both layers and has the expected global observable parity; graph export round-trips through upstream; memory reference regression passes. Annotation-only JSON is not this exit gate.

### Sprint 2 — bounded runtime reconfiguration in simulation

- Freeze maximum vertices/edges, weight width, boundary-state representation and update protocol.
- Implement phase FSM, safe update barriers, reset behavior, and inactive-edge semantics.
- Support layer-0 residual output and layer-1 residual input using the Sprint 1 oracle.
- Account for growth bounds, offloading and parity reporter behavior under updates.

**Exit:** cycle-level simulation matches the software oracle through repeated phase changes, including backpressure and stale-state tests. No claim of arbitrary topology reconfiguration.

### Sprint 3 — FPGA-design simulation prototype

- Integrate a deterministic stream generator and two-layer scheduling.
- Run windowed memory at multiple noise points with confidence intervals and a non-windowed baseline.
- Measure cycles/shot, throughput, queue depth, tail latency and synthesis resource estimates.

**Exit:** pre-agreed logical-error tolerance and throughput targets are met in simulation; confidence intervals and shot counts are reported. “Expected logical error rate” must be a measured baseline/tolerance, not an unspecified number.

### Sprint 4 — hardware and reproducibility

- Bring up hardware; validate stream transport, timing closure and end-to-end correctness.
- Record tool versions, board/clock, utilization, latency/throughput, reproducible fixtures and regression commands.
- Validate provided data if available and legally usable; synthetic public fixtures remain the baseline.

**Exit:** repeatable hardware demonstration with correctness/performance report. If board/tool access is blocked, explicitly deliver the agreed simulation fallback rather than claiming hardware completion.

## Process

Kanban fits the uncertainty. Add a weekly correctness/demo checkpoint and a short retrospective; limit work in progress, record decisions, and use the sprint exit tests as gates. Suggested work streams: compiler/data contracts, software window oracle, FPGA integration. Do not assign dates without team capacity and quarter dates.

## References and inspection boundary

Source inspected on 2026-10-13:
- [Micro Blossom](https://github.com/shsym/micro-blossom), commit `3dd1817eecdb4bf9899433108d9a2edccf198c8e`: `src/cpu/blossom/src/resources.rs` and `cli.rs`.
- [Fusion Blossom ErrorPatternReader](https://github.com/yuewuo/fusion-blossom/blob/main/src/example_codes.rs): parser input header, initializer, positions, even-weight constraint. A pinned upstream runtime round-trip remains to be done.
- [Stim](https://github.com/quantumlib/Stim): installed Python API version 1.15.0, exercised in local tests.

Reading/validation queue from the supplied vision (not represented as a completed literature review):
- [Parallel window decoding](https://arxiv.org/abs/2209.08552): seam/residual algorithm and correctness conditions.
- [Parallelization in time](https://arxiv.org/abs/2209.09219): alternative scheduling/boundary assumptions.
- [Micro Blossom paper](https://arxiv.org/abs/2502.14787): hardware baseline and measurements.
- [Below-threshold experiment](https://arxiv.org/abs/2408.13687): candidate realistic-data requirements.
- [LUCI](https://arxiv.org/abs/2410.14891), [Adapting the Surface Code to Defects](https://journals.aps.org/prxquantum/abstract/10.1103/815q-xjrb), [bandage-like adaptation](https://www.nature.com/articles/s41534-025-01023-y), [adaptive surface code](https://quantum-journal.org/papers/q-2023-07-25-1065/), [time dynamics](https://arxiv.org/abs/2302.02192): defect-adapted input-circuit extensions.
- [Neural MWPM](https://arxiv.org/abs/2601.00242): listed in the vision; its supplied title is not the Micro Blossom architecture reference. Use the Micro Blossom paper for that baseline.

External paper text was not retrieved in this environment; no numerical guarantees from those papers are assumed here.
