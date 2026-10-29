"""Local workbench generation driver; uses the existing CPU decoder unchanged."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

import benchmark_window
import export_d5_workbench


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--runner', type=Path, required=True)
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    settings = ['--distance', str(config['distance']), '--rounds', str(config['rounds']),
                '--shots', str(config['shots']), '--noise', str(config['noise']),
                '--seed', str(config['seed']), '--basis', config['basis'],
                '--commit-slices', str(config['commit']), '--buffer-slices', str(config['buffer']),
                '--workers', '1', '--batch-size', str(config['shots']),
                '--binary', str(args.binary), '--runner', str(args.runner), '--output', str(args.output)]
    benchmark_window.main(settings)
    output = args.output / 'comparison.json'
    export_d5_workbench.main(['--input', str(args.output), '--output', str(output)])
    capture = json.loads(output.read_text())
    capture['source_circuit'] = (args.output / 'memory.stim').read_text()
    capture['generation_settings'] = config
    capture['cpu_benchmark'] = None
    capture['cpu_benchmark_error'] = None
    if config.get('measure'):
        if any(s['error'] for s in capture['shots']):
            capture['cpu_benchmark_error'] = 'Timing comparison skipped: the window decoder rejected shots. Accuracy capture includes every rejected sample.'
        else:
            report = args.output / 'cpu.benchmark.json'
            try:
                subprocess.run([str(args.binary), 'benchmark', str(args.output / 'decoded.ir.json'),
                                '--shots', str(args.output / 'shots.json'), '--allow-correlated',
                                '--warmup', str(config['warmup']), '--repeats', str(config['repeats']),
                                '--output', str(report)], check=True, capture_output=True, text=True, timeout=120)
                capture['cpu_benchmark'] = json.loads(report.read_text())
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
                capture['cpu_benchmark_error'] = str(getattr(error, 'stderr', None) or error)[-1500:]
    # The capture is the portable import/export contract, including its inputs.
    output.write_text(json.dumps(capture, separators=(',', ':')) + '\n')
    print(f'Workbench capture ready: {output}')


if __name__ == '__main__':
    main()
