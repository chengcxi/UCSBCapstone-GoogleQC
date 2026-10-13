"""Real Rust CLI integration; stub handoff tests are NOT upstream hardware tests."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import stim

ROOT = Path(__file__).resolve().parents[1]
ENABLED = os.environ.get('RUN_RUST_TESTS') == '1'


@unittest.skipUnless(ENABLED, 'set RUN_RUST_TESTS=1 with cargo installed')
class RustCliTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cargo', 'build', '--locked', '--quiet'], cwd=ROOT, check=True)
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=ROOT))
        cls.binary = Path(metadata['target_directory']) / 'debug/microblossom-toolchain'

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='microblossom test ')
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name)
        self.prefix = self.path / 'nested/result'

    def run_cli(self, *args):
        # Running outside the checkout exercises the embedded Python bridge.
        return subprocess.run([str(self.binary), *map(str, args)], cwd=self.path,
                              capture_output=True, text=True)

    def compile(self, input=None, *args):
        return self.run_cli('compile', input or ROOT / 'examples/tiny.stim',
                            '--output', self.prefix, '--python', sys.executable, *args)

    def graph(self):
        return json.loads(Path(str(self.prefix) + '.graph.json').read_text())

    def assert_ok(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_compile_and_export_are_equivalent(self):
        self.assert_ok(self.compile())
        first = self.graph()
        result = self.run_cli('export', str(self.prefix) + '.ir.json', '--output', self.path / 'second')
        self.assert_ok(result)
        second = json.loads((self.path / 'second.graph.json').read_text())
        self.assertEqual(first, second)
        self.assertEqual(first['provenance']['input_kind'], 'circuit')

    def test_dem_annotations_and_scale(self):
        self.assert_ok(self.compile(ROOT / 'examples/seam.dem', '--kind', 'dem',
                                    '--windows', ROOT / 'examples/windows.json', '--weight-scale', '10'))
        graph = self.graph()
        self.assertEqual(graph['weight_scale'], 10)
        self.assertTrue(graph['window_annotations']['annotation_only'])
        self.assertEqual(graph['initializer']['weighted_edges'][0], [0, 1, 22])

    def test_invalid_circuit_keeps_previous_outputs(self):
        self.assert_ok(self.compile())
        before = {p: p.read_bytes() for p in self.prefix.parent.iterdir()}
        bad = self.path / 'bad.stim'
        bad.write_text('R 0\nH 0\nM 0\nDETECTOR rec[-1]\n')
        result = self.compile(bad)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Stim bridge failed', result.stderr)
        self.assertEqual(before, {p: p.read_bytes() for p in self.prefix.parent.iterdir()})

    def test_invalid_ir_rejected_before_writing(self):
        bad = self.path / 'bad.json'
        bad.write_text('{')
        result = self.run_cli('export', bad, '--output', self.prefix)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('invalid DEM IR', result.stderr)
        self.assertFalse(self.prefix.parent.exists())

    def test_missing_python_actionable_error(self):
        result = self.run_cli('compile', ROOT / 'examples/tiny.stim', '--output', self.prefix,
                              '--python', self.path / 'no-python')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('could not start Python', result.stderr)
        self.assertFalse(self.prefix.parent.exists())

    def test_argument_validation(self):
        for args in [('--weight-scale', 'nan'), ('--weight-scale', '-1'), ('--kind', 'unknown'),
                     ('--wat', 'x'), ('--weight-scale',), ('--output', 'duplicate')]:
            with self.subTest(args=args):
                result = self.compile(None, *args)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.prefix.parent.exists())
        result = self.run_cli('export', 'unused', '--output', self.prefix, '--windows', 'unused')
        self.assertIn('only valid for compile', result.stderr)

    def test_correlation_requires_explicit_opt_in(self):
        dem = self.path / 'correlated.dem'
        dem.write_text('error(0.1) D0 ^ D1 L0\n')
        rejected = self.compile(dem, '--kind', 'dem')
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn('--allow-correlated', rejected.stderr)
        self.assertFalse(self.prefix.parent.exists())
        accepted = self.compile(dem, '--kind', 'dem', '--allow-correlated')
        self.assert_ok(accepted)
        self.assertIn('warning: correlated', accepted.stderr)
        self.assertTrue(self.graph()['correlated_components_approximated'])

    def stub_parser(self, mode):
        """Protocol-only stand-in. Native decoder tests below use real Rust code."""
        stub = self.path / 'parser stub'
        stub.write_text(f'''#!{sys.executable}
import json, pathlib, sys
assert sys.argv[1] == 'parser'
assert sys.argv[3] == '--graph-file'
mode = {mode!r}
if mode == 'fail':
    print('injected parser failure', file=sys.stderr)
    sys.exit(9)
lines = pathlib.Path(sys.argv[2]).read_text().splitlines()
i = json.loads(lines[1])
def tree(n):
    nodes = [{{}} for _ in range(n)]
    roots = list(range(n))
    while len(roots) > 1:
        l, r = roots.pop(0), roots.pop(0)
        p = len(nodes)
        nodes[l]['p'] = p
        nodes[r]['p'] = p
        nodes.append({{'l': l, 'r': r}})
        roots.append(p)
    return {{'nodes': nodes}}
n, m = i['vertex_num'], len(i['weighted_edges'])
g = dict(i, positions=json.loads(lines[2]),
         vertex_binary_tree=tree(n), edge_binary_tree=tree(m),
         vertex_edge_binary_tree=tree(n+m),
         vertex_max_growth=[sum(e[2] for e in i['weighted_edges'])] * n, offloading=[])
g['weighted_edges'] = [dict(l=l, r=r, w=w) for l, r, w in i['weighted_edges']]
if mode == 'reorder': g['weighted_edges'].reverse()
if mode == 'missing': del g['vertex_binary_tree']
if mode == 'tree_cycle': g['vertex_binary_tree']['nodes'][0]['p'] = 0
if mode == 'short_tree': g['edge_binary_tree']['nodes'].pop()
if mode == 'growth': g['vertex_max_growth'][0] = -1
if mode == 'positions': g['positions'][0]['i'] = 99
if mode == 'offload': g['offloading'] = [{{'dm': {{'e': m}}}}]
if mode == 'virtual': g['offloading'] = [{{'vm': {{'e': 0, 'v': 0}}}}]
pathlib.Path(sys.argv[4]).write_text(json.dumps(g))
''')
        stub.chmod(0o755)
        return stub

    def test_upstream_protocol_handoff_stub(self):
        self.assert_ok(self.compile(None, '--micro-blossom', self.stub_parser('ok')))
        self.assertTrue(Path(str(self.prefix) + '.hardware.json').is_file())
        self.assertFalse(list(self.prefix.parent.glob('.microblossom-stage-*')))

    def test_upstream_failures_keep_previous_outputs(self):
        self.assert_ok(self.compile())
        before = {p: p.read_bytes() for p in self.prefix.parent.iterdir()}
        for mode, diagnostic in [('fail', 'injected parser failure'),
                                 ('reorder', 'sidecar would be invalid'),
                                 ('missing', 'missing vertex_binary_tree'),
                                 ('tree_cycle', 'parent must be a later node'),
                                 ('short_tree', 'expected 2 * leaves - 1'),
                                 ('growth', 'nonnegative integer'),
                                 ('positions', 'positions differ'),
                                 ('offload', 'offloading edge index'),
                                 ('virtual', 'virtual-match offloader')]:
            result = self.compile(None, '--micro-blossom', self.stub_parser(mode))
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(diagnostic, result.stderr)
            self.assertEqual(before, {p: p.read_bytes() for p in self.prefix.parent.iterdir()})

    def test_real_native_reader_and_logical_predictions(self):
        import fusion_blossom as fb  # Pinned in requirements-dev.txt; no mock.
        self.assert_ok(self.compile())
        reader = fb.ErrorPatternReader(str(self.prefix) + '.syndromes')
        initializer = reader.get_initializer()
        graph = self.graph()
        self.assertEqual(initializer.vertex_num, graph['initializer']['vertex_num'])
        self.assertEqual([list(e) for e in initializer.weighted_edges], graph['initializer']['weighted_edges'])
        self.assertEqual(initializer.virtual_vertices, graph['initializer']['virtual_vertices'])
        solver = fb.SolverSerial(initializer)
        samples, obs = stim.Circuit.from_file(ROOT / 'examples/tiny.stim').compile_detector_sampler(seed=7).sample(128, separate_observables=True)
        for detection, expected in zip(samples, obs):
            solver.clear()
            solver.solve(fb.SyndromePattern(defect_vertices=[i for i, bit in enumerate(detection) if bit]))
            predicted = False
            for edge in solver.subgraph():
                predicted ^= 0 in graph['edge_metadata'][edge]['observables']
            self.assertEqual(predicted, bool(expected[0]))

    def test_generated_memory_native_decoding(self):
        import fusion_blossom as fb
        circuit = stim.Circuit.generated('surface_code:rotated_memory_z', distance=3, rounds=3,
                                         after_clifford_depolarization=0.001)
        path = self.path / 'memory.stim'
        circuit.to_file(path)
        self.assert_ok(self.compile(path, '--allow-correlated'))
        graph = self.graph()
        reader = fb.ErrorPatternReader(str(self.prefix) + '.syndromes')
        solver = fb.SolverSerial(reader.get_initializer())
        self.assertEqual(reader.get_initializer().vertex_num, circuit.num_detectors + 1)
        for detection in circuit.compile_detector_sampler(seed=12).sample(64):
            defects = {i for i, bit in enumerate(detection) if bit}
            solver.clear()
            solver.solve(fb.SyndromePattern(defect_vertices=sorted(defects)))
            reconstructed = set()
            for edge in solver.subgraph():
                reconstructed.symmetric_difference_update(graph['edge_metadata'][edge]['detectors'])
            self.assertEqual(reconstructed, defects)

    def test_direct_ir_cannot_forge_window_semantics(self):
        self.assert_ok(self.compile(ROOT / 'examples/seam.dem', '--kind', 'dem',
                                    '--windows', ROOT / 'examples/windows.json'))
        ir = json.loads(Path(str(self.prefix) + '.ir.json').read_text())
        ir['window_annotations']['windows'][0]['temporal_boundary'] = 'closed'
        path = self.path / 'forged.json'
        path.write_text(json.dumps(ir))
        result = self.run_cli('export', path, '--output', self.path / 'forged')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('temporal boundary conflicts with layer', result.stderr)
        self.assertFalse((self.path / 'forged.graph.json').exists())

    def test_check_correction_xor_and_invalid_syndrome(self):
        self.assert_ok(self.compile())
        ir = str(self.prefix) + '.ir.json'
        shot = self.path / 'shot.json'
        for edges, defects, expected_obs, residual in [
            ([0], [0], [0], []),
            ([0, 1], [0, 1], [], []),
            ([0, 0], [], [], []),
            ([0], [1], [0], [0, 1]),
        ]:
            shot.write_text(json.dumps({'edges': edges, 'defects': defects}))
            result = self.run_cli('check-correction', ir, shot)
            self.assertEqual(result.returncode, int(bool(residual)), result.stderr)
            parity = json.loads(result.stdout)
            self.assertEqual(parity['observables'], expected_obs)
            self.assertEqual(parity['residual_syndrome'], residual)
        shot.write_text(json.dumps({'edges': [99], 'defects': []}))
        result = self.run_cli('check-correction', ir, shot)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('edge index 99', result.stderr)

    def test_zero_weight_diagnostic(self):
        dem = self.path / 'zero.dem'
        dem.write_text('error(0.5) D0\n')
        result = self.compile(dem, '--kind', 'dem')
        self.assert_ok(result)
        self.assertIn('zero weight', result.stderr)
        self.assertEqual(self.graph()['initializer']['weighted_edges'][0][2], 0)

    def test_legacy_empty_prefix_is_rejected(self):
        result = self.run_cli(str(self.path / 'unused.json'), '')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('output file prefix', result.stderr)

    @unittest.skipUnless(os.environ.get('MICROBLOSSOM_BINARY'), 'set MICROBLOSSOM_BINARY for full upstream hardware parser')
    def test_real_microblossom_hardware_parser(self):
        self.assert_ok(self.compile(None, '--micro-blossom', os.environ['MICROBLOSSOM_BINARY']))
        self.assertTrue(Path(str(self.prefix) + '.hardware.json').is_file())
