import subprocess
import sys
import unittest

class TestParallelWindowPrototype(unittest.TestCase):
    def test_prototype_runs(self):
        result = subprocess.run(
            [sys.executable, "parallel_window_prototype.py"],
            cwd=".",
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, msg=result.stdout + result.stderr)
        self.assertIn("All prototype demos passed", result.stdout)

    def test_oracle_module_imports(self):
        # Verify that the Python prototype's oracle functions are importable
        # and match the Rust oracle's expected behaviour on hand-worked examples.
        import parallel_window_prototype as pwp

        g = pwp.tiny_line_graph()
        windows = pwp.tiny_windows()
        # Same assertions as in the demo
        committed, parity, residual = pwp.commit_tentative(
            {"A0": [0, 1], "B0": []}, windows, g, [0, 2]
        )
        self.assertEqual(committed, [0])
        self.assertEqual(residual, [1, 2])
        self.assertEqual(parity, [])

if __name__ == "__main__":
    unittest.main()
