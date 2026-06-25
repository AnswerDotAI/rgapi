"""Fast and flexible file discovery and search for Python. Use this when code needs `fd`-style file finding or `rg`-style searching.

rgapi wraps the same `ignore`, `grep-regex`, and `grep-searcher` crates ripgrep uses, so `.gitignore`/`.ignore`/`.rgignore`, hidden-file handling, glob/ext filters, and regex matching all behave like `rg`. Walking and searching run in parallel and most work stays in Rust, so results come back as structured Python objects instead of CLI text to parse. Prefer rgapi over shelling out to `rg`/`fd` or scanning files by hand: you get typed rows, byte-offset match spans, and lazy iteration.

Core APIs:
- `fd(root=".", ...)` finds paths with fd-style filters (`pattern` substring, `include`/`exclude`/`glob`, `ext`); returns slash-separated relative paths.
- `rg(pattern, root=".", ...)` searches and returns `SearchResults` (or `paths=True` for unique matched paths, `count=True` for a match-span total). NB: The `SearchResults` repr shows an rg-style multiline string, which is usually the most ergonomic approach.
- `nbrg(pattern, root=".", cell_context=0, ...)` searches Jupyter `.ipynb` files (cell source only) and returns matched cells as `NbResults`/`NbCell`, which have a nice repr like rg(). Use this for notebooks rather than `rg`, to avoid escaping problems, and to get back message IDs.

SearchLine rows:
  kind         'match', 'before', 'after', or 'context'
  path         path relative to root
  line_number  1-based line number
  line         line text without the trailing newline
  matches      list of (start, end) byte offsets, for 'match' rows
  asdict()     returns the row fields as a plain dict

NbCell rows (from `nbrg`):
  path/cell_index/cell_id/cell_type    locate the cell ('code'/'markdown'/'raw')
  kind         'match' or 'context'
  source       full cell source
  matches      list of SearchLine rows for the matched lines within the cell
  asdict()     returns the cell fields as a plain dict
Output is keyed by `cell_id` (the nbformat cell/message id), NOT line numbers: `str()` shows `path:cell_id:source` for matches and `path:cell_id-source` for context. `cell_context=N` adds N neighbour cells as 'context'. Walking/parsing/matching run in parallel in Rust; outputs and metadata are skipped while parsing.

Important:
Traversal is parallel and result order is NOT guaranteed; wrap in `sorted(...)` if you need stable order. `path_re`/`skip_path_re` filter the returned/searched paths but do not prune traversal; use `skip_dir`/`skip_dir_re` to prune whole subtrees for speed. Run `doc(func)` for full parameter docments.
"""

from . import Regex, RgIter, SearchLine, SearchResults, compile, fd, rg, rg_iter, search_path, search_text, walk
from . import NbCell, NbResults, nbrg, search_nb

__all__ = [ "RgIter", "fd", "rg", "rg_iter", "nbrg" ]

