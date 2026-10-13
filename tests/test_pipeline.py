"""Full Python -> Rust CLI tests, enabled where a Rust toolchain is available."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.environ.get('RUN_RUST_TESTS') == '1', 'set RUN_RUST_TESTS=1 with cargo installed')
class PipelineTests(unittest.TestCase):
    def test_export_and_parser_contract(self):
        with tempfile.TemporaryDirectory() as tmp:
            ir = Path(tmp) / 'ir.json'
            prefix = Path(tmp) / 'graph'
            subprocess.run([sys.executable, 'tools/stim_export.py', 'examples/seam.dem', '--kind', 'dem', '--output', str(ir), '--windows', 'examples/windows.json'], cwd=ROOT, check=True)
            command = ['cargo', 'run', '--locked', '--quiet', '--', str(ir), str(prefix)]
            subprocess.run(command, cwd=ROOT, check=True)
            data = json.loads(prefix.with_suffix('.graph.json').read_text())
            lines = prefix.with_suffix('.syndromes').read_text().splitlines()
            self.assertTrue(lines[0].startswith('Syndrome Pattern v1.0 '))
            self.assertEqual(json.loads(lines[1]), data['initializer'])
            self.assertEqual(json.loads(lines[2]), data['positions'])
            self.assertEqual(data['initializer']['vertex_num'], 4)
            self.assertEqual(data['initializer']['virtual_vertices'], [3])
            self.assertEqual(len(data['edge_metadata']), 3)
            self.assertEqual(data['edge_metadata'][0]['observables'], [0])
            self.assertTrue(data['window_annotations']['annotation_only'])
            self.assertTrue(all(e[2] % 2 == 0 for e in data['initializer']['weighted_edges']))
            before = prefix.with_suffix('.graph.json').read_bytes()
            subprocess.run(command, cwd=ROOT, check=True)
            self.assertEqual(before, prefix.with_suffix('.graph.json').read_bytes())

    def test_unsupported_input_fails_without_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            ir = Path(tmp) / 'ir.json'
            prefix = Path(tmp) / 'bad'
            ir.write_text(json.dumps({'schema_version': 1, 'num_detectors': 3,
                'num_observables': 0, 'detector_coordinates': [[], [], []],
                'errors': [{'probability': 0.1, 'components': [
                    {'detectors': [0,1,2], 'observables': []}]}]}))
            result = subprocess.run(['cargo', 'run', '--locked', '--quiet', '--', str(ir), str(prefix)], cwd=ROOT, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('hyperedge', result.stderr)
            self.assertFalse(prefix.with_suffix('.graph.json').exists())
            self.assertFalse(prefix.with_suffix('.syndromes').exists())
