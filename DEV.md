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

Run `cargo fmt --check` and `cargo check` for Rust-only edits. Run `chkstyle` after Python edits once tests pass.

## Release

The canonical version lives in `Cargo.toml`. `pyproject.toml` gets the Python package version from Cargo via `dynamic = ["version"]`.

Release flow is: release first, then bump.

1. Run `ship-rs-test`.
2. Confirm the release version in `Cargo.toml` (`[package].version`).
3. Run `ship-rs-release`.
4. After pushing the release tag, run `ship-rs-bump`, commit the `Cargo.toml` version bump, and push to `main` without a tag.

The GitHub workflow builds wheels for Python 3.10-3.13 on Linux and macOS and publishes artifacts to GitHub Releases and PyPI when a `v*` tag is pushed.

## Design notes

Paths in `fd`, `walk`, `rg`, and `rg_iter` results are relative to the requested root and use `/` separators. Traversal uses `ignore::WalkParallel`, so result order is not part of the API contract. Search results are structured rows; collected result lists use rg-style `str()` and notebook display. Path regexes filter returned/searched paths; `skip_dir` and `skip_dir_re` prune traversal through `ignore::WalkBuilder::filter_entry`. Depth, size, symlink, filesystem, hidden, and ignore options are direct `ignore::WalkBuilder` settings. `rg_iter` exposes the same parallel search stream that `rg` collects by default; `paths=True` and `count=True` consume that stream with different reducers. Binary files and invalid UTF-8 are skipped for now.

This package intentionally has no CLI. Python is the interface.
