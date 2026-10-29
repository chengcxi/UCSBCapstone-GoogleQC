# Parallel Window Decoding for Micro Blossom — CPU Implementation

This document describes the software changes that enable **Micro Blossom**
to process two-layer parallel windows as specified in the project vision.
It is the technical companion to the implementation in
`src/cpu/blossom-nostd/src/window.rs`,
`src/cpu/blossom-nostd/src/window_oracle.rs`,
`src/cpu/blossom/src/parallel_window.rs`, and
`src/cpu/blossom/src/window_oracle.rs`.

## 1. Background and references

* **Skoric *et al.* — Parallel window decoding enables scalable fault-tolerant
  quantum computation** [2209.08552]: introduces the two-layer scheme,
  commitment of only the older half of a window, artificial defects on the
  commit/buffer seam, and the throughput condition
  `N_par (n_com + n_W) τ_rd ≥ 2 τ_W`.

* **Tan *et al.* — Scalable surface code decoders with parallelization in
  time** [2209.09219]: the *sandwich decoder*, a sliding-window decoder whose
  windows have open time boundaries and whose seams are decoded in a second
  subroutine.  Proves that `buffer = d` preserves the threshold for MWPM and
  Union-Find.

* **Wu *et al.* — Micro Blossom** [2502.14787]: the heterogeneous MWPM
  architecture.  The CPU runs the primal (blossom) logic, the FPGA holds one
  processing unit per graph vertex/edge and executes the dual phase at
  `O(p²|V|)` active memory.  Each window can be realised as a separate FPGA
  context.

* **McEwen *et al.* — Relaxing hardware requirements using time-dynamics**
  [2302.02192]: distinguishes *physical* code boundaries (spatial virtual
  vertices, present in both layers) from *artificial temporal* boundaries
  (window cuts).  Both layers preserve the former; only the top layer treats
  the latter as open.

* **Neural MWPM** [2601.00242] is listed in the vision but is not part of the
  window geometry; it is noted for completeness.

## 2. Window geometry

Sections 2–4 describe the original programmatic `WindowSpec` API and its
time-midpoint rule. The CLI's explicit detector-ID windows use the separate
detector-incident commit rule described in section 6; the two rules are not
interchangeable.

### 2.1 Definitions

| Concept | Meaning |
|---|---|
| **Window** | A spacetime subgraph `G_W = (V_W, E_W)` where `V_W = { v \| t(v)∈[t_start,t_end) }`. |
| **Commit region** | `C_W = { e∈E_W \| t_mid(e)∈[commit_start,commit_end) }`.  Only edges whose midpoint lies in the commit interval are committed. |
| **Buffer region** | `B_W = G_W \ C_W`.  Tentative edges here are discarded. |
| **Seam** | The boundary `commit_start` or `commit_end` between `C` and `B`.  A committed edge that has one endpoint outside `C` leaves an artificial detection event on the seam. |
| **Residual syndrome** | `S_res = S_in ⊕ ∂(E_committed)` — the original syndrome XOR the boundary of committed edges.  On a correctly chosen commit it contains only seam detectors. |
| **Global prediction** | `O_global = ⊕_{e∈E_committed} obs(e)` — XOR of logical-observable labels along committed edges.  Both layers contribute to the same parity. |

`∂(E)` denotes the *boundary* of a set of edges: the set of detectors
incident to an odd number of edges in the set (edges to a virtual boundary
contribute a single detector).  Repeated edges cancel (`e⊕e = 0`).

### 2.2 Layer properties

| Property | Layer 0 (top) | Layer 1 (bottom) |
|---|---|---|
| Temporal boundary | **Open** — an error may leave the window through the time cut at the same cost as the original crossing edge.  Modelled by a temporal virtual node per cut with edges that copy the weight/observable of each original crossing edge. | **Closed** — no temporal virtual; a defect must be matched inside the window. |
| Commit rule | `t_mid ∈ [commit_start,commit_end)` (strict sub-interval). | Fully committed: `commit = window` (no buffer). |
| Input syndrome | `S ∩ V_W` (global defects whose detector lies inside the window). | `(S ∩ V_W) ⊕ S_artificial` where `S_artificial` is the XOR of flipped outside endpoints of all layer-0 commits whose seam lies inside the window. |
| Spatial boundaries | Preserved in both layers (global virtual vertices are always included). |
| Logical observable | Commits to the global parity (same `logical_parity` accumulator). |

Overlapping commits within the same layer are forbidden; they would imply two
windows claiming the same logical correction. The validator in
`window_oracle::validate_window_layers` rejects this. The CLI's executable-shot
adapter additionally requires **exactly one** commit owner for each detector
across both layers; annotations alone may represent non-executable plans.

### 2.3 Generation schemes

The crate supports three equivalent ways to tile `[t_min,t_max)`; they differ
only in how `t_start` and `commit_*` are chosen:

* **Riverlane** (`WindowScheme::Riverlane { gap }`): non-overlapping *A*
  windows of size `w = c+2b` spaced by `gap`.  *B* windows fill exactly the
  gap between *A* commits and have size `gap+2b`.  With `c=b=d, gap=d` both
  layers have size `3d` (Skoric Fig. 3).

* **Sandwich** (`WindowScheme::Sandwich { w,s }`): sliding windows of length
  `w` and step `s`; the core is the middle `s`.  Seams are single-cycle
  boundaries between cores and are decoded as closed layer-1 windows of width
  one cycle (Tan Fig. 2).

* **Contiguous** (`WindowScheme::Contiguous`): the time axis is partitioned
  into `c`-sized blocks; each block's window is the block plus a `b` buffer
  on each side.  Seams are block boundaries.

All three satisfy the invariant that every error chain of length `≤d` is fully
contained in at least one window when `c=b=d`.

## 3. Software oracle

`window_oracle` is the **reference** that a correct decoder must match:

```rust
residual = S_in ⊕ ∂(E_committed)          // artificial + uncorrected original
parity   = ⊕_{e∈E_committed} obs(e)       // global logical prediction
```

`commit_tentative` implements the commit rule above and checks:
* `t_mid` inside commit → committed,
* otherwise discarded,
* overlapping commits → panic (validation error),
* flipped outside endpoints → artificial defects.

`validate_window_layers` checks open/closed invariants and same-layer
commit disjointness.  The oracle is used by the Rust tests and by the
Python prototype to verify hand-worked examples (odd parity, start/end
windows, multiple observables) required by the plan's Sprint 1 exit gate.

## 4. CPU decoder

`ParallelWindowDecoder` (in `src/cpu/blossom/src/parallel_window.rs`) is
the Micro Blossom CPU controller:

1. **`build_window_initializer`** — from the global `MicroBlossomSingle`
   (initializer, positions, virtual vertices) and a `WindowSpec`, build a
   window-local `SolverInitializer`:
   * keep all vertices `v` with `t(v)∈[t_start,t_end)`,
   * always keep spatial virtual vertices,
   * for open windows add two temporal virtual vertices at `t_start`/`t_end`
     and for each global edge crossing the time cut add a local edge
     `(inside, temporal_virtual)` with the same weight/observable,
   * for closed windows omit crossing edges.

2. **Layer-0 decode** — for each top window in parallel:
   * map global defects `S∩V_W` → local syndrome,
   * run an inner MWPM solver (`SolverEmbeddedComb` with
     `support_offloading=false, support_layer_fusion=false` for exactness;
     any `PrimalDualSolver` can be substituted),
   * split its subgraph into committed (midpoint inside commit) vs buffer,
   * update `global_committed` and `logical_parity`,
   * collect `flipped_outside` = boundary of committed edges restricted to
     detectors whose `t` is outside the commit → artificial syndrome.

3. **Layer-1 decode** — for each bottom window:
   * input syndrome = `(S∩V_W) ⊕ (artificial∩V_W)` (XOR),
   * build a closed window graph,
   * run the inner solver (fully committed),
   * update the same global parity accumulator.

The decoder returns `ParallelDecodeResult { committed_edges, logical_parity }`.
Throughput scales linearly with `N_par`; the second layer can start as soon
as its two neighbouring top windows finish (pipelined, see 2209.08552
Appendix D).

## 5. Observable accounting

The toolchain (`src/lib.rs`) stores per-edge logical labels in
`Graph::edge_metadata[*].observables: Vec<usize>`.  The CPU window decoder
receives an aligned `edge_observables: Vec<Option<usize>>` (or
`Vec<Vec<usize>>` for multi-observable codes) from the same source — the
`MicroBlossomSingle::weighted_edges` order is the graph order, not an
arbitrary reordering, and the decoder preserves it.  The validator in the
toolchain rejects parallel edges with conflicting observable labels; the
parallel decoder XORs observables exactly as `Graph::evaluate_correction`
does.

## 6. Relation to the toolchain (Sprint 1)

The circuit-to-graph compiler in `src/lib.rs` already:

* extracts a DEM with `num_detectors`, `num_observables`, and per-error
  `(detectors, observables, probability)`,
* merges duplicate mechanisms, quantises even weights `w = 2·round(scale/2 ·
  ln((1-p)/p))`, and builds a global `Graph` wrapper with `edge_metadata`,
* validates window *annotations* (commit/buffer detector lists, layer labels,
  same-layer commit exclusivity).

The CLI now calls `src/window.rs::build_window_graphs` for annotated IR and
writes the actual local graphs and maps to `PREFIX.windows.json`. With real
syndromes from `--shots` or `--sample-shots`, `src/decoding.rs` converts those
same local graphs into `EncodedWindow`s, instantiates the CPU
`ParallelWindowDecoder::from_encoded`, and verifies global residual/parity before
writing `PREFIX.corrections.json`. It requires complete time slices and explicit
`t` coordinates. Top windows own all edges incident to their commit detectors,
including either commit/buffer seam; buffer-only and rewired outer-cut edges
remain tentative. All real neighbours of a top commit must therefore be in its
window. Bottom windows own their full closed graphs. Every detector and global
edge must have exactly one owner, including the first/last time slices; missing
buffers, unowned edges and conflicting owners are rejected before decoding.
This preserves the local correction parity at every top commit detector and
avoids leaving the edge at a bottom→top seam uncommittable. It does not establish
global minimum weight or matchability of arbitrary residuals in a closed bottom
graph. The adapter also checks the cut graphs' edge endpoints, weights and global
index maps before solving. The original
programmatic `WindowSpec`/`build_window_initializer`
path remains separate; detector-ID annotations are **not** automatically
translated into time-interval specs. Both CPU paths run sequential window loops.

## 7. What remains (Sprints 2–4)

* **Bounded reconfiguration** — pre-allocate a superset graph for the worst-case
  window (Sprint 2).  Runtime updates change only edge weights / enable flags
  and temporal-virtual adjacency under a phase FSM; vertex count and
  communication trees are static.

* **FPGA prototype** (Sprint 3–4, marked TODO): the Scala generator
  `MicroBlossomBusGenerator` must accept a `WindowSpec` and emit a window-sized
  `MicroBlossomBus` with the temporal-virtual handling above, plus a scheduler
  for the two-layer pipeline.  The software oracle's result is the golden
  reference for cycle-level simulation.

## 8. Testing

* `cargo test --locked --lib decoding` — root toolchain's real two-layer
  graph/shot bridge and fail-closed schedule checks, both seam orientations,
  artificial-event handoff and all 16 fault subsets of a five-detector line
  under forward, reversed and uneven detector times.

* `cargo test --release --locked --manifest-path src/cpu/blossom/Cargo.toml --lib parallel_window`
  — programmatic CPU window decoder tests.

* `python3 -m unittest tests.test_parallel_window` — pure-Python seam example
  (no Rust toolchain). With Cargo and Stim installed, run
  `RUN_RUST_TESTS=1 python -m unittest tests.test_rust_cli` for actual CLI
  window cutting, supplied/sampled shots, parity, multi-label edges, and errors.
  This includes the three-window alternating seam fixture, replay of the saved
  tiny shots without resampling, and syndrome consistency on 64 sampled shots
  of a distance-3, 12-round memory with three disjoint top and two bottom windows.
  The memory check is not a statistical logical-error-rate benchmark.
