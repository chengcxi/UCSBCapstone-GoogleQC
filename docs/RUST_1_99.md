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
   references. Python uses the same membership policy. Annotations alone do not
   imply executable coverage: the CPU decoding adapter additionally checks
   complete time slices and exclusive commit ownership when shots are supplied,
   and consumes the cut window graphs rather than treating annotation as decoding.
4. **Lossless coordinate sidecar.** Preserve complete detector coordinates and
   detector/observable counts in graph output. Visualization positions remain
   distinct from logical/surgery semantics.
5. **Global correction parity.** Add a Rust API and `check-correction` CLI. Detector
   and observable supports compose by XOR; repeated correction edges cancel,
   input defects must be unique, and out-of-range IDs fail. Return the global
   residual syndrome; CLI exits nonzero if it remains. The standalone
   `check-correction` command does not choose a correction; the separate
   `compile`/`export --shots` path runs the two-layer CPU decoder, then uses
   this global algebra to verify its committed edges and observable parity.
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

Rustup downloads from `static.rust-lang.org` are unavailable in this sandbox;
Rust 1.99 binaries and standard libraries from npm packages were sufficient for
local formatting and compiler metadata checks, but full local Cargo builds lack
cached registry packages. The GitHub Actions run
[40394312450](https://github.com/chengcxi/microblossom-parallelism/actions/runs/40394312450)
passed with Rust **1.99.0**: the root job ran Cargo formatting, tests, docs,
Clippy, Python/Stim integration and installed-binary smoke tests; the CPU job
built the nested Micro Blossom executable and tested parallel windows. The
current change set adds further mismatch/ownership checks and reruns the same
jobs in CI. No MSRV bypass or toolchain downgrade is used there.

Earlier compatibility checks under Rust 1.88 with a sandbox-only
`--ignore-rust-version` wrapper also passed, but were not themselves Rust 1.99
certification. The optional upstream parser/hardware job remains unverified;
structural parser checks and real CPU decoding do **not** prove a window-aware
FPGA implementation.
