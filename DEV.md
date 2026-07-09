# Development

`rgapi` is a PyO3/maturin package. The Rust crate contains the core implementation; `python/rgapi/__init__.py` is the public Python API over the private `rgapi._core` extension module.

## Layout

```text
src/walk.rs       ignore/globset/grep-regex-backed path walking and filtering
src/search.rs     grep-regex/grep-searcher-backed searching
src/python.rs     PyO3 classes and private core functions
python/rgapi/     public Python wrappers over `rgapi._core`
tests/            pytest coverage for the Python API
```

The public Python API lives in `python/rgapi/__init__.py`. The extension module is private as `rgapi._core`; keep crate-like functions there and put Python-facing argument policy in the wrapper when that stays concise. For example, `glob=` and `ext=` are Python wrapper conveniences over the core include glob list.

## Commands

```bash
ship-rs-test
```

All tests are Python (`tests/`), run against the built extension; there are no `cargo test` unit tests. For a fast local loop use `maturin develop && pytest -q`. Run `cargo fmt --check` and `cargo check` for Rust-only edits. Run `chkstyle` after Python edits once tests pass.

## Release

The canonical version lives in `Cargo.toml`. `pyproject.toml` gets the Python package version from Cargo via `dynamic = ["version"]`.

Release flow is: release first, then bump.

1. Run `ship-rs-test`.
2. Confirm the release version in `Cargo.toml` (`[package].version`).
3. Run `ship-rs-release`.
4. After pushing the release tag, run `ship-rs-bump`, commit the `Cargo.toml` version bump, and push to `main` without a tag.

The GitHub workflow builds wheels for Python 3.10-3.13 on Linux and macOS and publishes artifacts to GitHub Releases and PyPI when a `v*` tag is pushed.

## Design notes

Paths in `fd`, `walk`, `rg`, and `rg_iter` results are relative to the requested root and use `/` separators. Traversal uses `ignore::WalkParallel`, so result order is not part of the API contract. Search results are structured rows; collected result lists use rg-style `str()` and notebook display. `SearchLine.lnhash` is computed with the same no-dependency `DefaultHasher` line-content hash format as exhash (`lineno|hash|`); `lnhash=True` only changes row display, not `line_number` or matching behavior. Path regexes filter returned/searched paths; `skip_dir` and `skip_dir_re` prune traversal through `ignore::WalkBuilder::filter_entry`. Depth, size, symlink, filesystem, hidden, and ignore options are direct `ignore::WalkBuilder` settings. `rg_iter` exposes the same parallel search stream that `rg` collects by default; `paths=True` and `count=True` consume that stream with different reducers. Binary files and invalid UTF-8 are skipped for now.

Streaming engine: `walk.rs` owns the generic machinery. `StreamIter<T>` is the worker-thread-plus-bounded-channel iterator (`sync_channel(8192)`, so producers block rather than buffer without limit when a consumer lags), and `spawn_walk` owns the shared scaffold: walker config, panic catching, cancel flag, worker thread. `rg_iter` (`T = SearchLine`) and `nb_iter` (`T = NbCell`) are entry closures plugged into that engine, and their collect forms drain it with the GIL released. A future file-type searcher gets streaming, cancellation, and async by writing one entry closure.

Async API: `fda`, `rga`, `rga_iter`, `nbrga`, and `nbrga_iter` wrap `_core.find_async`, `_core.rg_async`, `_core.rg_iter_async`, `_core.nb_search_async`, and `_core.nb_iter_async`. Each core function takes a Python callback, runs everything on Rust threads (the generic `stream_async`/`stream_iter_async` helpers in `python.rs`), and delivers with a single GIL attach at the end, or once per batch for the iterators. The Python side settles an `asyncio.Future` (`_acall`) or feeds an `asyncio.Queue` (`_abatches`) via `loop.call_soon_threadsafe`. No Python thread ever blocks, and `asyncio.to_thread` is not involved. `AsyncHandle.cancel()` sets the same atomic flag the iterators use. The wrappers call it in `finally` blocks, so `asyncio.wait_for`, task cancellation, and breaking out of `async for` all stop the Rust workers within about one row. `fd` and `walk` release the GIL for the whole walk. Python-side per-batch flow control for the async iterators was considered and deferred.

Truncation is recorded on the result: `max_results` sets `stop_reason="max_results"`, and `timeout_ms` on `rg`/`rga`/`nbrg`/`nbrga` cancels at the deadline, returns the rows collected so far, and sets `stop_reason="timeout"`. `SearchResults`, `PathResults`, and `NbResults` share this via the `_Results` base, and `complete` means `stop_reason is None`. `fd`, `walk`, and `rg(paths=True)` all return `PathResults`. `count=True` returns a plain int, which cannot carry a truncation flag, so it rejects `timeout_ms`.

This package intentionally has no CLI. Python is the interface.
