#!/usr/bin/env python
"""Small rg vs rgapi benchmark.

Creates temporary text fixtures, then compares ripgrep CLI startup/search cost with
rgapi's in-process search. Import and fixture creation time are outside the timed
sections.
"""

import shutil, subprocess, tempfile, timeit
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


def rg_cli(root):
    cmd = ["rg", "--color", "never", "--no-heading", "--line-number", PATTERN, str(root)]
    return subprocess.run(cmd, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True).stdout


def rgapi(root): return rgapi_rg(PATTERN, str(root))


def bench_one(name, func, root, number=1):
    timer = timeit.Timer(lambda: func(root))
    times = timer.repeat(repeat=REPEATS, number=number)
    best = min(times) / number
    avg = sum(times) / len(times) / number
    print(f"{name:24} best {best * 1000:8.2f} ms   avg {avg * 1000:8.2f} ms")


def bench(root):
    large = make_large_dir(root)
    many_small = make_many_small_dir(root)
    tiny = make_tiny_dir(root)

    # Warm imports, dynamic libraries, and disk caches before timing.
    rgapi(large)
    rg_cli(large)

    print(f"fixture: {root}")
    print(f"large files: {LARGE_FILES} x {LARGE_FILE_BYTES:,} bytes")
    print(f"many small files: {SMALL_FILES} x {SMALL_FILE_BYTES:,} bytes")
    print(f"repeats: {REPEATS}\n")

    bench_one("rg large", rg_cli, large)
    bench_one("rgapi large", rgapi, large)
    print()
    bench_one("rg many-small", rg_cli, many_small)
    bench_one("rgapi many-small", rgapi, many_small)
    print()
    bench_one("rg tiny x30", rg_cli, tiny, number=SMALL_REPEATS_PER_TIMING)
    bench_one("rgapi tiny x30", rgapi, tiny, number=SMALL_REPEATS_PER_TIMING)


def main():
    if shutil.which("rg") is None: raise SystemExit("rg executable not found")
    with tempfile.TemporaryDirectory(prefix="rgapi-bench-") as d: bench(Path(d))


if __name__ == "__main__": main()
