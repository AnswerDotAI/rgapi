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
cargo fmt --check
cargo check
maturin develop
pytest -q
```

Run `cargo test` for Rust unit tests. Run `chkstyle` after Python edits once tests pass.

## Design notes

Paths in `fd`, `walk`, `rg`, and `rg_iter` results are relative to the requested root and use `/` separators. Search results are structured rows; collected result lists use rg-style `str()` and notebook display. Path regexes filter returned/searched paths; `skip_dir` and `skip_dir_re` prune traversal through `ignore::WalkBuilder::filter_entry`. Depth, size, symlink, filesystem, sort, hidden, and ignore options are direct `ignore::WalkBuilder` settings. `rg_iter` streams `ignore` walk entries into `grep-searcher` and buffers one file's matched/context rows at a time. `rg` collects `rg_iter` by default; `paths=True` and `count=True` stream only the selected output. Binary files and invalid UTF-8 are skipped for now.

This package intentionally has no CLI. Python is the interface.
