# Development

`rgapi` is a PyO3/maturin package. The Rust crate contains the core implementation; `python/rgapi/__init__.py` is the public Python API over the private `rgapi._core` extension module.

## Layout

```text
src/walk.rs       ignore/globset/grep-regex-backed path walking and filtering
src/search.rs     grep-regex/grep-searcher-backed searching
src/block.rs      blank-line-delimited block grouping, matching, and block context
src/python.rs     PyO3 classes and private core functions
python/rgapi/     public Python wrappers over `rgapi._core`
tests/            pytest coverage for the Python API
```

The public Python API lives in `python/rgapi/__init__.py`. The extension module is private as `rgapi._core`; keep crate-like functions there and put Python-facing argument policy in the wrapper when that stays concise. For example, `glob=` and `ext=` are Python wrapper conveniences over the core include glob list.

## Commands

```bash
maturin develop
pytest -q
```

All tests are Python (`tests/`), run against the built extension; there are no `cargo test` unit tests. For a fast local loop use `maturin develop && pytest -q`. Run `cargo fmt --check` and `cargo check` for Rust-only edits. Run `chkstyle` after Python edits once tests pass.

## Release

The canonical version lives in `Cargo.toml`. `pyproject.toml` gets the Python package version from Cargo via `dynamic = ["version"]`.

Release flow is: release first, then bump.

1. Run `maturin develop && pytest -q`.
2. Confirm the release version in `Cargo.toml` (`[package].version`).
3. Run `ship-rs-release`.
4. After pushing the release tag, run `ship-rs-bump`, commit the `Cargo.toml` version bump, and push to `main` without a tag.

The GitHub workflow builds wheels for Python 3.10-3.13 on Linux and macOS and publishes artifacts to GitHub Releases and PyPI when a `v*` tag is pushed.

## Design notes

Paths in `fd`, `walk`, `rg`, and `rg_iter` results are relative to the requested root and use `/` separators. Traversal uses `ignore::WalkParallel`, so result order is not part of the API contract. Search results are structured rows; collected result lists use rg-style `str()` and notebook display. `SearchLine.lnhash` is computed with the same CRC-32-based line-content hash format as exhash (`lineno|hash|`, low 16 bits of CRC-32 over the line's UTF-8 bytes); `lnhashs=True` only changes row display, not `line_number` or matching behavior. Path regexes filter returned/searched paths; `skip_dir` and `skip_dir_re` prune traversal through `ignore::WalkBuilder::filter_entry`. Depth, size, symlink, filesystem, hidden, and ignore options are direct `ignore::WalkBuilder` settings. `rg_iter` exposes the same parallel search stream that `rg` collects by default; `paths=True` and `count=True` consume that stream with different reducers. Binary files and invalid UTF-8 are skipped for now.

Streaming engine: `walk.rs` owns the generic machinery. `StreamIter<T>` is the worker-thread-plus-bounded-channel iterator (`sync_channel(8192)`, so producers block rather than buffer without limit when a consumer lags), and `spawn_walk` owns the shared scaffold: walker config, panic catching, cancel flag, and worker thread. `rg_iter` (`T = SearchLine`), `block_iter` (`T = SearchBlock`), and `nb_iter` (`T = NbCell`) plug entry closures into that engine. Block search reads each file once, searches it once, groups nonblank lines into blocks, maps matching lines to their blocks, and expands context by block index.
Each `SearchBlock` carries numeric boundaries plus hashes for its first and last source lines. Python keeps both and chooses the displayed address without another file read.

Async API: `fda`, `rga`, `rga_iter`, `nbrga`, and `nbrga_iter` wrap the corresponding private core operations. `rga(summary=True)` uses `_core.block_search_async`; ordinary `rga` uses `_core.rg_async`. Each collected core function takes a Python callback, runs on Rust threads through the generic `stream_async` helper, and delivers with one GIL attach at the end. Iterator forms use `stream_iter_async` and attach once per batch. The Python side settles an `asyncio.Future` or feeds an `asyncio.Queue` via `loop.call_soon_threadsafe`; no Python thread blocks and `asyncio.to_thread` is not involved. `AsyncHandle.cancel()` sets the same atomic flag used by the Rust iterators.

Truncation is recorded on collected results: `max_results` sets `stop_reason="max_results"`, and `timeout_ms` on `rg`/`rga`/`nbrg`/`nbrga` sets `stop_reason="timeout"`. `SearchResults`, `BlockResults`, `PathResults`, and `NbResults` share this through `_Results`; `complete` means `stop_reason is None`. In block summary mode, `max_results` counts matching blocks and keeps their block context. `count=True` returns a plain int, so it rejects timeouts and block summary mode.

Path results are `FileEntry` rows: a `str` subclass carrying the walk root, so paths stay plain strings for compatibility while stat info loads lazily (one cached `os.lstat` per entry, read only on attribute access). The wrapping happens at result construction on the Python side; Rust still streams plain strings. `PathResults.__repr__` shows an `ls -l`-style listing capped at `MAX_REPR` rows, so a huge result never stats everything, while `str()` stays one plain path per line. `ls` is `fd` with shell-style defaults (one level, dirs, ignore rules off), re-sorted with `stop_reason` preserved.

This package intentionally has no CLI. Python is the interface.
