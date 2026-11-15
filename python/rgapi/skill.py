# NB: this file is designed for an LLM reader, not a human. Add it to your LLM's context and ask it to explain.
"""Find files, search text, and search notebook cell sources from Python with ripgrep semantics and structured results. Use for fd-style discovery, regex searches, and notebook searches returning stable cell IDs rather than escaped JSON.

- prefer over shell parsing or manual file scans
- edits may follow: `lnhashs=True`; orient first: `rg(summary=True)`, then line results where needed    /line results show fragments
- display bare; too much: narrow with parameters, never join, slice or reformat
- regex parameters: fancy-regex, no engine flag; ignore files, hidden files, globs, extensions: ripgrep's rules
- walk, search: parallel: sort when order matters; unreadable file or directory, notebook `nbrg` can't parse: skipped, `RuntimeWarning` naming it
- `path_re`, `skip_path_re`: filter paths, no pruning; `skip_dir`, `skip_dir_re`: prune subtrees
- links, without `follow_links`: returned as entries, root and dangling ones too; linked directories not walked
- `follow_links=True`: linked directories walked, listed as directories; file links at link path; dangling links still entries
- several roots: results, filters relative to their common ancestor; a file under two roots appears once
- `SearchLine` (`rg` row): `path`, `line_number` (1-based), `lnhash`, `line` (no trailing newline), `kind` (match, before, after, context); match rows add `matches`: byte-offset `(start, end)` pairs
- other results: `PathResults` (`fd`, `ls`, `paths=True`), `SearchBlock` (`summary=True`), `NbCell` (`nbrg`): `doc()` each
"""

from . import RgIter, fd, ls, nbrg, rg, rg_iter, rgstr

__all__ = [ "RgIter", "fd", "ls", "rg", "rg_iter", "nbrg", "rgstr" ]

__pyskill_params__ = {'walk_params': ('glob', 'include', 'exclude', 'hidden', 'min_depth', 'max_filesize',
    'follow_links', 'same_file_system', 'path_re', 'skip_path_re', 'skip_dir', 'skip_dir_re')}
