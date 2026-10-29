# Sprint 1 data contract (schema version 1)

## Normalized DEM IR

`tools/stim_export.py` uses Stim as the authoritative parser for circuit and DEM
syntax. `flattened()` expands repeat blocks and detector shifts; coordinates come
from `get_detector_coordinates()` so coordinate shifts are resolved too.

```json
{
  "schema_version": 1,
  "num_detectors": 2,
  "num_observables": 1,
  "detector_coordinates": [[0, 0, 0], [0, 0, 1]],
  "errors": [{
    "probability": 0.1,
    "components": [{"detectors": [0, 1], "observables": [0]}]
  }]
}
```

- Detector indices are global, contiguous `0..num_detectors`; missing coordinates
  are empty arrays, not invented spatial locations. Unknown fields in the version-1
  DEM, error and component records are rejected rather than silently ignored.
  IDs are validated even for zero-probability mechanisms.
- IDs within each component are parity-normalized (duplicates cancel) and sorted.
- Each `error` is one original independent DEM mechanism. Multiple components within
  it were separated by `^`; they are correlated, **not independent**.
- `provenance` records Stim version, input kind and SHA-256 of input text re-encoded
  as UTF-8 after reading. This is a content provenance aid, not a signed artifact.
- Optional `window_annotations` carries the annotation schema below.
- Flattening can use considerable memory for long repeated circuits. A streaming
  compiler is not implemented; use bounded fixtures initially.

## Window annotation schema

Input is a JSON object with exactly one nonempty `windows` list. Each entry has exactly:
`id` (unique nonempty string), `layer` (0 or 1), `commit` (nonempty detector list),
`buffer` (detector list; empty for layer 1). Both lists must be unique, in range,
and disjoint within a window. Same-layer commit regions must be disjoint;
buffer overlap is allowed. The annotation-only schema also accepts cross-layer
commit overlap, but **executable shot decoding rejects it** because the two
layers would claim the same detector.

Output adds `temporal_boundary`, `commit_buffer_seams` and `outer_crossings` to each
entry. Crossings reference `{ "error": index, "component": index }` in the IR.
Rust independently validates the annotation schema, layer/boundary agreement and
recomputed crossing references on every import, including direct IR export.
The **IR metadata** sets `annotation_only: true` and never changes the global
graph. The CLI now *separately* calls `build_window_graphs` for annotated IR and
writes `PREFIX.windows.json`. Membership alone still does not prove a valid
decoding schedule.

The physical boundary of a code is not an omitted detector outside a time window.
A one-detector DEM error has a physical boundary endpoint and is not automatically
an artificial temporal cut. A closed layer-1 *temporal* boundary must not remove
valid physical boundary edges. For shot decoding, the runtime requires complete,
contiguous time slices with explicit `t` coordinates and coverage by commit
regions. It commits every selected edge touching a top commit detector, requiring all
real endpoints inside commit plus buffer. Insufficient buffers are rejected.
After layer 0, residuals inside top commits or outside layer-1 coverage fail
before layer 1 runs. A single fully committed full-graph window is supported. Ambiguous
ownership, uncovered detectors and unmatched residual syndromes are errors.

## Rust output

`OUTPUT_PREFIX.graph.json` is a project wrapper, **not** `MicroBlossomSingle` JSON:

- `initializer`: `vertex_num`, `weighted_edges` as `[left,right,even_weight]`, and
  `virtual_vertices` as integer IDs. Detector IDs remain unchanged. If needed, one
  shared physical virtual vertex has ID `num_detectors`.
- `num_detectors`, `num_observables`, `detector_coordinates`: complete source counts
  and full coordinate vectors, including axes beyond the first three. These are
  additive version-1 sidecar fields.
- `positions`: visualization-only `{i,j,t}` values from the first three detector
  coordinates, zero-filled if missing. This does not establish surgery semantics.
- `edge_metadata`: aligned with `weighted_edges`, retaining global detector support,
  logical observable IDs, effective probability and original `(error,component)`
  source pairs. Sorting by detector support gives deterministic edge ordering.
- `weight_scale`: defaults to 100; configurable with `--weight-scale` in the CLI or through the Rust library.
- `correlated_components_approximated`: true only if nonzero split mechanisms were
  accepted with explicit opt-in. Original correlation group IDs remain in sources.
- `provenance`, `window_annotations`: copied from the IR.

Keep the original IR for probability/correlation audit. The graph sidecar also
retains full coordinates; their physical/surgery meaning is not inferred.
Observable prediction is the XOR of logical labels on correction edges. It is
not determined by shortest-path weight or detector IDs alone.

`OUTPUT_PREFIX.syndromes` contains three lines:

1. `Syndrome Pattern v1.0 microblossom-toolchain`
2. Serialized initializer.
3. Serialized visualization positions.

There are no shot records. This is the input to upstream `micro_blossom parser`;
upstream derives hardware-specific fields instead of this project inventing them.
The upstream parser uses even weights. The integrated CLI retains `.ir.json` alongside these outputs.

With `--windows` on `compile` (or annotated IR on `export`) the CLI also writes
`OUTPUT_PREFIX.windows.json`, with `schema_version: 1` and a `windows` list.
Each local `WindowGraph` includes its `window_id`, `layer`, `temporal_boundary`,
local `initializer`, `positions`, `edge_metadata`, `detector_map` (local vertex
to global detector, `null` for virtuals), and `edge_map` (local edge to global
edge). Global `.graph.json` remains unchanged. No shots are fabricated by
window cutting alone.

With `--shots INPUT.json` or `compile --sample-shots COUNT` the CPU decoder
writes `OUTPUT_PREFIX.corrections.json` after checking the **global** residual
and observable parity of *every* shot. Explicit shot input has the strict shape:

```json
{"shots": [{"defects": [0, 2], "observables": [0]}]}
```

`observables` is optional and, if present, means actual measured flips, not the
prediction. IDs must be unique and in range. The output has `schema_version: 1`,
`num_detectors`, `num_observables`, and a `shots` list. Each shot has `defects`,
`committed_edges` (global edge IDs), `predicted_observables`, and an empty
`residual_syndrome`. If actual labels were supplied/sampled it also has sorted
`actual_observables` and `logical_error` (their mismatch with the prediction).
An invalid shot or nonzero residual fails the invocation before any files are
published. `--sample-shots` takes a reproducible `--seed` (default 0), uses
Stim's circuit/DEM sampler and is compile-only. `--shots` can also run on
annotated IR via pure-Rust `export`.

The converter bounds individual weights,
not the resource limits, total growth or accumulator widths of a particular FPGA;
those must be checked before hardware generation.

## Not yet guaranteed

- Full Micro Blossom hardware-JSON round-trip. Native Fusion Blossom 0.2.13
  ErrorPatternReader deserialization and SolverSerial decoding are now tested.
  Optional `--micro-blossom` handoff checks index preservation, tree structure,
  positions, growth-array shape and offloader references; full upstream build
  still needs to be exercised.
- Hardware resources/timing, admissible zero-weight behavior across all backends.
- Logical-error equivalence after weight quantization or correlation approximation.
- Runtime mutation of weights, virtual flags, offloading or parity reporters.
- Concurrent FPGA scheduling and real-time transport. The implemented CPU
  two-layer path uses sequential window loops, verified graph/parity outputs and
  strict time-slice plans; it does not establish correctness for arbitrary
  lattice-surgery geometries.
- General lattice-surgery and defect-adapted circuit generation.

## Correction parity (not window residual decoding)

`Graph::evaluate_correction(edges, defects)` and CLI `check-correction` compute
XOR over selected edges' detector and observable supports. Edges may repeat and
cancel; input defect IDs must be unique and refer to physical detectors (not
virtual vertices). Outputs are sorted `detectors`, `observables`, and
`residual_syndrome` arrays. The last is the input syndrome XOR correction boundary.
No measurement/logical outcome is inferred from the residual alone. A zero residual
proves syndrome consistency, not maximum likelihood or physical logical correctness.
