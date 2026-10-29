# Incident-edge commitment validation

Top windows commit every selected edge touching a commit detector. All real endpoints must be inside the window; insufficient buffers and duplicate top ownership are rejected. Before layer 1, residuals inside top commits or outside bottom coverage fail with detector/window context. A full-graph top commit needs no bottom window.

Validation: 27 toolchain library tests, 6 oracle tests, 20 UI tests, and the production UI build passed. Tests include the three-detector middle commit, both seam directions, experiment endpoints, insufficient buffers, duplicate top edge claims, and a genuinely fully committed full-graph window (all eight syndromes and logical parity).

## Paired buffer sweep

Distance 5 rotated surface-code Z memory, 29 rounds, 1,000 identical circuit-sampled shots per configuration, seed 20261027, channel noise 0.003, commit width 5 slices. Correlated DEM components are approximated; reference Fusion Blossom uses the same quantized graph. Release window solver is Micro Blossom's CPU SolverEmbeddedComb. This measures the repaired implementation; it is not a before/after measurement of the old convention.

| Buffer slices per side | Logical accuracy | Rejected shots | Nonzero final residuals | Predictions differing from full graph |
| --- | --- | --- | --- | --- |
| 1 | 97.7% | 0 | 0 | 7 |
| 3 | 97.8% | 0 | 0 | 2 |
| 5 | 98.0% | 0 | 0 | 0 |

Full-graph reference: 98.0% (20 logical failures). The 5-slice window buffer matched every reference prediction on this sample. These finite-sample results do not establish universal equality with full-graph decoding or monotonic accuracy with buffer size. Residual validity alone does not prove logical accuracy.

A smaller distance-3, 11-round, 200-shot sweep with buffers 1, 2, and 3 returned 97.0% for each, matching every reference prediction with no rejected shots.

Reproduce each distance-5 row after building the release binary and `window_accuracy` example:

```sh
.venv/bin/python tools/benchmark_window.py --distance 5 --rounds 29 --shots 1000 --buffer-slices 5 --workers 2 --batch-size 100 --output artifacts/repair-window/release-d5-buffer-5
```

Change `--buffer-slices` and the output suffix for the other rows. Raw summaries, labeled shots, corrections and circuit inputs are in `artifacts/repair-window/`. The summary's inherited `host_patch` text describes an older host printing workaround, not the commitment repair; binary hashes identify the measured builds.

## Additional edge-coverage regression tests

Imported from `fix/window-commit-boundaries`: five-detector alternating top/bottom/top windows, all 16 fault subsets under forward, reversed, and uneven detector times, missing owned cut edges, and unowned edges between closed bottom windows. The CLI integration suite also exercises both seam directions, saved-shot replay, and a 64-shot distance-3 memory with three top and two bottom windows. Existing pre-layer-1 residual checks, incident-edge oracle, and fully committed full-graph tests remain in place.
