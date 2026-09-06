# rgapi

`rgapi` provides `fd`-style file discovery and `rg`-style text search from Python without starting a shell command.

It uses the same `ignore`, `grep-regex`, and `grep-searcher` crates that ripgrep uses for walking, regex matching, and file scanning. Walking and searching run in parallel by default. Most expensive work stays in Rust.

## Overview

For common file discovery and search:

```python
from rgapi import fd, ls, rg, rg_iter

fd(".", ext="py", exclude="test_*.py")
ls("src")
for row in rg_iter("TODO", ".", include="*.py", context=2): print(row.asdict())
rg("TODO", ".", ext="py", skip_dir=".venv", paths=True)
```

For cell-aware search of Jupyter notebooks (see [Notebooks](#notebooks)):

```python
from rgapi import nbrg

nbrg("read_csv", ".", cell_context=1)
```

Walk and search functions have async versions. Streaming versions yield results as the search finds them. See [Async](#async):

```python
from rgapi import fda, rga, rga_iter, nbrga, nbrga_iter

await rga("TODO", ".", ext="py", timeout_ms=200)
async for row in rga_iter("TODO", "."): print(row)
```

Use the lower-level functions to compile a regex, search text or a single file, or walk a directory:

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

## File discovery

`fd` and `walk` return slash-separated paths relative to `root`. Pass `root` as a `str` or `pathlib.Path`. The sync and async APIs expand `~` and accept `.`, `./`, and paths containing `..`.

Discovery uses the `ignore` crate with ripgrep's default filters. It reads `.gitignore`, `.ignore`, and `.rgignore` files. `.rgignore` takes precedence over `.gitignore`. Pass `ignore=False` to disable all ignore-file filtering, including `.rgignore`.

Hidden files are skipped unless `hidden=True`. Symlinks are followed only with `follow_links=True`. Use `same_file_system=True` to avoid crossing filesystem boundaries.

Traversal runs in parallel without guaranteed result order. Use `sorted(...)` when order matters.

`fd` adds filename filters to `walk`. Its `pattern` is a smart-case regex matched against each basename. Lowercase patterns match case-insensitively. A pattern containing uppercase letters is case-sensitive. Use `path_re` to match the slash-separated relative path instead.

`include` and `exclude` use glob syntax. `glob=` is an alias for `include=`. A basename glob such as `*.py` also matches nested paths such as `src/app.py`.

Filter extensions with `ext="py"` or `ext=["py", "rs"]`. Extension and glob filters must both match. For example, `include="src/*", ext="py"` requires `src/*` and `*.py`, like combining `rg -g` with `-t`.

Set `min_depth` and `max_depth` to bound recursion. `max_filesize` skips files above a byte limit.

`ls` follows the shell command's listing conventions. It uses `fd` with `max_depth=1`, includes directories, disables ignore rules, and sorts by name. Set `hidden=True` for `ls -a` behaviour. All `fd` filters remain available.

`fd_iter` yields `FileEntry` paths as the walk finds them. It accepts every `fd` filter. Stopping iteration ends the walk. It does not accept `timeout_ms`.

`path_re` and `skip_path_re` filter slash-separated relative paths using regexes. They select returned paths or searched files without changing traversal. To skip entire subtrees, use `skip_dir` with a glob or `skip_dir_re` with a regex.

## Text search

`rg` and `rg_iter` return structured `SearchLine` rows. They accept the same filters as `fd`: `include`, `exclude`, `glob`, `ext`, `path_re`, `skip_path_re`, `skip_dir`, `skip_dir_re`, `min_depth`, `max_depth`, `max_filesize`, `follow_links`, and `same_file_system`.

Search is case-sensitive by default, matching `rg`. Use `smart_case=True` for `rg --smart-case` behaviour. Use `case_sensitive=False` to force case-insensitive matching.

Each `SearchLine` has these fields:

```text
kind         'match', 'before', 'after', or 'context'
path         path relative to root
line_number  1-based line number
lnhash       exhash-style `lineno|hash|` address for the line
line         line text without the trailing newline
matches      list of (start, end) byte offsets for match rows
```

`rg`, `search_text`, and `search_path` return `SearchResults` by default. This list subclass displays as rg-style multiline text in `str()` and notebook pretty output. `rg_iter` yields rows lazily.

`SearchLine` has a structured `repr` and an rg-style `str`. The string display truncates `line` to 180 characters with a trailing `…`. `repr` and `asdict()` retain the full line. `SearchLine.asdict()` returns the fields as a plain Python dictionary.

Pass `lnhashs=True` to `rg` or `rg_iter` to display hash addresses instead of line numbers. The `line_number` field remains available.

For other result forms, use `rg(..., paths=True)` to return unique matched paths or `rg(..., count=True)` to count match spans. `paths` and `count` cannot both be set.

### Path results

`fd`, `walk`, and `ls` return `PathResults`. So do `rg` and `nbrg` with `paths=True`. This is a list of `FileEntry` rows.

A `FileEntry` is a `str` subclass containing a relative path. Its `stat` property calls `os.lstat` on first access and caches the result. It returns `None` if the path has vanished. `size`, `mtime`, and `is_dir` use the cached stat result. The object also supports ordinary string operations.

`PathResults` displays as an `ls -l`-style listing of at most `rgapi.MAX_REPR` rows. A final `… N more` line reports omitted rows. Only displayed rows need stat calls. `str()` returns one plain path per line. `list(res)` also displays plain paths.

### Limits and timeouts

Set `timeout_ms` on `rg`, `fd`, `walk`, or `ls` to stop at a deadline and return the results collected so far. Their async versions accept it too. Results report why the operation stopped:

- `stop_reason=None` means the result is complete.
- `stop_reason="max_results"` means `max_results` truncated the result.
- `stop_reason="timeout"` means the deadline was reached.

`complete` is true exactly when `stop_reason` is `None`. `count=True` returns a plain integer without a completion flag. It cannot be combined with `timeout_ms`.

### Context lines

`before_context`, `after_context`, and `context` correspond to `rg -B`, `rg -A`, and `rg -C`. Files containing NUL bytes or invalid UTF-8 are skipped.

### Block summaries

`rg(..., summary=True)` returns one row per blank-line-delimited block instead of one row per matching line. Empty and whitespace-only lines delimit blocks. A block containing several matching lines appears once and keeps every matching `SearchLine` in `matches`.

```python
rg("TODO", ".", summary=True, context=1, maxlen=120)
```

The result is `BlockResults`, a list of `SearchBlock` objects. Each block has `path`, `block_index`, `start_line`, `end_line`, `start_lnhash`, `end_lnhash`, `kind`, full `source`, and `matches`.

Matches display as `path:start-end:source`. Context displays as `path:start-end-source`. With `lnhashs=True`, the range uses copyable boundary addresses such as `path:4|a3f2|,6|b1c3|:source`. Newline runs display as `¶`. `maxlen` limits the displayed text without changing `source` or `asdict()`.

In summary mode, `before_context`, `after_context`, and `context` count neighbouring blocks. `max_results` counts matching blocks and retains their context. `summary=True` cannot be combined with `paths` or `count`. It can be combined with `lnhash` for copyable block boundaries.

## Notebooks

`nbrg` searches cell source in Jupyter `.ipynb` files and returns matching cells. Each result identifies the cell by its nbformat cell/message id, which stays stable across edits.

Plain `rg` searches escaped notebook JSON, including outputs and metadata. Its line numbers refer to that JSON file. `nbrg` searches the reconstructed cell source and identifies the cell you would edit.

```python
from rgapi import nbrg

nbrg("read_csv", ".")                  # cells whose source matches, across all notebooks under "."
nbrg("read_csv", ".", cell_context=1)  # also include neighbouring cells as context
```

Notebook discovery, parsing, and matching run together in one parallel Rust pass. Matching uses `rg`'s regex engine with the same `case_sensitive` and `smart_case` behaviour. `nbrg` accepts the discovery filters from `fd` and `rg`, including `include`, `exclude`, `glob`, `hidden`, `max_depth`, and `skip_dir`.

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

`NbCell.asdict()` returns these fields as a plain dictionary. Its `matches` field contains `SearchLine` dictionaries.

`str()` and pretty display show one line per cell. Matches use `path:cell_id:source`. Context uses `path:cell_id-source`. Newline runs display as `¶`.

A matching cell's display starts at its first matched line. Earlier lines are replaced by `…[Ln]`, where `n` is the matched line's one-based number within the cell. A leading `#|` directive is retained, as in `#| export…[L4]needle here`.

`maxlen` limits displayed source and defaults to 120. The full source remains in `source` and `asdict()`. Each cell appears once even when it has multiple matches. All hits remain in `matches`.

`cell_context=N` includes the `N` cells before and after each match as `kind="context"` rows. Context cells are deduplicated within each notebook.

`nbrg_iter` yields `NbCell` rows as notebooks are parsed. For collected results, `nbrg` accepts these limits and result options:

- `max_results` returns at most that many cells after sorting by path and cell index.
- `count=True` returns the number of matching cells.
- `timeout_ms` applies a deadline with the same `stop_reason` values as `rg`.

The parser reads only each cell's `id`, `cell_type`, and `source`. It skips outputs and metadata without loading embedded images or plots. `search_nb(pattern, path, ...)` searches a single notebook file in the same way.

`rgapi-nbrg` exposes notebook search without requiring a Python kernel:

```bash
rgapi-nbrg 'read_csv' .
rgapi-nbrg 'read_csv' . --cell-context 1
rgapi-nbrg 'read_csv' nbs --glob '*.ipynb' --max-results 20
```

Run `rgapi-nbrg --help` for its discovery, matching, and output options.

## Async

`fda`, `rga`, and `nbrga` are async versions of `fd`, `rg`, and `nbrg`. The async generators `fda_iter`, `rga_iter`, and `nbrga_iter` yield rows as the search finds them. Async functions accept the same arguments and return the same types as their synchronous equivalents.

```python
from rgapi import fda, rga, rga_iter

await fda(".", ext="py")
res = await rga("TODO", ".", timeout_ms=200)
if not res.complete: print(f"partial results: {res.stop_reason}")
async for row in rga_iter("TODO", "."): ...
```

Walking and searching use Rust threads, without `asyncio.to_thread` or the event loop's executor. A callback uses `loop.call_soon_threadsafe` to complete the awaited future or supply results to the generator's queue. The event loop remains unblocked, with normal `contextvars` behaviour.

Cancellation stops the Rust workers within about one row. This includes timeouts from `asyncio.wait_for` or `asyncio.timeout`. It also includes task cancellation, such as starlette cancelling a disconnected client's request.

Wrap an async iterator in `contextlib.aclosing` when leaving its loop early. `break` alone delays generator finalization until garbage collection. The context manager provides prompt cleanup:

```python
async with aclosing(rga_iter("TODO", ".")) as it:
    async for row in it:
        if enough(row): break
```

Use streaming results for incremental display, such as sending batches to a browser as they arrive. Collected results with `timeout_ms` return what was found before the deadline. Use `asyncio.wait_for` when a timeout should raise instead.


## Benchmarks

`tools/bench.py` compares the `rg` CLI with in-process `rgapi`. Run it against a release build. One run on this machine, using best time from seven repeats:

| fixture | rg | rgapi |
| --- | ---: | ---: |
| 6 x 2 MB files, 2 matches | 6.54 ms | 1.44 ms |
| 800 x 1.5 KB files, 2 matches | 13.90 ms | 10.94 ms |
| tiny dir, repeated 30x | 5.92 ms | 2.14 ms |
