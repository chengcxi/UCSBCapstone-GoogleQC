# Rust integration and verification

## Architecture

```
compile CIRCUIT/DEM
  └─ embedded tools/stim_export.py, run by selected Python + Stim
      └─ versioned JSON IR on stdout (diagnostics on stderr)
          └─ Rust conversion and weight quantization
              ├─ .ir.json
              ├─ .graph.json (observable/provenance sidecar)
              └─ .syndromes (upstream parser input)
                  └─ optional Micro Blossom parser → .hardware.json
```

`export IR` starts at the Rust conversion step and needs neither Python nor Stim.
The Python bridge is included at Rust compile time (`include_str!`), so a binary
installed with Cargo works outside the checkout. Python is a runtime dependency
only for the `compile` subcommand; install the pinned requirements into the selected
interpreter. No shell is used to invoke either external executable.

Conversion and optional upstream validation happen before existing output files
are changed. Files are staged in a unique sibling directory and individually renamed
to their destinations; the staging directory is removed on success or error. This
protects against conversion/parser failures, not against partial publication caused
by disk failures or simultaneous writes to the same output prefix. Use distinct
prefixes for concurrent jobs.

## Reproducible standard build

- Rust toolchain: 1.99.0, edition 2024, `rustfmt` and `clippy` components.
- Cargo dependencies: registry packages pinned by the checked-in `Cargo.lock`.
- Runtime Python packages: `requirements.txt`.
- Test-only native decoder: `fusion-blossom==0.2.13` in `requirements-dev.txt`.

```sh
cargo build --locked
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
RUN_RUST_TESTS=1 .venv/bin/python -m unittest discover -s tests -v
```

### Restricted-workspace note

This workspace cannot access `static.rust-lang.org`, `index.crates.io` or the
crates download host. For local verification only, Rust 1.88.0 components were
obtained from the third-party `@rustbin/*-1.88.0-x86_64-unknown-linux-gnu` npm
repackages. The compiler reports `rustc 1.88.0 (6b00bc388 2025-06-23)`.
Dependency sources were obtained from matching upstream GitHub release tags in a
sandbox-only Cargo directory-source cache. Workspace/path dependency manifests were
normalized there; registry package checksums for the generated lockfile came from
the crates.io index on GitHub. These source snapshots are not claimed to be
byte-for-byte verified `.crate` archives.

The project does **not** depend on npm, sandbox-normalized package sources or sandbox paths.
Those bootstrap artifacts/configuration are outside the repository and not part
of the deliverable. CI uses the official rustup toolchain and registry packages
with `--locked`; its run remains the authoritative check of the standard clean
network build. No CI run is claimed in these local results.

After the 1.99 upgrade, the pinned compiler could not be downloaded either. The
old toolchain above was used only for compatibility checks with
`--ignore-rust-version`; the Python suite used a sandbox-only wrapper that inserted
that flag for Cargo build/run commands. Normal development and CI must use 1.99
without a bypass. See [the migration verification record](RUST_1_99.md). A real 1.99
build/format/lint run has not been claimed.

## What was verified locally (older-compiler compatibility checks)

- 12 Rust unit tests, formatting, and Clippy with warnings denied on the available
  older compiler (not a 1.99 validation).
- 27 Python/integration tests passed; only the full Micro Blossom test skipped.
- Existing two-stage pipeline plus the new integrated CLI.
- Embedded bridge invoked outside the checkout and paths containing spaces.
- DEM/window inputs, weight-scale selection, explicit correlation opt-in.
- Invalid arguments/IR/circuits, missing Python and upstream failure diagnostics.
- Preservation of existing files and cleanup after parser failure/mismatched output.
- **Real native Rust loader:** Fusion Blossom 0.2.13 `ErrorPatternReader` loads the
  emitted Syndrome Pattern file and preserves vertex/edge/boundary data.
- **Real native decoder:** `SolverSerial` reconstructs correct observable parity
  for 128 seeded tiny-circuit samples; distance-3, three-round memory produces a
  25-vertex/78-edge graph, with correction-syndrome consistency on 64 seeded samples.

The memory test explicitly permits the independent-component approximation.
These tests are not a statistically meaningful logical-error-rate benchmark, not
a window decoder and not a proof of arbitrary-circuit equivalence. The tiny fixture
has an exactly checkable logical parity; the memory fixture checks syndrome
consistency, not equality to the sampled physical logical outcome.

## Full Micro Blossom handoff (optional)

Inspected/pinned revision:
`shsym/micro-blossom@3dd1817eecdb4bf9899433108d9a2edccf198c8e`.
The native Fusion Blossom wheel tests the parser-input format but is not a substitute
for this full hardware-metadata path.

In an existing upstream checkout at that revision:

```sh
cargo build --release --locked --manifest-path src/cpu/blossom/Cargo.toml \
  --bin micro_blossom
```

Then, from this project:

```sh
MICROBLOSSOM_BINARY=/absolute/path/to/upstream/src/cpu/blossom/target/release/micro_blossom \
  RUN_RUST_TESTS=1 .venv/bin/python -m unittest discover -s tests -v
```

Use an absolute executable path because integration tests deliberately run outside
the repository. Without that environment variable, only the full Micro Blossom
parser test is skipped when `RUN_RUST_TESTS=1` is set. Normal CI executes native
reader/decoder tests; manually dispatch the workflow with `upstream: true` to also
build the pinned Micro Blossom parser and exercise hardware-JSON generation.

The upstream build/full-parser test has not been executed here. Protocol-stub tests
check invocation, failure propagation, index preservation and structural validation
with synthetic tree/offloader fixtures only. FPGA
simulation, synthesis, runtime graph changes and residual syndromes remain separate
Sprint 1/2 acceptance tasks.
