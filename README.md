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

`fd` and `walk` return slash-separated paths relative to `root`. They use the `ignore` crate, so `.gitignore` and the usual ripgrep filters apply by default. Hidden files are skipped unless `hidden=True`. Pass `ignore=False` to disable ignore filtering. Symlinks are not followed unless `follow_links=True`; `same_file_system=True` avoids crossing filesystem boundaries. Traversal is parallel, and result order is not guaranteed; use `sorted(...)` if order matters.

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

`SearchLine` has a structured `repr`, an rg-style `str`, and `SearchLine.asdict()` returns row fields as a plain Python dict. `rg(..., paths=True)` returns unique matched paths, and `rg(..., count=True)` returns the total number of match spans. `paths` and `count` cannot both be set.

`before_context`, `after_context`, and `context` match the shape of `rg -B`, `rg -A`, and `rg -C`. Files containing NUL bytes or invalid UTF-8 are skipped.

Search is case-sensitive by default, matching `rg`. Use `smart_case=True` for `rg --smart-case` behavior, or `case_sensitive=False` to force case-insensitive matching.

## Benchmarks

`tools/bench.py` compares the `rg` CLI with in-process `rgapi`. Run it against a release build. One run on this machine, using best time from seven repeats:

| fixture | rg | rgapi |
| --- | ---: | ---: |
| 6 x 2 MB files, 2 matches | 6.54 ms | 1.44 ms |
| 800 x 1.5 KB files, 2 matches | 13.90 ms | 10.94 ms |
| tiny dir, repeated 30x | 5.92 ms | 2.14 ms |

