"""Fast and flexible file discovery and search for Python. Use this when code needs `fd`-style file finding or `rg`-style searching.

rgapi wraps the same `ignore`, `grep-regex`, and `grep-searcher` crates ripgrep uses, so `.gitignore`/`.ignore`/`.rgignore`, hidden-file handling, glob/ext filters, and regex matching all behave like `rg`. Walking and searching run in parallel and most work stays in Rust, so results come back as structured Python objects instead of CLI text to parse. Prefer rgapi over shelling out to `rg`/`fd` or scanning files by hand: you get typed rows, byte-offset match spans, and lazy iteration.

Core APIs:
- `fd(root=".", ...)` finds paths with fd-style filters (`pattern` smart-case basename regex, `include`/`exclude`/`glob`, `ext`); returns slash-separated relative paths.
- `rg(pattern, root=".", ...)` returns matching `SearchLine` rows. `summary=True` instead returns blank-line-delimited `SearchBlock` rows, with newlines escaped and `maxlen` source characters shown per block. Context is line-based normally and block-based in summary mode. `paths=True` returns unique paths, `count=True` returns a match-span total, and `lnhash=True` shows exhash addresses. `summary=True` is incompatible with `paths` and `count`, but combines with `lnhash` to show copyable block boundaries.
- `nbrg(pattern, root=".", cell_context=0, maxlen=120, ...)` searches Jupyter `.ipynb` files (cell source only) and returns matched cells as `NbResults`/`NbCell`. Its display is always a one-line cell summary. Use this for notebooks rather than `rg`, to avoid escaped JSON and get stable cell ids.

SearchLine rows:
  kind         'match', 'before', 'after', or 'context'
  path         path relative to root
  line_number  1-based line number
  lnhash       exhash-style `lineno|hash|` address
  line         line text without the trailing newline
  matches      list of (start, end) byte offsets, for 'match' rows
  asdict()     returns the row fields as a plain dict

SearchBlock rows (from `rg(summary=True)`):
  path/block_index/start_line/end_line/start_lnhash/end_lnhash    locate the block
  kind         'match' or 'context'
  source       full block source
  matches      list of matching SearchLine rows within the block
  asdict()     returns the row fields as a plain dict
Output uses `path:start-end:source` for matches and `path:start-end-source` for context. With `lnhash=True`, `start-end` becomes `start_lnhash,end_lnhash` (or one hash for a one-line block). Empty or whitespace-only lines delimit blocks; `context=N` adds N neighbouring blocks. Multiple matches in one block produce one row.

NbCell rows (from `nbrg`):
  path/cell_index/cell_id/cell_type    locate the cell ('code'/'markdown'/'raw')
  kind         'match' or 'context'
  source       full cell source
  matches      list of SearchLine rows for the matched lines within the cell
  asdict()     returns the cell fields as a plain dict
Output is keyed by `cell_id` (the nbformat cell/message id), not line number: `path:cell_id:source` for matches and `path:cell_id-source` for context. Newlines are escaped and `maxlen` limits displayed source without changing `source`. `cell_context=N` adds N neighbouring cells. Walking, parsing, and matching run in parallel in Rust; outputs and metadata are skipped.

Important:
Traversal is parallel and result order is NOT guaranteed; wrap in `sorted(...)` if you need stable order. `path_re`/`skip_path_re` filter the returned/searched paths but do not prune traversal; use `skip_dir`/`skip_dir_re` to prune whole subtrees for speed. Run `doc(func)` for full parameter docments.
"""

from . import RgIter, fd, nbrg, rg, rg_iter

__all__ = [ "RgIter", "fd", "rg", "rg_iter", "nbrg" ]
