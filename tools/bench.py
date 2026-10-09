#!/usr/bin/env python
"""Small rg vs rgapi benchmark.

Creates temporary text fixtures, then compares ripgrep CLI startup/search cost with
rgapi's in-process search. Import and fixture creation time are outside the timed
sections.
"""

import argparse, shutil, subprocess, tempfile, timeit
from pathlib import Path

from rgapi import rg as rgapi_rg

PATTERN = "needle_rgapi_bench"
REPEATS = 7
LARGE_FILES = 6
LARGE_FILE_BYTES = 2_000_000
SMALL_FILES = 800
SMALL_FILE_BYTES = 1_500
SMALL_REPEATS_PER_TIMING = 30


def write_text_file(path, target_bytes, match=False):
    line = "alpha beta gamma delta epsilon zeta eta theta iota kappa\n"
    chunk = line * max(1, target_bytes // len(line))
    if match: chunk += f"{PATTERN} only here\n"
    path.write_text(chunk)


def make_large_dir(root):
    d = root/"large"
    d.mkdir()
    for i in range(LARGE_FILES): write_text_file(d/f"large_{i:02}.txt", LARGE_FILE_BYTES, match=i in (1, 4))
    return d


def make_many_small_dir(root):
    d = root/"many-small"
    d.mkdir()
    for i in range(SMALL_FILES): write_text_file(d/f"small_{i:04}.txt", SMALL_FILE_BYTES, match=i in (123, 654))
    return d


def make_tiny_dir(root):
    d = root/"tiny"
    d.mkdir()
    for i in range(8): write_text_file(d/f"tiny_{i}.txt", 400, match=i == 3)
    return d


def rg_cli(root, pattern):
    cmd = ["rg", "--color", "never", "--no-heading", "--line-number", pattern, str(root)]
    return subprocess.run(cmd, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True).stdout


def rgapi(root, pattern): return rgapi_rg(pattern, str(root))


def bench_one(name, func, root, pattern, number=1):
    timer = timeit.Timer(lambda: func(root, pattern))
    times = timer.repeat(repeat=REPEATS, number=number)
    best = min(times) / number
    avg = sum(times) / len(times) / number
    print(f"{name:24} best {best * 1000:8.2f} ms   avg {avg * 1000:8.2f} ms")


def bench(root, pattern):
    large = make_large_dir(root)
    many_small = make_many_small_dir(root)
    tiny = make_tiny_dir(root)

    # Warm imports, dynamic libraries, and disk caches before timing.
    rgapi(large, pattern)
    rg_cli(large, pattern)

    print(f"fixture: {root}")
    print(f"pattern: {pattern}")
    print(f"large files: {LARGE_FILES} x {LARGE_FILE_BYTES:,} bytes")
    print(f"many small files: {SMALL_FILES} x {SMALL_FILE_BYTES:,} bytes")
    print(f"repeats: {REPEATS}\n")

    for label, fixture, number in [("large", large, 1), ("many-small", many_small, 1), ("tiny x30", tiny, SMALL_REPEATS_PER_TIMING)]:
        for name, func in [("rg", rg_cli), ("rgapi", rgapi)]: bench_one(f"{name} {label}", func, fixture, pattern, number)
        print()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pattern", default=PATTERN, help="Regex to search; fixtures contain 'needle_rgapi_bench only here'")
    args = parser.parse_args()
    if shutil.which("rg") is None: raise SystemExit("rg executable not found")
    with tempfile.TemporaryDirectory(prefix="rgapi-bench-") as d: bench(Path(d), args.pattern)


if __name__ == "__main__": main()
