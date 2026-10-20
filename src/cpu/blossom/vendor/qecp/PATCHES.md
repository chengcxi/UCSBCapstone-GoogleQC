# qecp 0.2.7 (MIT) — stable Rust compatibility patch

This is the upstream QEC-Playground `qecp` 0.2.7 source from
[yuewuo/QEC-Playground](https://github.com/yuewuo/QEC-Playground/tree/f88ca391857db6a602758c404623cefea2e7ab68)
(commit `f88ca391857db6a602758c404623cefea2e7ab68`, published 2024-09-20),
with non-build artifacts removed. See `LICENSE` for its MIT license.

MicroBlossom uses `qecp::decoder_fusion`, so turning off qecp's default
`fusion_blossom` feature breaks the CPU decoder. The published qecp dependency
enables `fusion-blossom/dangerous_pointer` whenever that feature is enabled,
which in turn requires unstable `get_mut_unchecked`. We apply only two edits:

* `Cargo.toml`: do not enable `dangerous_pointer` in qecp's optional
  `fusion-blossom` dependency, and disable its default features. The normal
  safe pointer backend provides the same decoder API.
* `src/decoder_fusion.rs`: import the pointer traits for whichever backend is
  selected (`RwLockPtr` for the safe default, `UnsafePtr` if explicitly enabled)
  rather than unconditionally importing the unsafe-only trait.

Trailing whitespace in `src/distributed_uf_decoder.rs` was also removed;
no behavior changed there.

The parent crate uses `[patch.crates-io]` so both qecp and MicroBlossom share
the same `fusion-blossom` 0.2.12 types. The vendored copy is only for CPU
builds; the FPGA design is unchanged.
