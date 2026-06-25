# rgapi

`rgapi` is a Python API for ripgrep-style walking and search. It is meant for Python code that wants `fd`-style file discovery or `rg`-style searching without shelling out.

It uses the same `ignore`, `grep-regex`, and `grep-searcher` crates that ripgrep uses for walking, regex matching, and file scanning. Walking and searching run in parallel by default. Most expensive work stays in Rust.

## Overview

For common file discovery and search:

```python
from rgapi import fd, rg, rg_iter

fd(".", ext="py", exclude="test_*.py")
for row in rg_iter("TODO", ".", include="*.py", context=2): print(row.asdict())
rg("TODO", ".", ext="py", skip_dir=".venv", paths=True)
```

For cell-aware search of Jupyter notebooks (see [Notebooks](#notebooks)):

```python
from rgapi import nbrg

nbrg("read_csv", ".", cell_context=1)
```

For direct access to the regex, search, and walk pieces:

```python
from rgapi import compile, search_path, search_text, walk

matcher = compile("TODO")
matcher.is_match("TODO")
matcher.finditer("TODO TODO")

walk(".")
search_text(matcher, "alpha\nTODO\nomega\n", path="memory.txt", context=1)
search_path(matcher, "src/lib.rs", display_path="src/lib.rs")
```

## Install

```bash
pip install rgapi
```

## Semantics

`fd` and `walk` return slash-separated paths relative to `root`. They use the `ignore` crate, so `.gitignore`, `.ignore`, and the usual ripgrep filters apply by default. `.rgignore` files are also honored and take precedence over `.gitignore`. Hidden files are skipped unless `hidden=True`. Pass `ignore=False` to disable all ignore filtering (including `.rgignore`). Symlinks are not followed unless `follow_links=True`; `same_file_system=True` avoids crossing filesystem boundaries. Traversal is parallel, and result order is not guaranteed; use `sorted(...)` if order matters.
`root` arguments accept `str` or `pathlib.Path` and expand `~`; `search_path` also accepts path-like file paths. Display labels such as `display_path` are stringified without expansion.

`fd` adds fd-like filtering on top of `walk`: `pattern` is a substring match on the relative path, and `include`/`exclude` use glob syntax. `glob=` is accepted as an alias for `include=`. A basename glob such as `*.py` also matches recursively, so it finds `src/app.py`. Use `ext="py"` or `ext=["py", "rs"]` for extension filters, `min_depth=`/`max_depth=` to bound recursion, and `max_filesize=` to skip files above a byte limit.

`path_re` and `skip_path_re` are regex filters on slash-separated relative paths. They filter returned paths or searched files, but do not control traversal. `skip_dir` uses glob syntax to prune matching directory subtrees, and `skip_dir_re` does the same with regex.

`rg` and `rg_iter` return structured rows rather than raw CLI text. They accept the same `include`, `exclude`, `glob`, `ext`, `path_re`, `skip_path_re`, `skip_dir`, `skip_dir_re`, `min_depth`, `max_depth`, `max_filesize`, `follow_links`, and `same_file_system` filters as `fd`. Each row is a `SearchLine` with:

```text
kind         'match', 'before', 'after', or 'context'
path         path relative to root
line_number  1-based line number
line         line text without the trailing newline
matches      list of (start, end) byte offsets for match rows
```

`rg`, `search_text`, and `search_path` return `SearchResults` by default, a list subclass whose `str()` and notebook pretty display are rg-style multiline text. `rg_iter` yields rows lazily.

`SearchLine` has a structured `repr`, an rg-style `str` (the `line` is truncated to 120 chars with a trailing `…` for display; `repr` and `asdict()` keep the full line), and `SearchLine.asdict()` returns row fields as a plain Python dict. `rg(..., paths=True)` returns unique matched paths, and `rg(..., count=True)` returns the total number of match spans. `paths` and `count` cannot both be set.

`before_context`, `after_context`, and `context` are like `rg -B`, `rg -A`, and `rg -C`. Files containing NUL bytes or invalid UTF-8 are skipped.

Search is case-sensitive by default, matching `rg`. Use `smart_case=True` for `rg --smart-case` behavior, or `case_sensitive=False` to force case-insensitive matching.

## Notebooks

`nbrg` searches Jupyter `.ipynb` files cell-by-cell, so results are *cells* rather than raw JSON lines, and each match is identified by its **cell id** (the nbformat cell/message id) rather than a line number. Searching a notebook with plain `rg` matches the escaped JSON text (including outputs and metadata) and reports meaningless JSON line numbers; `nbrg` instead searches each cell's reconstructed **source** and reports the cell id, which is stable across edits and points at the actual unit you work with.

```python
from rgapi import nbrg

nbrg("read_csv", ".")                  # cells whose source matches, across all notebooks under "."
nbrg("read_csv", ".", cell_context=1)  # also include neighbouring cells as context
```

Notebooks are walked, parsed, and matched together in one parallel Rust pass, using the same regex engine as `rg`, so regex behaviour and the `case_sensitive`/`smart_case` flags match `rg`. Only cell `source` is searched, not outputs or metadata. `nbrg` accepts the same discovery filters as `fd`/`rg` (`include`, `exclude`, `glob`, `hidden`, `max_depth`, `skip_dir`, …).

`nbrg` returns `NbResults`, a list of `NbCell`. Each `NbCell` has:

```text
path         notebook path relative to root
cell_index   0-based position of the cell in the notebook
cell_id      nbformat cell id (falls back to the cell index for notebooks without ids)
cell_type    'code', 'markdown', or 'raw'
kind         'match' or 'context'
source       full cell source
matches      list of SearchLine rows for the matched lines within the cell
```

`NbCell.asdict()` returns those fields as a plain dict (with `matches` as `SearchLine` dicts). `str()`/pretty display is one truncated, newline-escaped line per cell, keyed by `cell_id` rather than a line number: `path:cell_id:source` for matches and `path:cell_id-source` for context cells. A cell with several matches appears once, with every hit collected in `matches`.

`cell_context=N` includes the `N` cells before and after each matching cell as `kind="context"` rows (deduplicated per notebook).

Notebook walking, parsing, and matching all happen in parallel in Rust, in the same pass as the file walk. Parsing uses a lean model that reads only each cell's `id`, `cell_type`, and `source` and skips outputs and metadata, so large embedded outputs (images, plots) are never materialized. `search_nb(pattern, path, ...)` searches a single notebook file the same way.

## Benchmarks

`tools/bench.py` compares the `rg` CLI with in-process `rgapi`. Run it against a release build. One run on this machine, using best time from seven repeats:

| fixture | rg | rgapi |
| --- | ---: | ---: |
| 6 x 2 MB files, 2 matches | 6.54 ms | 1.44 ms |
| 800 x 1.5 KB files, 2 matches | 13.90 ms | 10.94 ms |
| tiny dir, repeated 30x | 5.92 ms | 2.14 ms |

