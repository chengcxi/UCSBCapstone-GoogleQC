#!/usr/bin/env python3
"""Validate Stim input and emit a versioned, flattened DEM IR for the Rust exporter."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import stim


def parity(values):
    result = set()
    for value in values:
        result.symmetric_difference_update({value})
    return sorted(result)


def normalize(dem):
    errors = []
    for instruction in dem.flattened():
        if instruction.type != "error":
            continue
        components = [{"detectors": [], "observables": []}]
        for target in instruction.targets_copy():
            if target.is_separator():
                components.append({"detectors": [], "observables": []})
            elif target.is_relative_detector_id():
                components[-1]["detectors"].append(target.val)
            elif target.is_logical_observable_id():
                components[-1]["observables"].append(target.val)
            else:
                raise ValueError(f"unsupported DEM target: {target}")
        components = [{key: parity(value) for key, value in c.items()} for c in components]
        errors.append({"probability": instruction.args_copy()[0], "components": components})
    coords = dem.get_detector_coordinates()
    return {
        "schema_version": 1,
        "num_detectors": dem.num_detectors,
        "num_observables": dem.num_observables,
        "detector_coordinates": [coords.get(d, []) for d in range(dem.num_detectors)],
        "errors": errors,
    }


def annotate(ir, plan):
    """Validate explicit detector membership. This does not cut or decode windows."""
    if set(plan) != {"windows"} or not isinstance(plan["windows"], list):
        raise ValueError("window plan must contain only a windows list")
    if not plan["windows"]:
        raise ValueError("window plan must contain at least one window")
    seen = set()
    committed = [set(), set()]
    result = []
    n = ir["num_detectors"]
    for window in plan["windows"]:
        if set(window) != {"id", "layer", "commit", "buffer"}:
            raise ValueError("each window needs exactly id, layer, commit, buffer")
        name, layer = window["id"], window["layer"]
        if not isinstance(name, str) or not name.strip() or name in seen:
            raise ValueError("window IDs must be unique nonempty strings")
        seen.add(name)
        if type(layer) is not int or layer not in (0, 1):
            raise ValueError("window layer must be 0 or 1")
        groups = []
        for region in ("commit", "buffer"):
            values = window[region]
            if not isinstance(values, list) or any(type(d) is not int or not 0 <= d < n for d in values):
                raise ValueError("window detector ID out of range or not an integer")
            if len(set(values)) != len(values):
                raise ValueError("duplicate detector within a region")
            groups.append(set(values))
        commit, buffer = groups
        if not commit or commit & buffer:
            raise ValueError("commit must be nonempty and disjoint from buffer")
        if layer == 1 and buffer:
            raise ValueError("layer 1 does not have a buffer in this annotation schema")
        if committed[layer] & commit:
            raise ValueError("same-layer commit regions overlap")
        committed[layer].update(commit)
        members = commit | buffer
        seams, cuts = [], []
        for ei, error in enumerate(ir["errors"]):
            for ci, component in enumerate(error["components"]):
                ds = set(component["detectors"])
                ref = {"error": ei, "component": ci}
                if ds & commit and ds & buffer:
                    seams.append(ref)
                if ds & members and ds - members:
                    cuts.append(ref)
        result.append({**window, "temporal_boundary": "open" if layer == 0 else "closed",
                       "commit_buffer_seams": seams, "outer_crossings": cuts})
    return {"schema_version": 1, "annotation_only": True, "windows": result}


def export(path, kind, windows=None):
    text = path.read_text()
    if kind == "circuit":
        circuit = stim.Circuit(text)
        # No approximate_disjoint_errors or gauge-detector relaxation: fail closed.
        dem = circuit.detector_error_model(decompose_errors=True)
    else:
        dem = stim.DetectorErrorModel(text)
    ir = normalize(dem)
    ir["provenance"] = {"stim_version": stim.__version__, "input_kind": kind,
                        "input_sha256": hashlib.sha256(text.encode()).hexdigest()}
    if windows is not None:
        ir["window_annotations"] = annotate(ir, windows)
    return ir, str(dem)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--kind", choices=("circuit", "dem"), default="circuit")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--dem-output", type=Path)
    parser.add_argument("--windows", type=Path)
    args = parser.parse_args()
    try:
        windows = json.loads(args.windows.read_text()) if args.windows else None
        ir, dem = export(args.input, args.kind, windows)
        serialized = json.dumps(ir, indent=2, allow_nan=False) + "\n"
        if str(args.output) == "-":
            sys.stdout.write(serialized)
        else:
            args.output.write_text(serialized)
        if args.dem_output:
            args.dem_output.write_text(dem + "\n")
    except (ValueError, OSError, TypeError, KeyError) as exc:
        print(f"stim-export: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
