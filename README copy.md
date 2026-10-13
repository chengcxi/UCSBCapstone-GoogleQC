# Micro Blossom parallel-window toolchain

Sprint 1 foundation for compiling Stim circuits into graph inputs for
[Micro Blossom](https://github.com/shsym/micro-blossom).

**Status:** The project targets **Rust 1.99.0 / edition 2024**. Graph conversion,
strict IR/window validation, correction parity and native Fusion Blossom integration
pass compatibility tests with the locally available older compiler. The sandbox
blocks the 1.99 download, so a real 1.99 build/lint run remains pending in CI.
Full Micro Blossom hardware-JSON generation has an optional parser handoff and test,
but the upstream binary has not yet been built here. Window cutting, seam-residual
decoding, FPGA changes and hardware validation are **not implemented**.

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

Use `requirements.txt` instead if you only need compilation, not native decoder
tests. Fusion Blossom is a **test dependency**, not a dependency of the Rust binary.

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
  shots are fabricated. This is **not** the final Micro Blossom hardware JSON.

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

**Annotations do not modify graph connectivity, compute residual syndromes, enforce
global schedule coverage or assign committed observables.** Physical code boundaries
are distinct from artificial temporal window boundaries. Actual window graph
construction and a software residual oracle are the next work items.

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
