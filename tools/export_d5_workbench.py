"""Export the recorded d5_1000 experiment for the browser graph comparison.

No latency report is generated. Recompute the full-graph Fusion Blossom
corrections and verify them against the experiment's recorded predictions.
"""
import argparse
import json
from pathlib import Path
import re

import fusion_blossom as fb


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, default=Path('artifacts/window_benchmark_d5_1000'))
    parser.add_argument('--output', type=Path, default=Path('demo/public/fixtures/d5_1000.comparison.json'))
    args = parser.parse_args(argv)
    def load(name):
        return json.loads((args.input / name).read_text())
    graph = load('decoded.graph.json')
    cuts = load('decoded.windows.json')['windows']
    inputs = load('shots.json')['shots']
    predictions = load('per-shot.json')
    errors = {row['shot']: row['error'] for row in load('rejected-shots.json')}
    accepted = {}
    for batch in sorted(args.input.glob('batch-*')):
        offset = int(batch.name.split('-')[1])
        for row in json.loads((batch / 'corrections.json').read_text())['shots']:
            if 'correction' in row:
                accepted[offset + row['shot']] = row['correction']
    reference = fb.SolverSerial(fb.ErrorPatternReader(str(args.input / 'decoded.syndromes')).get_initializer())
    rows = []
    for index, shot in enumerate(inputs):
        reference.clear()
        reference.solve(fb.SyndromePattern(defect_vertices=shot['defects']))
        edges = reference.subgraph()
        residual = set(shot['defects'])
        observables = set()
        for edge in edges:
            residual.symmetric_difference_update(graph['edge_metadata'][edge]['detectors'])
            observables.symmetric_difference_update(graph['edge_metadata'][edge]['observables'])
        if residual or sorted(observables) != predictions[index]['reference']:
            raise RuntimeError(f'Reference capture mismatch at shot {index}')
        correction = accepted.get(index)
        if correction is not None and correction['predicted_observables'] != predictions[index]['window']:
            raise RuntimeError(f'Window capture mismatch at shot {index}')
        error = errors.get(index)
        match = re.search(r'unmatched residual detectors: \{([^}]*)\}', error) if error else None
        unmatched = [int(value) for value in match.group(1).split(',')] if match else []
        rows.append({'index': index, 'defects': shot['defects'], 'actual': shot['observables'],
                     'reference_edges': edges, 'reference_prediction': sorted(observables),
                     'window_edges': correction['committed_edges'] if correction else None,
                     'window_prediction': correction['predicted_observables'] if correction else None,
                     'error': error, 'unmatched': unmatched})
    summary = load('summary.json')
    # These historical wall times have different scopes and aren't the
    # workbench's controlled same-solver CPU benchmark protocol.
    summary.pop('timings_seconds', None)
    summary.pop('timing_note', None)
    result = {'schema_version': 1, 'kind': 'recorded_window_graph_comparison',
              'reference_method': 'Full-graph Fusion Blossom SolverSerial',
              'window_method': 'Micro Blossom ParallelWindowDecoder (sequential CPU)',
              'timing': None, 'summary': summary, 'graph': graph, 'windows': cuts, 'shots': rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, separators=(',', ':')) + '\n')
    print(f'Exported {len(rows)} shots and {len(cuts)} window graphs to {args.output}')


if __name__ == '__main__':
    main()
