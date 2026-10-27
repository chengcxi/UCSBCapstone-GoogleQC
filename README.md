# Micro Blossom parallel-window toolchain

Compile Stim circuits and detector error models into graph inputs for
[Micro Blossom](https://github.com/shsym/micro-blossom). With an explicit
window plan and syndrome shots, the CLI now runs the two-layer CPU decoder and
publishes verified corrections. Window-aware FPGA generation remains future work.

**Status:** The project targets **Rust 1.99.0 / edition 2024**. CI runs Rust
formatting, tests, docs and Clippy, the Python/Stim end-to-end decoding tests,
and an installed-binary smoke test on Rust 1.99. The CPU-only window path is
implemented; concurrent FPGA window decoding is not. Full Micro Blossom
hardware-JSON generation has an optional parser handoff and test, but the
upstream parser build is not part of the default CI run.

**Parallel window decoding (Sprint 2, CPU):** Implemented.  The CPU decoder in
`src/cpu/blossom/src/parallel_window.rs` and the oracle in
`src/cpu/blossom-nostd/src/window_oracle.rs` realise the two-layer scheme
from [Skoric *et al.* 2209.08552](https://arxiv.org/abs/2209.08552) and
[Tan *et al.* 2209.09219](https://arxiv.org/abs/2209.09219): top-layer
windows have open temporal boundaries, a commit region and buffers, and emit
residual (artificial) syndromes on the seam; bottom-layer windows have
closed boundaries and decode the residuals.  Both layers commit to a single
global observable parity.  See `docs/PARALLEL_WINDOW.md`, the hand-worked
Python prototype `parallel_window_prototype.py`, and the CPU Rust tests
`cargo test --locked --manifest-path src/cpu/blossom/Cargo.toml --lib parallel_window`.

**FPGA window generator (Sprints 3–4):** Marked TODO.  The toolchain's
`src/window.rs` builds window-sized hardware graphs (the golden reference);
the Scala generator `MicroBlossomBusGenerator` still needs a window-aware
front-end (see `src/hardware.rs` notes).

See [the feasibility review and sprint gates](docs/PLAN.md),
[the data contract](docs/FORMAT.md), and [integration verification notes](docs/INTEGRATION.md).

## Setup

Requires Python 3.11+ and Rust 1.99.0 (pinned in `rust-toolchain.toml`). With rustup
installed, Cargo selects the pinned toolchain automatically. `Cargo.lock` is tracked;
use `--locked` for reproducible dependency resolution.

```sh
python -m venv .venv
. .venv/bin/activate
python -m pip install -r requirements-dev.txt
cargo build --locked
```

Use `requirements.txt` instead if you only need compilation and CPU window
decoding, not Python reference-decoder tests. The Python `fusion-blossom`
package is a test dependency; the Rust binary now links the existing
`micro-blossom` CPU decoder and its Rust Fusion Blossom dependency.

## One-command circuit → graph compilation

```sh
cargo run --locked -- compile examples/tiny.stim \
  --output artifacts/tiny --python .venv/bin/python
```

The Rust CLI embeds the Python/Stim bridge, invokes it without a shell, validates
its IR, converts the graph and writes:

- `tiny.ir.json`: normalized DEM, full detector coordinates, observable labels,
  grouped correlated components, input hash and Stim version.
- `tiny.graph.json`: initializer, visualization positions and index-aligned edge metadata.
- `tiny.syndromes`: graph-only **Syndrome Pattern v1.0** parser input; no syndrome
  shots are fabricated by default. This is **not** the final Micro Blossom hardware JSON.

With a window plan, `PREFIX.windows.json` also holds the cut local graphs and
index maps. Add `--shots` or `--sample-shots` to decode real supplied or sampled
syndromes into `PREFIX.corrections.json`.

Output directories are created automatically. `--python` accepts an executable
path/name; alternatively set `MICROBLOSSOM_PYTHON`. Default: `python3`.

```sh
# Existing DEM input with explicit window annotations:
cargo run --locked -- compile examples/seam.dem --kind dem \
  --windows examples/windows.json --output artifacts/seam --python .venv/bin/python

# Pure Rust conversion of an existing normalized IR (does not invoke Python):
cargo run --locked -- export artifacts/tiny.ir.json --output artifacts/reexport

# Optional installation; the resulting binary does not need the checkout:
cargo install --locked --path .
microblossom-toolchain --help
```

The JSON files under `examples/` are **input fixtures**, not generated outputs.
When copying this toolchain into another repository, commit the strict window
plans (`windows.json`, `tiny-windows.json`), the illustrative
`parallel_window_windows.json` and `seam-shots.json`. If a destination ignores
`*.json`, add exceptions or stage these fixtures with `git add -f`.

The legacy `cargo run --locked -- INPUT.ir.json PREFIX [--allow-correlated]` syntax
also works. To save Stim's textual DEM separately, use the lower-level bridge:

```sh
python tools/stim_export.py examples/tiny.stim \
  --output artifacts/tiny.ir.json --dem-output artifacts/tiny.dem
```

## Micro Blossom parser handoff

After building an upstream `micro_blossom` executable:

```sh
cargo run --locked -- compile examples/tiny.stim \
  --output artifacts/tiny --python .venv/bin/python \
  --micro-blossom /absolute/path/to/micro_blossom
```

The CLI runs upstream `parser INPUT --graph-file OUTPUT` to derive communication
trees and hardware metadata, then writes `tiny.hardware.json`. It rejects output
that changes vertex IDs, edge ordering, weights or virtual boundaries, because
those changes would invalidate the observable sidecar. It also checks communication-tree structure, positions, growth-vector dimensions
and offloader references. These are structural checks, **not** proof of valid growth
bounds/offloading behavior, hardware timing, or all derived metadata.

Keep `.graph.json` alongside the hardware graph: the upstream parser input does not
carry logical-observable labels. Predictions require the selected correction edges
and their labels. Conversion/parser failures leave previous outputs intact; staging
files are cleaned up. Publication uses per-file renames, not an all-files transaction.
An invocation without `--micro-blossom` does not refresh an existing hardware file.

See [integration notes](docs/INTEGRATION.md) for the pinned upstream revision and
optional real-parser test. CI also offers a manually dispatched upstream build job;
that job has not yet been verified.

## Window annotations

The annotation plan explicitly lists detector IDs for each window's `commit` and
`buffer` regions. Layer 0 is labeled temporally open; layer 1 is labeled temporally
closed. Seam and outer-crossing components are identified by source indices. Rust revalidates
these references even when importing IR directly, rejects conflicting boundary
labels and forbids overlapping commit regions within the same layer. Buffers and
cross-layer membership may overlap.

**Sprint 1:** annotations were validated but did not modify graph connectivity.
**Sprint 2:** window graph construction (copying vertices/edges by `t`,
duplicating spatial virtuals, synthesising temporal virtuals for open windows)
and the residual/observable oracle are implemented in `src/window.rs` (toolchain
golden reference) and `src/cpu/blossom-nostd/src/window_oracle.rs`
(residual `S ⊕ ∂(E_committed)` and global `⊕ obs(e)`).  Physical code
boundaries are distinct from artificial temporal window boundaries; both layers
preserve the former.

Run the hand-worked prototype without a Rust toolchain:

```sh
python3 parallel_window_prototype.py
python -m unittest tests.test_parallel_window -v
```

Cut the graph into local open/closed windows and decode **actual** syndromes:

```sh
# Supplied DEM shots: global detector IDs and optional measured observables.
cargo run --locked -- compile examples/seam.dem --kind dem \
  --windows examples/windows.json --shots examples/seam-shots.json \
  --output artifacts/seam --python .venv/bin/python

# Or sample a Stim circuit reproducibly in the embedded Python bridge.
cargo run --locked -- compile examples/tiny.stim \
  --windows examples/tiny-windows.json --sample-shots 24 --seed 7 \
  --output artifacts/tiny-windowed --python .venv/bin/python

# Re-decode saved annotated IR and supplied shots without Python/Stim.
cargo run --locked -- export artifacts/seam.ir.json \
  --shots examples/seam-shots.json --output artifacts/seam-redecoded
```

`--windows` always calls `build_window_graphs` and writes `PREFIX.windows.json`
(local `initializer`, `positions`, `detector_map`, `edge_map` and observable
sidecars). It does **not** invent a syndrome: decoding and
`PREFIX.corrections.json` require `--shots` or `--sample-shots`. The latter
supports circuit and DEM input; both sources retain real Stim observable flips.
No upstream `micro_blossom` executable or FPGA is needed for the CPU path.

`--shots` expects `{"shots":[{"defects":[0,2],"observables":[0]}]}`. The
`observables` field is optional; when present it is compared with the decoded
`predicted_observables` to set `logical_error`. Correction edges refer to the
**global** `.graph.json` indices. Each shot also records a verified empty
`residual_syndrome`; unsolvable shots or incomplete window schedules fail before
publishing outputs. For decoding, every detector needs a time coordinate,
windows must cover complete contiguous time slices, and every detector must
belong to exactly one commit across both layers. The CPU window loop is sequential
(today's *software* end-to-end path, not hardware parallelism).

`examples/parallel_window_windows.json` illustrates a richer window description,
but its extra keys are not accepted by the strict `--windows` input loader.
See [architecture diagrams](tutorial/README.md) and
[the parallel-window scheme](docs/PARALLEL_WINDOW.md) for the full geometry.

## Supported conversion subset

- Deterministic-detector circuits whose DEM extraction succeeds without gauge or
  disjoint-error relaxation; existing DEM files can also be imported.
- Nonzero mechanisms with one or two detectors, probability at most 0.5.
- One-detector mechanisms become edges to an explicit physical virtual vertex.
- Independent duplicate mechanisms with identical detector **and observable** support
  merge using `p + q - 2pq`. Conflicting logical labels on parallel edges are rejected.
- Weights are even integers: `2 * round(scale * ln((1-p)/p) / 2)`, with checked signed
  32-bit bounds. Default scale: 100; change with `--weight-scale NUMBER`.
  Zero-probability mechanisms are omitted; p=0.5 produces zero weight.
  Finite resolution may also round small positive weights to zero; the CLI warns.
  Log odds are computed as `ln_1p(-p) - ln(p)` to avoid overflow at tiny probabilities.

The converter rejects hyperedges, undetectable logical faults, out-of-range IDs and
unsupported probability ranges. Correlated `^` components are rejected by default.
Pass `--allow-correlated` only to explicitly approximate them as independent graph
edges; the output records the approximation and the CLI emits a warning.
This is not a correlation-aware decoder and may change logical error performance.

## Check correction parity

Given `shot.json` containing `{"defects": [0], "edges": [0]}`:

```sh
cargo run --locked -- check-correction artifacts/tiny.ir.json shot.json
```

This reports the correction's detector boundary, observable flips, and the residual
`input syndrome XOR correction boundary`. Repeated edge selections cancel by XOR;
invalid edge/defect IDs and duplicate input defects are rejected. A nonempty
residual produces exit status 1. Use `--allow-correlated` when the IR needs that
explicit conversion policy. This algebraic check neither chooses an MWPM correction
nor implements the two-layer **seam** residual algorithm. Edge IDs must refer to
this compiler's deterministic graph ordering, not an arbitrary decoder graph.

## Tests

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
RUN_RUST_TESTS=1 .venv/bin/python -m unittest discover -s tests -v
```

See [Rust 1.99 migration notes](docs/RUST_1_99.md) for the changes and exact
verification limitations. The Python suite includes Rust CLI tests, native Fusion Blossom loading, 128 tiny
circuit logical-prediction checks, and correction-syndrome checks on 64 distance-3
memory samples. A stub parser tests subprocess/error handling only; it is not
presented as a real hardware-graph validation. The full upstream test is enabled
separately with `MICROBLOSSOM_BINARY`.

Generated artifacts, virtual environments and proprietary data must not be
committed. `artifacts/` and Rust build outputs are ignored.
