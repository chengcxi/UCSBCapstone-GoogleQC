import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import stim
from tools.stim_export import annotate, export, normalize

ROOT = Path(__file__).resolve().parents[1]


class StimExportTests(unittest.TestCase):
    def test_tiny_circuit(self):
        ir, dem = export(ROOT / 'examples/tiny.stim', 'circuit')
        self.assertEqual(ir['num_detectors'], 2)
        self.assertEqual(ir['num_observables'], 1)
        self.assertEqual(ir['detector_coordinates'], [[0, 0, 0], [0, 0, 1]])
        self.assertIn('error', dem)
        self.assertEqual(ir, export(ROOT / 'examples/tiny.stim', 'circuit')[0])
        samples, obs = stim.Circuit.from_file(ROOT / 'examples/tiny.stim').compile_detector_sampler(seed=1).sample(100, separate_observables=True)
        self.assertTrue(((samples[:, 0] ^ samples[:, 1]) == obs[:, 0]).all())

    def test_repeat_shift_coordinates_and_separators(self):
        ir = normalize(stim.DetectorErrorModel('''
            repeat 2 {
                detector(0, 0, 0) D0
                error(0.1) D0 ^ D1 L0
                shift_detectors(0, 0, 1) 1
            }
            detector(0, 0, 0) D0
        '''))
        self.assertEqual(ir['num_detectors'], 3)
        self.assertEqual(ir['detector_coordinates'], [[0,0,0], [0,0,1], [0,0,2]])
        self.assertEqual(ir['errors'][1]['components'][1], {'detectors': [2], 'observables': [0]})

    def test_parity_cancellation(self):
        ir = normalize(stim.DetectorErrorModel('error(0.1) D0 D0 L0 L0'))
        self.assertEqual(ir['errors'][0]['components'], [{'detectors': [], 'observables': []}])

    def test_generated_memory(self):
        circuit = stim.Circuit.generated('surface_code:rotated_memory_z', distance=3, rounds=3, after_clifford_depolarization=0.001)
        ir = normalize(circuit.detector_error_model(decompose_errors=True))
        self.assertEqual(ir['num_detectors'], circuit.num_detectors)
        self.assertGreater(len(ir['errors']), 0)
        self.assertTrue(all(len(c['detectors']) <= 2 for e in ir['errors'] for c in e['components']))

    def test_nondeterministic_detector_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / 'invalid.stim'
            p.write_text('R 0\nH 0\nM 0\nDETECTOR rec[-1]\n')
            with self.assertRaises(ValueError):
                export(p, 'circuit')

    def test_annotations_preserve_physical_boundaries(self):
        ir, _ = export(ROOT / 'examples/seam.dem', 'dem')
        before = copy.deepcopy(ir)
        plan = json.loads((ROOT / 'examples/windows.json').read_text())
        a = annotate(ir, plan)
        self.assertEqual(a['windows'][0]['temporal_boundary'], 'open')
        self.assertEqual(a['windows'][1]['temporal_boundary'], 'closed')
        self.assertEqual(a['windows'][0]['commit_buffer_seams'], [{'error': 0, 'component': 0}])
        self.assertEqual(a['windows'][0]['outer_crossings'], [{'error': 1, 'component': 0}])
        self.assertEqual(ir, before)  # Annotation is NOT graph cutting.

    def test_invalid_plans(self):
        ir, _ = export(ROOT / 'examples/seam.dem', 'dem')
        for updates in ({'layer': 2}, {'layer': True}, {'commit': [3]}, {'commit': [0,0]},
                        {'commit': []}, {'buffer': [0]}, {'layer': 1, 'buffer': [1]}):
            w = {'id': 'a', 'layer': 0, 'commit': [0], 'buffer': [1]}
            w.update(updates)
            with self.assertRaises(ValueError):
                annotate(ir, {'windows': [w]})
        with self.assertRaises(ValueError):
            annotate(ir, {'windows': [w, w]})

    def test_same_layer_commit_overlap_rejected(self):
        ir, _ = export(ROOT / 'examples/seam.dem', 'dem')
        window = {'id': 'a', 'layer': 0, 'commit': [0], 'buffer': [1]}
        with self.assertRaisesRegex(ValueError, 'same-layer'):
            annotate(ir, {'windows': [window, {**window, 'id': 'b'}]})
        with self.assertRaises(ValueError):
            annotate(ir, {'windows': []})

    def test_cli_failure_does_not_write_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'ir.json'
            result = subprocess.run([sys.executable, str(ROOT / 'tools/stim_export.py'), str(Path(tmp) / 'missing'), '--output', str(output)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            self.assertIn('stim-export:', result.stderr)
            self.assertFalse(output.exists())

    def test_cli_dem_and_window_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            output, dem = Path(tmp) / 'ir.json', Path(tmp) / 'out.dem'
            subprocess.run([sys.executable, str(ROOT / 'tools/stim_export.py'), str(ROOT / 'examples/seam.dem'), '--kind', 'dem', '--output', str(output), '--dem-output', str(dem), '--windows', str(ROOT / 'examples/windows.json')], check=True)
            self.assertTrue(json.loads(output.read_text())['window_annotations']['annotation_only'])
            self.assertEqual(stim.DetectorErrorModel.from_file(dem).num_detectors, 3)

    def test_seeded_shots_are_real_stim_samples(self):
        for path, kind, plan in [
            (ROOT / 'examples/tiny.stim', 'circuit', ROOT / 'examples/tiny-windows.json'),
            (ROOT / 'examples/seam.dem', 'dem', ROOT / 'examples/windows.json'),
        ]:
            command = [sys.executable, str(ROOT / 'tools/stim_export.py'), str(path),
                       '--kind', kind, '--windows', str(plan), '--output', '-',
                       '--sample-shots', '8', '--seed', '3']
            one = subprocess.check_output(command, text=True)
            self.assertEqual(one, subprocess.check_output(command, text=True))
            data = json.loads(one)
            self.assertEqual(data['ir']['num_detectors'], 2 if kind == 'circuit' else 3)
            self.assertEqual(len(data['shots']['shots']), 8)
            for shot in data['shots']['shots']:
                self.assertTrue(all(d < data['ir']['num_detectors'] for d in shot['defects']))
        bad = subprocess.run(command[:-2] + ['--seed', '-1'], capture_output=True, text=True)
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn('nonnegative', bad.stderr)


if __name__ == '__main__':
    unittest.main()
