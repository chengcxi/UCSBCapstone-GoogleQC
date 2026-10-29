"""Reproducible circuit-sampled accuracy benchmark of the existing window CLI.

Run after `cargo build --locked --release` and
`cargo build --locked --release --example window_accuracy`.
The reference uses the exact same quantized, correlation-approximated graph.
"""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

import fusion_blossom as fb
import numpy as np
import stim


def interval(successes, n):
    z = 1.959963984540054
    p = successes / n
    denominator = 1 + z * z / n
    center = (p + z * z / (2 * n)) / denominator
    radius = z * ((p * (1 - p) / n + z * z / (4 * n * n)) ** 0.5) / denominator
    return [center - radius, center + radius]


def run(command, log):
    start = time.perf_counter()
    with log.open('w') as stream:
        result = subprocess.run(list(map(str, command)), stdout=stream, stderr=subprocess.STDOUT, timeout=120)
    if result.returncode:
        raise RuntimeError(f"Command failed ({result.returncode}); see {log}:\n{log.read_text()[-2500:]}")
    return time.perf_counter() - start


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--distance', type=int, default=5)
    parser.add_argument('--rounds', type=int, default=59)
    parser.add_argument('--noise', type=float, default=0.003)
    parser.add_argument('--shots', type=int, default=1000)
    parser.add_argument('--seed', type=int, default=20261027)
    parser.add_argument('--basis', choices=['x', 'z'], default='z')
    parser.add_argument('--commit-slices', type=int)
    parser.add_argument('--buffer-slices', type=int)
    parser.add_argument('--output', type=Path, default=Path('artifacts/window_benchmark'))
    parser.add_argument('--binary', type=Path, default=Path('target/release/microblossom-toolchain'))
    parser.add_argument('--workers', type=int, default=4)
    parser.add_argument('--batch-size', type=int, default=250)
    parser.add_argument('--runner', type=Path, default=Path('target/release/examples/window_accuracy'))
    args = parser.parse_args(argv)
    commit_width = args.commit_slices or args.distance
    buffer_width = args.buffer_slices if args.buffer_slices is not None else args.distance
    if commit_width < 1 or buffer_width < 1:
        parser.error('Commit and buffer widths must be positive')
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    circuit = stim.Circuit.generated(
        f'surface_code:rotated_memory_{args.basis}', distance=args.distance, rounds=args.rounds,
        after_clifford_depolarization=args.noise,
        before_round_data_depolarization=args.noise,
        before_measure_flip_probability=args.noise,
        after_reset_flip_probability=args.noise,
    )
    circuit_path = out / 'memory.stim'
    circuit.to_file(circuit_path)
    coords = circuit.get_detector_coordinates()
    times = sorted({c[2] for c in coords.values()})
    windows = []
    # Alternate d-slice top and bottom commits. Top windows have d-slice
    # buffers on either side, clipped only at experiment endpoints.
    for block, start in enumerate(range(0, len(times), commit_width)):
        stop = min(start + commit_width, len(times))
        layer = block % 2
        commit_times = set(times[start:stop])
        member_times = (set(times[max(0, start - buffer_width):min(len(times), stop + buffer_width)])
                        if layer == 0 else commit_times)
        windows.append({
            'id': f'{"top" if layer == 0 else "bottom"}-{block // 2}',
            'layer': layer,
            'commit': [d for d, c in coords.items() if c[2] in commit_times],
            'buffer': [d for d, c in coords.items() if c[2] in member_times - commit_times],
        })
    plan_path = out / 'windows.json'
    plan_path.write_text(json.dumps({'windows': windows}))
    detections, actual = circuit.compile_detector_sampler(seed=args.seed).sample(
        args.shots, separate_observables=True)
    shot_rows = [{'defects': np.flatnonzero(d).tolist(), 'observables': np.flatnonzero(o).tolist()}
                 for d, o in zip(detections, actual)]
    shots_path = out / 'shots.json'
    shots_path.write_text(json.dumps({'shots': shot_rows}))
    prefix = out / 'decoded'
    binary = args.binary.resolve()
    base = [binary, 'compile', circuit_path, '--windows', plan_path,
            '--output', prefix, '--python', sys.executable, '--allow-correlated']
    print(f'Generated d={args.distance}, rounds={args.rounds}, detectors={circuit.num_detectors}, '
          f'windows={len(windows)}, shots={args.shots}', flush=True)
    compile_seconds = run(base, out / 'compile.log')
    graph = json.loads((out / 'decoded.graph.json').read_text())
    print(f'Compiled {len(graph["initializer"]["weighted_edges"])} edges in {compile_seconds:.2f}s', flush=True)
    start = time.perf_counter()
    def decode_chunk(offset):
        folder = out / f'batch-{offset:06d}'
        folder.mkdir(exist_ok=True)
        batch_path = folder / 'shots.json'
        batch_path.write_text(json.dumps({'shots': shot_rows[offset:offset + args.batch_size]}))
        seconds = run([args.runner.resolve(), out / 'decoded.ir.json', batch_path,
                       folder / 'corrections.json'], folder / 'decode.log')
        rows = json.loads((folder / 'corrections.json').read_text())['shots']
        return offset, rows, seconds
    correction_rows = [None] * args.shots
    chunk_seconds = []
    completed = 0
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = [pool.submit(decode_chunk, offset) for offset in range(0, args.shots, args.batch_size)]
        for future in as_completed(futures):
            offset, rows, seconds = future.result()
            correction_rows[offset:offset + len(rows)] = rows
            chunk_seconds.append(seconds)
            completed += len(rows)
            print(f'Window decode: {completed}/{args.shots} shots, '
                  f'{time.perf_counter() - start:.1f}s wall time', flush=True)
    decode_seconds = time.perf_counter() - start
    if len(correction_rows) != args.shots:
        raise RuntimeError('Wrong number of decoded shots')
    window_predictions = np.zeros_like(actual)
    rejected = np.zeros(args.shots, dtype=bool)
    errors = []
    for index, row in enumerate(correction_rows):
        if 'error' in row:
            rejected[index] = True
            errors.append({'shot': index, 'error': row['error']})
        else:
            correction = row['correction']
            if correction['residual_syndrome']:
                raise RuntimeError('Accepted correction has nonempty residual')
            window_predictions[index, correction['predicted_observables']] = True
    print(f'Window CLI completed in {decode_seconds:.2f}s; running full-graph reference', flush=True)
    reader = fb.ErrorPatternReader(str(out / 'decoded.syndromes'))
    solver = fb.SolverSerial(reader.get_initializer())
    metadata = graph['edge_metadata']
    reference_predictions = np.zeros_like(actual)
    reference_residual_failures = 0
    start = time.perf_counter()
    for index, row in enumerate(shot_rows):
        solver.clear()
        solver.solve(fb.SyndromePattern(defect_vertices=row['defects']))
        residual = set(row['defects'])
        for edge in solver.subgraph():
            residual.symmetric_difference_update(metadata[edge]['detectors'])
            for observable in metadata[edge]['observables']:
                reference_predictions[index, observable] ^= True
        reference_residual_failures += bool(residual)
    reference_seconds = time.perf_counter() - start
    window_wrong = np.any(window_predictions != actual, axis=1) | rejected
    reference_wrong = np.any(reference_predictions != actual, axis=1)
    def metrics(wrong):
        failures = int(wrong.sum())
        return {'logical_failures': failures, 'logical_error_rate': failures / args.shots,
                'accuracy': 1 - failures / args.shots,
                'accuracy_95pct_wilson': interval(args.shots - failures, args.shots)}
    summary = {
        'git_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'runner_sha256': hashlib.sha256(args.runner.resolve().read_bytes()).hexdigest(),
        'working_tree_diff_sha256': hashlib.sha256(subprocess.check_output(['git', 'diff'])).hexdigest(),
        'host_patch': 'std-only printing fallback in blossom-nostd/src/util.rs; decoding logic unchanged',
        'stim_version': stim.__version__, 'python_version': sys.version,
        'reference_fusion_blossom_version': importlib.metadata.version('fusion-blossom'),
        'distance': args.distance, 'rounds': args.rounds, 'noise_probability_each_channel': args.noise,
        'basis': args.basis,
        'shots': args.shots, 'seed': args.seed, 'circuit_qubits': circuit.num_qubits,
        'independent_shot_batch_workers': args.workers, 'shots_per_batch': args.batch_size,
        'detectors': circuit.num_detectors, 'edges': len(metadata), 'time_slices': len(times),
        'top_windows': sum(w['layer'] == 0 for w in windows),
        'bottom_windows': sum(w['layer'] == 1 for w in windows),
        'commit_slices': commit_width, 'buffer_slices_each_side': buffer_width,
        'correlated_components_approximated': graph['correlated_components_approximated'],
        'window': {**metrics(window_wrong), 'rejected_shots': int(rejected.sum()),
                   'logical_failures_among_accepted': int(np.sum(window_wrong & ~rejected)),
                   'nonempty_residuals_among_accepted': 0,
                   'accepted_shots': int((~rejected).sum())},
        'full_graph_fusion_blossom': {**metrics(reference_wrong),
                                    'nonempty_residuals': reference_residual_failures},
        'prediction_disagreements_among_accepted': int(np.sum(np.any(window_predictions != reference_predictions, axis=1) & ~rejected)),
        'window_only_failures': int(np.sum(window_wrong & ~reference_wrong)),
        'reference_only_failures': int(np.sum(reference_wrong & ~window_wrong)),
        'timings_seconds': {'compile': compile_seconds,
                           'window_initialization_decode_verify_serialize': decode_seconds,
                           'reference_solve_and_python_parity': reference_seconds},
        'timing_note': 'Different measured scopes; not a decoder speed comparison or FPGA measurement.',
        'accuracy_definition': 'Fraction of all shots returning a valid correction and all logical observables correct; rejected shots count as unsuccessful.',
    }
    summary['window']['unsuccessful_shots'] = summary['window'].pop('logical_failures')
    summary['window']['unsuccessful_fraction'] = summary['window'].pop('logical_error_rate')
    (out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    (out / 'rejected-shots.json').write_text(json.dumps(errors, indent=2) + '\n')
    (out / 'per-shot.json').write_text(json.dumps([
        {'shot': i, 'actual': row['observables'],
         'window': None if rejected[i] else np.flatnonzero(window_predictions[i]).tolist(),
         'reference': np.flatnonzero(reference_predictions[i]).tolist()}
        for i, row in enumerate(shot_rows)]))
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == '__main__':
    main()
