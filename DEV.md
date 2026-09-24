# Development

`rgapi` is a PyO3/maturin package. The Rust crate contains the core implementation; `python/rgapi/__init__.py` is the public Python API over the private `rgapi._core` extension module.

## Layout

```text
src/walk.rs       ignore/globset/grep-regex-backed path walking and filtering
src/search.rs     grep-regex/grep-searcher-backed searching
src/block.rs      blank-line-delimited block grouping, matching, and block context
src/python.rs     PyO3 classes and private core functions
python/rgapi/     public Python wrappers over `rgapi._core`, plus the `rgapi-nbrg` CLI
tests/            pytest coverage for the Python API
```

The public Python API lives in `python/rgapi/__init__.py`. The extension module is private as `rgapi._core`; keep crate-like functions there and put Python-facing argument policy in the wrapper when that stays concise. For example, `glob=` and `ext=` are Python wrapper conveniences over the core include glob list. Every walking `_core` function takes the walk options as one dict, which PyO3 reads into `WalkOptions`. `_walk(root, **kwargs)` builds that dict. A new walk parameter adds a field to `WalkOptions` and a parameter to `_walk_args`. No `_core` signature changes.

## Commands

```bash
maturin develop
pytest -q
```

Python tests (`tests/`) run against the built extension. `cargo test` covers the Rust-only API, and takes no feature flags: `extension-module` stops pyo3 linking libpython, which a test binary needs. For a fast local loop use `maturin develop && pytest -q`. Run `cargo fmt --check` and `cargo check --all-features` for Rust-only edits. Run `chkstyle` after Python edits once tests pass.

## Release

The canonical version lives in `Cargo.toml`. `pyproject.toml` gets the Python package version from Cargo via `dynamic = ["version"]`.

Release flow is: release first, then bump - `ship-release` does both.

1. Run `maturin develop && pytest -q`.
2. Confirm the release version in `Cargo.toml` (`[package].version`).
3. Run `ship-release`. It tags `v<version>`, pushes branch and tag (CI builds and publishes), then bumps `Cargo.toml`, refreshes the editable install, and pushes the bump without a tag.

The GitHub workflow builds wheels for Python 3.10-3.13 on Linux and macOS and publishes the Rust crate, GitHub release artifacts, and PyPI package when a `v*` tag is pushed.

## Design notes

Python discovery and `paths=True` results contain absolute `pathlib.Path` objects. Structured search rows hold base-relative string labels with `/` separators. Traversal uses `ignore::WalkParallel`, so result order is not part of the API contract. Search results are structured rows; collected result lists use rg-style `str()` and notebook display. `SearchLine.lnhash` is computed with the same CRC-32-based line-content hash format as exhash (`lineno|hash|`, low 12 bits of CRC-32 over the line's UTF-8 bytes, encoded as two Base64url characters); `lnhashs=True` only changes row display, not `line_number` or matching behavior. Path regexes filter returned/searched paths; `skip_dir` and `skip_dir_re` prune traversal through `ignore::WalkBuilder::filter_entry`. Depth, size, filesystem, hidden, and ignore options use `ignore::WalkBuilder` settings. The `ignore` walker follows every root it is given. Discovery checks each root with `symlink_metadata`. The worker sends each unfollowed link root as a result before walking the other roots. This also supports dangling roots, which the underlying walker would reject. `ls` sets `walk_root_links`, which gives a root link to a directory to the walker. The walker then reports paths under the link. Other discovery roots use absolute paths without canonicalizing. Content searches retain canonical root resolution. `rg_iter` exposes the same parallel search stream that `rg` collects by default; `paths=True` and `count=True` consume that stream with different reducers. Text search skips binary files and invalid UTF-8 content.

Streaming engine: `walk.rs` owns the generic machinery. `StreamIter<T>` is the worker-thread-plus-bounded-channel iterator (`sync_channel(8192)`, so producers block rather than buffer without limit when a consumer lags), and `spawn_walk` owns the shared scaffold: walker config, panic catching, cancel flag, and worker thread. `rg_iter` (`T = SearchLine`), `block_iter` (`T = SearchBlock`), `nb_iter` (`T = NbCell`), and `find_iter` (`T = PathBuf`, the path walk) plug entry closures into that engine. Block search reads each file once, searches it once, groups nonblank lines into blocks, maps matching lines to their blocks, and expands context by block index. Each `SearchBlock` carries numeric boundaries plus hashes for its first and last source lines. Python keeps both and chooses the displayed address without another file read.

`resolve_roots` makes the roots absolute, or canonical for content searches, and drops repeats. It also computes the base that result paths and filters are relative to. A directory root is its own base. A file root, or a link root that is not followed, has its parent as its base. Several roots use the common ancestor of their bases. `spawn_walk` walks the roots one after another on its worker thread, with one `ignore` walker per root. Each walker applies the file-root rules to its own root. Entry closures receive the base, not the root. When one root is inside another, a shared set of visited paths lets each path reach the entry closure once. Python gets the same base from `_core.walk_base`.

Async API: `fda`, `fda_iter`, `rga`, `rga_iter`, `nbrga`, and `nbrga_iter` wrap the corresponding private core operations. `rga(summary=True)` uses `_core.block_search_async`; ordinary `rga` uses `_core.rg_async`. Each collected core function takes a Python callback, runs on Rust threads through the generic `stream_async` helper, and delivers with one GIL attach at the end. Iterator forms use `stream_iter_async` and attach once per batch. The Python side settles an `asyncio.Future` or feeds an `asyncio.Queue` via `loop.call_soon_threadsafe`; no Python thread blocks and `asyncio.to_thread` is not involved. `AsyncHandle.cancel()` sets the same atomic flag used by the Rust iterators.

Truncation is recorded on collected results: `max_results` sets `stop_reason="max_results"`, and `timeout_ms` on `rg`/`rga`/`nbrg`/`nbrga`/`fd`/`fda`/`walk`/`ls` sets `stop_reason="timeout"`. `SearchResults`, `BlockResults`, `PathResults`, and `NbResults` share this through `_Results`; `complete` means `stop_reason is None`. In block summary mode, `max_results` counts matching blocks and keeps their block context. `count=True` returns a plain int, so it rejects timeouts and block summary mode.

Rust discovery returns base-relative `PathBuf` values. Glob filters operate on native paths; only regex matching uses string labels. PyO3 converts discovery roots and results with its filesystem-path support. Python joins each result to the base from `_core.walk_base` without resolving links. `PathResults` stores that display base and completion status, including across slices. Its `__repr__` uses `Path.lstat()` for an `ls -l`-style listing capped at `MAX_REPR` rows. `str()` returns one base-relative name per line. `ls` is `fd` with shell-style defaults (one level, dirs, ignore rules off) and `walk_root_links`, sorted in place.

Rust callers can consume a `StreamIter` with `cancel_and_join()` to cancel, drain queued results, and wait for the walk's workers to finish. `Drop` remains nonblocking. Since filesystem calls already in progress must return before joining completes, keep `cancel_and_join()` off async executors.

Notebook hierarchy uses `heading_level(source)`, `section_range(levels, idx)` and `ancestor_indices(levels, idx)`. Callers supply zero for non-heading cells. Heading detection skips blank lines and lines starting with `#|`, then checks the first remaining line against `^#{1,6} \w`. It does not search past ordinary text. Section ranges include the addressed cell and end before the next equal-or-higher heading; non-headings select themselves. Ancestors exclude the addressed cell and are returned outermost first. These are calculations over the supplied levels, without retained outline state. Rustygate uses them for its cell selectors.

`cell_refs(cell)` takes an nbformat cell as a `serde_json::Value` and returns its sigil references as `CellRefs { vars, cmds, tools }`, in order of appearance, with duplicates. A prompt cell has `solveit_ai: true` in its metadata. `vars` holds each `expr` written as `` $`expr` `` in a prompt cell's source. `cmds` holds each `cmd` written as `` !`cmd` `` in a prompt cell's source. `tools` holds each name written as `` &`name` `` or `` &`[a, b]` ``. A name holds word characters and dots. `cell_refs` reads `tools` from the source of a prompt or Markdown cell. For every other cell it reads the `text/markdown` data of `display_data` and `execute_result` outputs. It never reads a prompt cell's outputs. This function is Rust-only. Rustygate uses it for the cells API's `refs=true`.
