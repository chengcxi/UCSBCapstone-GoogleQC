# Micro Blossom CPU workbench

A deliberately plain, MATLAB-like scientific UI for the **Stim → Rust**
toolchain. The **Decoder** tab replays verified CLI captures on one global
detector graph; the **Benchmark** tab compares two *real CPU implementations*
and never invents latency data.

## Open the preview

```sh
cd demo
npm ci
npm run dev
```

Open Vite's URL (port 4173 by default). The Decoder tab uses real saved CLI artifacts: the Stim circuit supplies 24 seed-7 sampled shots, while the seam DEM supplies five explicit shots. Pick a shot and step through the top/open and bottom/closed windows. The diagram draws each global graph detector and weighted edge once, with window membership overlaid on the same IDs. A dashed T marker describes a **local** temporal cut, not a vertex in the global graph. Exported tentative solver edges do not exist, so the UI does not depict any.

## Generate datasets directly in CPU Benchmark

The **Benchmark** page includes a **Generate a benchmark dataset** panel.
Choose distance, rounds, shots and noise; seed, basis and commit/buffer widths
are under **Seed and window settings**. Set the warmup and measured passes
in the normal Configuration panel, then click **Generate and benchmark**.

The local server runs Stim and the Rust toolchain and measures full-graph
Micro Blossom against sequential-window Micro Blossom. A successful run is
selected in the normal Dataset menu and fills the existing latency chart,
accuracy summary and per-shot comparison table. There is no separate generator
page or graph visualization step.

Generated datasets persist in this browser. Select one and use **Run CPU
comparison** to regenerate its same seeded samples and measure again with the
current warmup/repeat settings. **Export dataset JSON** retains circuit,
graph, inputs, settings and timing report; **Import benchmark dataset JSON**
loads that portable dataset back into the same benchmark page. **Save JSON**
and **Load report JSON** remain available for timing reports alone (select
the matching dataset before loading a report). Imported timings are not
independently attested.

Set up generation from the repository root:

```sh
.venv/bin/python -m pip install -r requirements-dev.txt
cargo build --release --locked
cargo build --release --locked --example window_accuracy
npm --prefix demo run dev
```

The server prefers `.venv/bin/python` and the release CLI. Executable overrides:
`MICROBLOSSOM_PYTHON`, `MICROBLOSSOM_TOOLCHAIN_BINARY` and
`MICROBLOSSOM_ACCURACY_BINARY`. Restart Vite after setup.

Interactive limits are 1,500 global detectors, 400 detectors per window, 256
shots and 5,000 measured samples per method. The four configured noise channels
use the specified probability. Correlated DEM components are explicitly
approximated as independent edges. Invalid window corrections stop the timing
comparison with an explicit error; no samples are silently dropped and no
partial report is shown. Generation requires the local development server;
saved datasets can be imported in a static preview.

The Benchmark tab opens its seam example with a **real release-binary CI
measurement** from [GitHub Actions run 40595098673](https://github.com/chengcxi/microblossom-parallelism/actions/runs/40595098673),
commit `c486703`. Both CPU methods match the supplied logical-observable truth
on 5/5 shots (4/4 nonempty); this small shared-runner capture is not a
representative latency or logical-error-rate claim. The tiny dataset starts
without timing data. A corrections capture alone is *not* a
timing measurement. Build the Rust binary to measure your own CPU, or use
**Load report JSON** to inspect a CLI report produced elsewhere.

The browser checks imported reports' schema, sample counts, statistics,
explicit accuracy (if present) and global correction parity against the
matching graph; imported timings remain user-provided, not independently
attested. No browser JavaScript decoder, fake timings, or FPGA projections are
used. The checked-in `public/fixtures/seam.benchmark.json` has SHA-256
`e7c3768c09a8a731610f567c731aab31394b639602c3a0ffa94e059506863d6f`;
the CI run also publishes it as the `cpu-benchmark-seam` artifact. Rerun times
will vary with CPU and load.

## Run the CPU benchmark

From the repository root, with Rust 1.99 available:

```sh
mkdir -p artifacts
cargo run --release --locked -- benchmark demo/public/fixtures/seam.ir.json \
  --shots examples/seam-shots.json --warmup 2 --repeats 20 \
  --output artifacts/seam.benchmark.json

# Restore the original seed-7 Stim inputs from the CLI capture. Only
# actual_observables are used as ground truth, never predicted_observables.
node demo/scripts/benchmark-shots.mjs \
  demo/public/fixtures/tiny.corrections.json artifacts/tiny-shots.json
cargo run --release --locked -- benchmark demo/public/fixtures/tiny.ir.json \
  --shots artifacts/tiny-shots.json --warmup 2 --repeats 20 \
  --output artifacts/tiny.benchmark.json
```

The input is normalized, annotated **IR** plus a `{"shots":[{"defects":[0,2],"observables":[0]}]}` shot batch. The benchmark command needs neither Python nor Stim; those are needed only to compile/sample new inputs. It writes one JSON report atomically after both paths pass the global parity check. To time the CLI from the UI, build a release binary and start Vite with its location:

```sh
cargo build --release --locked
MICROBLOSSOM_TOOLCHAIN_BINARY="$PWD/target/release/microblossom-toolchain" \
  npm --prefix demo run dev
```

**What is compared.** Baseline = `SolverEmbeddedComb` on the full global
Micro Blossom graph, in software. Window = the existing `ParallelWindowDecoder`,
solving the open top cut followed by the closed bottom cut **sequentially on a
CPU**. Both paths use the same edge weights, shots, solver type and
disabled-offloading configuration.

**Timing.** Per-shot monotonic timing covers decoder calls: CPU solver
construction, the solve and subgraph extraction, **plus local routing and seam
stitching in the window path**. Graph conversion, Stim, external parity
verification, setup and file I/O are excluded. Method order alternates by shot
and pass. The report stores raw nanosecond samples, median, p95, mean, min/max,
host/profile metadata, actual versus predicted logical flips, and prediction
disagreements.

**Logical accuracy.** For each CPU method, `correct / total` truth-labeled input
shots whose *entire* predicted observable-flip set matches the actual set. Each
input shot counts once regardless of warmups or measured passes; shots with
`observables` omitted are excluded and yield `null`/N/A if none have truth.
The **Nonempty** toggle excludes empty syndromes from both the latency view and
its accuracy denominator. Clearing the detector syndrome is a separate,
mandatory parity check, **not** an accuracy score. Older schema-v1 CLI reports
without the additive `logical_accuracy` field still import: the browser derives
it from their per-shot actual/predicted arrays and checks any explicit accuracy
field for consistency.

Tiny fixture graphs are useful for demonstrating the method, **not** for claims about hardware throughput. Re-run on larger workloads and a dedicated machine for performance analysis. CPU ratio is not an FPGA speedup. Parallel FPGA window decoding and its scheduler are not implemented or measured; that result is explicitly **NOT MEASURED**.

## Rerun the full Stim → decoder pipeline

For the Decoder tab's **Run CLI locally** action, install Stim and build the default binary:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r requirements.txt
cargo build --locked
cd demo && npm ci && npm run dev
```

The local Vite development API only accepts the fixed repository scenarios. `/api/run` executes the CLI and loads `.ir.json`, `.graph.json`, `.windows.json`, `.corrections.json` and `.syndromes`. `/api/benchmark` derives the already-captured shot *inputs* from `.corrections.json`, then executes the Rust benchmark on the fixed matching IR; it requires the binary but not Python. Neither endpoint accepts a browser-provided filesystem path or shell command. Set `MICROBLOSSOM_TOOLCHAIN_BINARY` and `MICROBLOSSOM_PYTHON` before starting Vite to override defaults.

To refresh and verify the saved decoding captures:

```sh
bash demo/scripts/capture.sh demo/public/fixtures
npm --prefix demo test
npm --prefix demo run build
```

The saved fixture tests verify graph and window indices, global parity and the provenance of the Stim source. The visual split of a final correction is derived only from committed global edge IDs and Rust's detector-incident ownership rule in `src/decoding.rs`: a top window owns edges touching its commit detectors, including edges into either buffer. Concurrent CPU or FPGA window execution is **not** depicted. A static production build serves the saved decoder replay and can **import** a real benchmark report; the two live API routes only exist on the local Vite development server. The Rust toolchain itself does not depend on Node.
