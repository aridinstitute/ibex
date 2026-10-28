"""Contract tests for the process-level GUI memory benchmark."""

import csv
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name("benchmark_gui_memory.py")


class BenchmarkGuiMemoryTests(unittest.TestCase):
    def test_fresh_process_per_size_and_csv_results(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            fake = root / "fake-gui"
            fake.write_text(
                "#!/usr/bin/env python3\n"
                "import pathlib, sys, time\n"
                "size = sys.argv[sys.argv.index('--demo-resources') + 1]\n"
                "with pathlib.Path(__file__).with_suffix('.runs').open('a') as log:\n"
                "    log.write(f'{size} {__import__(\"os\").getpid()}\\n')\n"
                "print('OEUVRE_GUI_READY', flush=True)\n"
                "time.sleep(10)\n"
            )
            fake.chmod(0o755)
            output = root / "results.csv"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--binary", str(fake),
                 "--sizes", "0,10", "--settle", "0.2", "--interval", "0.05",
                 "--output", str(output)],
                capture_output=True, text=True, timeout=15,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            with output.open(newline="") as stream:
                rows = list(csv.DictReader(stream))
            self.assertEqual([row["resources"] for row in rows], ["0", "10"])
            self.assertTrue(all(int(row["settled_rss_kib"]) > 0 for row in rows))
            self.assertTrue(all(int(row["observed_peak_rss_kib"]) >= int(row["settled_rss_kib"]) for row in rows))
            runs = fake.with_suffix(".runs").read_text().splitlines()
            self.assertEqual(len({line.split()[1] for line in runs}), 2)

    def test_exit_before_ready_fails_without_results(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            fake = root / "bad-gui"
            fake.write_text("#!/usr/bin/env python3\nimport sys\nsys.exit(3)\n")
            fake.chmod(0o755)
            output = root / "results.csv"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--binary", str(fake),
                 "--sizes", "1", "--output", str(output)],
                capture_output=True, text=True, timeout=15,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
