# Architecture tutorial

These Markdown chapters include Mermaid architecture diagrams (view them on GitHub or in a Mermaid-enabled Markdown renderer):

1. [Stim to Rust decoding-graph toolchain](src/stim-to-rust-architecture.md) — circuit/DEM normalization, Rust conversion and output formats, plus the optional upstream parser.
2. [Parallel-window graph encoding and CPU decoding](src/parallel-window-architecture.md) — window-local graphs, open/closed time cuts, seam events, the two-layer decode and current implementation boundaries.
3. [Developer notes](src/developer-notes.md) — existing implementation notes.

The chapters are also listed in [`src/SUMMARY.md`](src/SUMMARY.md) for mdBook navigation. Mermaid rendering in a local mdBook build depends on having a Mermaid-capable renderer/preprocessor; the diagrams are ordinary `mermaid`-fenced Markdown and render on GitHub.
