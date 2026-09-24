"""Find files, search text, and search notebook cell sources from Python with ripgrep semantics and structured results. Use for fd-style discovery, regex searches, and notebook searches returning stable cell IDs rather than escaped JSON.

Built on ripgrep's `ignore`, `grep-regex`, and `grep-searcher` crates: ignore files, hidden files, globs, extensions, and regex syntax behave as in ripgrep. Prefer these to shell parsing or manual file scans: `fd`/`ls` for listings, `rg` for files, `nbrg` for notebook cells, `rgstr` for text already in hand.

Orient with `rg(summary=True)`: one row per block (a paragraph of prose/config) with boundary addresses, where line results show fragments that may need another view. Switch to line results where needed; add `lnhashs=True` when edits may follow. Display results bare; narrow oversized results with parameters, not by joining, slicing, or reformatting.

Walking and searching run in parallel: sort when order matters. Shared walk parameters (`fd`, `ls`, `rg`, `nbrg`): `path_re`/`skip_path_re` filter paths without pruning; `skip_dir`/`skip_dir_re` prune subtrees. Without `follow_links`, links (including root and dangling links) are returned as entries (`ls` instead lists a root link to a directory) and linked directories aren't walked; `follow_links=True` walks linked directories (listing them as directories), still returns file links at their link path, and returns dangling links as entries. `root` can be a list of paths. Results and filters then use paths relative to the common ancestor of the roots. A file under two roots appears once.

## Results

`fd`, `ls`, and `paths=True` return `PathResults` of absolute `pathlib.Path`s. `rg` returns `SearchLine` rows (`SearchBlock` with `summary=True`); `nbrg` returns `NbCell` rows. `doc()` on `rg`, `nbrg`, `PathResults`, `SearchBlock`, and `NbCell` gives their rules, fields, and displays. `SearchLine` fields: `path`, 1-based `line_number`, `lnhash`, `line` (no trailing newline), `kind` (match/before/after/context); match rows add `matches`, byte-offset `(start, end)` pairs.
"""

from . import RgIter, fd, ls, nbrg, rg, rg_iter, rgstr

__all__ = [ "RgIter", "fd", "ls", "rg", "rg_iter", "nbrg", "rgstr" ]

__pyskill_params__ = {'walk_params': ('glob', 'include', 'exclude', 'hidden', 'min_depth', 'max_filesize',
    'follow_links', 'same_file_system', 'path_re', 'skip_path_re', 'skip_dir', 'skip_dir_re')}
