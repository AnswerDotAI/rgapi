"""Fast and flexible file discovery and search for Python. Use this when code needs `fd`-style file finding or `rg`-style searching.

rgapi wraps the same `ignore`, `grep-regex`, and `grep-searcher` crates ripgrep uses, so `.gitignore`/`.ignore`/`.rgignore`, hidden-file handling, glob/ext filters, and regex matching all behave like `rg`. Walking and searching run in parallel and most work stays in Rust, so results come back as structured Python objects instead of CLI text to parse. Prefer rgapi over shelling out to `rg`/`fd` or scanning files by hand: you get typed rows, byte-offset match spans, and lazy iteration.

Core APIs:
- `fd(root=".", ...)` finds paths with fd-style filters (`pattern` substring, `include`/`exclude`/`glob`, `ext`); returns slash-separated relative paths.
- `rg(pattern, root=".", ...)` searches and returns `SearchResults` (or `paths=True` for unique matched paths, `count=True` for a match-span total). NB: The `SearchResults` repr shows an rg-style multiline string, which is usually the most ergonomic approach.

SearchLine rows:
  kind         'match', 'before', 'after', or 'context'
  path         path relative to root
  line_number  1-based line number
  line         line text without the trailing newline
  matches      list of (start, end) byte offsets, for 'match' rows
  asdict()     returns the row fields as a plain dict

Important:
Traversal is parallel and result order is NOT guaranteed; wrap in `sorted(...)` if you need stable order. `path_re`/`skip_path_re` filter the returned/searched paths but do not prune traversal; use `skip_dir`/`skip_dir_re` to prune whole subtrees for speed. Run `doc(func)` for full parameter docments.
"""

from . import Regex, RgIter, SearchLine, SearchResults, compile, fd, rg, rg_iter, search_path, search_text, walk

__all__ = ["Regex", "RgIter", "SearchLine", "SearchResults", "compile", "fd", "rg", "rg_iter", "search_path", "search_text", "walk"]

