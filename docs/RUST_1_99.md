# Rust 1.99 migration and semantic hardening

## Version and scope

The [official Rust 1.99.0 release](https://github.com/rust-lang/rust/releases/tag/1.99.0)
was checked through GitHub on 2026-10-13; it was published on 2026-10-01 and was
reported as the latest stable release. The project now pins:

- Toolchain and both CI jobs: **1.99.0**.
- Minimum supported Rust version: **1.99**.
- Edition: **2024** (an explicit migration; a compiler upgrade alone does not change editions).
- Crate lint policy: `unsafe_code = "forbid"` for the project's own targets.
- Existing registry dependency versions remain in `Cargo.lock`; this upgrade does
  not gratuitously change third-party libraries or their behavior.

## Which release changes apply?

The code does not need C variadics, naked functions, inline assembly, raw-pointer
layout queries, `UnsafeCell`, raw allocation parts or pinning changes. Introducing
those interfaces here would add risk without functionality. The new boxed-array
iteration, `VecDeque::retain_back`, file timestamp APIs and inherited workspace
feature overrides also have no current use site.

Subprocess diagnostics already use borrowed `String::from_utf8_lossy` views of
captured output; replacing them with the new owned-lossy APIs would not remove a
needed allocation in this flow. Likewise, the new built-in Cargo `debug` profile
has the same behavior as `dev` in 1.99, so existing development build commands and
target paths are retained. Compiler/rustdoc improvements apply automatically. The public validation and
correction APIs now include doc aliases (`validate_ir`, `validate_hardware_graph`,
`check_correction`), which also participate in Rust 1.99's improved missing-method
suggestions; these are search/diagnostic aliases, not additional callable methods.
The existing integer constants use associated constants, not deprecated integral
modules. There are no foreign symbols or unsafe casts requiring compatibility fixes.

Edition 2024 and the unsafe-code ban are enabled without adding unsafe environment
mutation. Python and upstream configuration continues to be passed as subprocess
arguments. There is no claim that changing the compiler implements QEC window
semantics or accelerates the FPGA datapath.

## Implemented semantic improvements

1. **Numerically stable weights.** Compute log odds as `ln_1p(-p) - ln(p)` instead
   of forming `(1-p)/p`. Valid subnormal probabilities no longer overflow the
   intermediate ratio. Positive finite scales, even integer quantization and
   signed 32-bit bounds remain checked. Zero weights produce a diagnostic rather
   than silently implying universal backend support.
2. **One Rust validation entry point.** Validate IDs and components even when an
   error has zero probability. Reject misspelled/unknown fields in version-1 DEM,
   error and component records. Probability and finite-coordinate constraints
   are checked before conversion.
3. **Untrusted window annotations.** Validate annotation version and annotation-only
   status; detector bounds, IDs, disjoint local regions, layer/boundary agreement,
   and same-layer commit exclusivity. Recompute seam/crossing references from the
   DEM so direct Rust imports cannot bypass Python's checks or provide stale
   references. Python uses the same membership policy. Coverage and actual graph
   slicing are still outside this annotation contract.
4. **Lossless coordinate sidecar.** Preserve complete detector coordinates and
   detector/observable counts in graph output. Visualization positions remain
   distinct from logical/surgery semantics.
5. **Global correction parity.** Add a Rust API and `check-correction` CLI. Detector
   and observable supports compose by XOR; repeated correction edges cancel,
   input defects must be unique, and out-of-range IDs fail. Return the global
   residual syndrome; CLI exits nonzero if it remains. This is not the layer-0
   commit/seam residual algorithm and does not prove a correction is optimal.
6. **Hardware handoff checks.** In addition to index/weight/boundary identity,
   validate visualization positions, binary-tree node counts and leaf/root roles,
   ordered acyclic parent/child links, growth-vector size/nonnegative entries,
   offloader kinds and referenced edges/vertices. Virtual-match offloaders must
   reference a virtual endpoint of their edge. These checks do not prove the
   numerical growth bound or offloading policy is correct for runtime updates.
7. **CLI failure behavior.** Reject empty legacy output prefixes and turn invalid
   UTF-8 command arguments into a controlled error instead of an `env::args()`
   panic. Existing staging/output preservation behavior is retained.

## Verification and limitation

Installing 1.99.0 with rustup was attempted, but this sandbox blocks
`static.rust-lang.org`. The npm mirror used earlier does not have 1.99 packages.
**A real Rust 1.99 compile, formatter, Clippy and CI run are therefore pending.**
No downgrade was made to the project's pin or MSRV to hide that limitation.

Compatibility checks with the already-installed 1.88 compiler, using Cargo's
`--ignore-rust-version` only in the sandbox, passed:

- 12 Rust unit tests.
- 27 Python/integration tests; one optional full Micro Blossom parser test skipped.
- Formatting and Clippy under the available older toolchain.
- Native Fusion Blossom reader/decoder tests included in that integration suite.

The sandbox-only Cargo wrapper inserted the MSRV override for subprocess build/run
commands during Python tests. It is not part of the repository, normal usage, or
CI. These checks establish useful behavioral regression coverage, **not** Rust
1.99 certification. Both checked-in CI jobs use 1.99 with no MSRV bypass. The full
upstream parser/hardware path still requires the optional upstream job.
