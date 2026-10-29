#!/usr/bin/env bash
# Reproduce the small, real CLI outputs used by the guided visual replay.
# Run from anywhere after `cargo build --locked` and `pip install -r requirements.txt`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-$ROOT/demo/public/fixtures}"
BIN="${MICROBLOSSOM_TOOLCHAIN_BINARY:-$ROOT/target/debug/microblossom-toolchain}"
PYTHON="${MICROBLOSSOM_PYTHON:-python3}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

"$BIN" compile "$ROOT/examples/tiny.stim" \
  --windows "$ROOT/examples/tiny-windows.json" \
  --sample-shots 24 --seed 7 --python "$PYTHON" --output "$OUT/tiny"
"$BIN" compile "$ROOT/examples/seam.dem" --kind dem \
  --windows "$ROOT/examples/windows.json" \
  --shots "$ROOT/examples/seam-shots.json" \
  --python "$PYTHON" --output "$OUT/seam"

# Fail closed: every example must come from the CLI, contain the expected
# number of actual supplied/sampled shots, and have a globally clear syndrome.
"$PYTHON" - "$OUT" <<'PY'
import json
from pathlib import Path
import sys

out = Path(sys.argv[1])
for name, count in (("tiny", 24), ("seam", 5)):
    for suffix in ("ir.json", "graph.json", "windows.json", "corrections.json", "syndromes"):
        assert (out / f"{name}.{suffix}").is_file(), (name, suffix)
    graph = json.loads((out / f"{name}.graph.json").read_text())
    batch = json.loads((out / f"{name}.corrections.json").read_text())
    windows = json.loads((out / f"{name}.windows.json").read_text())
    assert len(batch["shots"]) == count and len(windows["windows"]) == 2
    for shot in batch["shots"]:
        assert not shot["residual_syndrome"], (name, shot)
        assert shot["logical_error"] is False, (name, shot)
        boundary = set(shot["defects"])
        parity = set()
        for edge_id in shot["committed_edges"]:
            edge = graph["edge_metadata"][edge_id]
            boundary.symmetric_difference_update(edge["detectors"])
            parity.symmetric_difference_update(edge["observables"])
        assert not boundary and sorted(parity) == shot["predicted_observables"], (name, shot)
print("Verified CLI captures: 24 seeded Stim shots and 5 supplied DEM shots")
PY
