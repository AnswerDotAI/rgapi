# rgapi

`rgapi` is a Python API for ripgrep-style walking and search. It is meant for Python code that wants `fd`-style file discovery or `rg`-style searching without shelling out.

It uses the same `ignore`, `grep-regex`, and `grep-searcher` crates that ripgrep uses for walking, regex matching, and file scanning. Most expensive work stays in Rust, so common searches over large trees and files should be close to ripgrep/fd speed. Returning Python objects and collected lists adds overhead.

For common file discovery and search:

```python
from rgapi import fd, rg, rg_iter

fd(".", ext="py", exclude="test_*.py", sort=True)

for row in rg_iter("TODO", ".", include="*.py", context=2):
    print(row.asdict())

rg("TODO", ".", ext="py", skip_dir=".venv", paths=True)
```

For direct access to the regex, search, and walk pieces:

```python
from rgapi import compile, search_path, search_text, walk

matcher = compile("todo")
matcher.is_match("TODO")
matcher.finditer("todo TODO")

walk(".")
search_text(matcher, "alpha\nTODO\nomega\n", path="memory.txt", context=1)
search_path(matcher, "src/lib.rs", display_path="src/lib.rs")
```

## Semantics

`fd` and `walk` return slash-separated paths relative to `root`. They use the `ignore` crate, so `.gitignore` and the usual ripgrep filters apply by default. Hidden files are skipped unless `hidden=True`. Pass `ignore=False` to disable ignore filtering. Symlinks are not followed unless `follow_links=True`; `same_file_system=True` avoids crossing filesystem boundaries; `sort=True` sorts traversal.

`fd` adds fd-like filtering on top of `walk`: `pattern` is a substring match on the relative path, and `include`/`exclude` use glob syntax. `glob=` is accepted as an alias for `include=`. A basename glob such as `*.py` also matches recursively, so it finds `src/app.py`. Use `ext="py"` or `ext=["py", "rs"]` for extension filters, `min_depth=`/`max_depth=` to bound recursion, and `max_filesize=` to skip files above a byte limit.

`path_re` and `skip_path_re` are regex filters on slash-separated relative paths. They filter returned paths or searched files, but do not control traversal. `skip_dir` uses glob syntax to prune matching directory subtrees, and `skip_dir_re` does the same with regex.

`rg` and `rg_iter` return structured rows rather than raw CLI text. They accept the same `include`, `exclude`, `glob`, `ext`, `path_re`, `skip_path_re`, `skip_dir`, `skip_dir_re`, `min_depth`, `max_depth`, `max_filesize`, `follow_links`, `same_file_system`, and `sort` filters as `fd`. Each row is a `SearchLine` with:

```text
kind         'match', 'before', 'after', or 'context'
path         path relative to root
line_number  1-based line number
line         line text without the trailing newline
matches      list of (start, end) byte offsets for match rows
```

`rg`, `search_text`, and `search_path` return `SearchResults` by default, a list subclass whose `str()` and notebook pretty display are rg-style multiline text. `rg_iter` yields rows lazily.

`SearchLine` has a structured `repr`, an rg-style `str`, and `SearchLine.asdict()` returns row fields as a plain Python dict. `rg(..., paths=True)` returns the unique matched paths in search order, and `rg(..., count=True)` returns the total number of match spans. `paths` and `count` cannot both be set.

`before_context`, `after_context`, and `context` match the shape of `rg -B`, `rg -A`, and `rg -C`. Files containing NUL bytes or invalid UTF-8 are skipped.

Case handling follows ripgrep's smart-case behavior by default. Use `case_sensitive=True` or `case_sensitive=False` to force it.

## Install from a checkout

```bash
maturin develop
pytest -q
```

There is no CLI. The package is for Python code that wants the ripgrep crates as a library.
