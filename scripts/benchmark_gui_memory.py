#!/usr/bin/env python3
"""Sample the real GUI process's RSS for fresh, deterministic library sizes."""

import argparse
import csv
import os
import pathlib
import selectors
import subprocess
import tempfile
import time

MARKER = b"OEUVRE_GUI_READY\n"
FIELDS = ("resources", "settled_rss_kib", "observed_peak_rss_kib", "seconds_to_ready")


def rss_kib(pid):
    """Resident set size of the GUI process, in KiB (macOS/Linux ps)."""
    result = subprocess.run(
        ["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True
    )
    if result.returncode != 0 or not result.stdout.strip():
        raise RuntimeError(f"cannot sample RSS of process {pid}")
    return int(result.stdout.strip())


def sample(binary, count, settle, interval, timeout):
    with tempfile.TemporaryFile() as errors:
        process = subprocess.Popen(
            [str(binary), "--memory", "--demo-resources", str(count),
             "--benchmark-ready"],
            stdout=subprocess.PIPE, stderr=errors,
        )
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        os.set_blocking(process.stdout.fileno(), False)
        started = time.monotonic()
        ready_at = None
        buffer = b""
        peak = 0
        try:
            while True:
                now = time.monotonic()
                if now - started > timeout:
                    raise RuntimeError(f"{count} resources: no completed sample within {timeout}s")
                if process.poll() is not None:
                    errors.seek(0)
                    detail = errors.read().decode(errors="replace")[-2000:]
                    raise RuntimeError(
                        f"{count} resources: GUI exited ({process.returncode}): {detail}"
                    )
                current = rss_kib(process.pid)
                peak = max(peak, current)
                if ready_at is not None and now - ready_at >= settle:
                    return dict(resources=count, settled_rss_kib=current,
                                observed_peak_rss_kib=peak,
                                seconds_to_ready=round(ready_at - started, 3))
                for _key, _events in selector.select(timeout=interval):
                    chunk = os.read(process.stdout.fileno(), 4096)
                    buffer += chunk
                    if MARKER in buffer and ready_at is None:
                        ready_at = time.monotonic()
                    buffer = buffer[-len(MARKER):]
        finally:
            selector.close()
            process.stdout.close()
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/release/ovr-gui"))
    parser.add_argument("--sizes", default="0,1000,10000,100000", help="comma-separated resource counts")
    parser.add_argument("--settle", type=float, default=3.0, help="seconds after window opens")
    parser.add_argument("--interval", type=float, default=0.1, help="RSS sampling interval in seconds")
    parser.add_argument("--timeout", type=float, default=600.0, help="maximum seconds per size")
    parser.add_argument("--output", type=pathlib.Path, default=pathlib.Path("target/gui-memory.csv"))
    args = parser.parse_args()
    try:
        sizes = [int(s.strip()) for s in args.sizes.split(",")]
    except ValueError:
        parser.error("--sizes must be comma-separated nonnegative integers")
    if not sizes or any(n < 0 for n in sizes):
        parser.error("--sizes must contain nonnegative integers")
    if args.interval <= 0 or args.settle < 0 or args.timeout <= 0:
        parser.error("--interval and --timeout must be positive; --settle cannot be negative")
    if not args.binary.is_file():
        parser.error(f"binary not found: {args.binary}; build with cargo build -p oeuvre-gui --release")
    try:
        rows = []
        for count in sizes:
            print(f"sampling {count} resources...", flush=True)
            rows.append(sample(args.binary, count, args.settle, args.interval, args.timeout))
        with args.output.open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=FIELDS)
            writer.writeheader()
            writer.writerows(rows)
        print(f"wrote {args.output}")
    except (OSError, RuntimeError) as exc:
        parser.exit(1, f"benchmark failed: {exc}\n")


if __name__ == "__main__":
    main()
