"""Find files, search text, and search notebook cell sources from Python with ripgrep semantics and structured results. Use for fd-style discovery, regex searches, and notebook searches returning stable cell IDs rather than escaped JSON.

Uses ripgrep's `ignore`, `grep-regex`, and `grep-searcher` crates, including ignore files, hidden-file handling, globs, extensions, and regex semantics. Prefer these APIs to shell parsing or manual file scans in Python; use `rgstr` for held text instead of split-line loops, and `ls`/`fd` for kernel-side listings.

For orientation, start with `rg(summary=True)`; use line-level results where needed and `lnhashs=True` when edits may follow. Summary blocks suit prose/config paragraphs as well as code. Display results bare; narrow oversized results with API parameters rather than joining, slicing, or reformatting them.

## Search units

- `rg`: lines, or `SearchBlock` rows with `summary=True`. Blank/whitespace-only lines separate blocks; multiple matches in one block yield one row. Context counts the selected unit. Summary mode cannot combine with `paths` or `count`, but supports hashed block boundaries.
- `nbrg`: `NbResults` of `NbCell` rows from source only, never metadata/outputs. `multiline=True` matches across cell lines while `^`/`$` remain line anchors; ordinary line-oriented `rg` rejects newline patterns.
- Traversal/search run in parallel in Rust; sort when stable order is required. `path_re`/`skip_path_re` filter paths without pruning; `skip_dir`/`skip_dir_re` prune subtrees.

## Result fields and display

`FileEntry` is a slash-separated relative-path `str` with lazy `size`/`mtime`/`is_dir`/`stat`. Path lists render as ls-style tables capped at `MAX_REPR`; `str(res)`/`list(res)` yield plain paths. Unfollowed symlinks remain, marked `l`; `link_target` is their target or `None` for non-links. `ls(hidden=True)` corresponds to `ls -a`.

Search rows provide `asdict()`. All paths are relative to the search root:

| Row | Location | Content/matches | `kind` |
|---|---|---|---|
| `SearchLine` | `path`, 1-based `line_number`, `lnhash` | `line` without trailing newline; `matches` as byte-offset `(start, end)` pairs on match rows | match/before/after/context |
| `SearchBlock` | `path`, `block_index`, `start_line`, `end_line`, `start_lnhash`, `end_lnhash` | full `source`; `matches` as matching `SearchLine`s | match/context |
| `NbCell` | `path`, `cell_index`, `cell_id`, `cell_type` (code/markdown/raw) | full `source`; `matches` as matching `SearchLine`s | match/context |

Block displays use `path:start-end:source`; notebook displays use `path:cell_id:source`. Context replaces the last separator colon with `-`. Hashed block locations are `start_lnhash,end_lnhash`, or one hash for a single line. Newline runs display as ¶, retaining indentation; `maxlen` limits displayed source, not stored `source`.

Notebook match previews start at the first matched line, marking omitted earlier lines as `…[Ln]` (1-based), but retain a leading directive: `#| export…[L4]needle here`. `cell_context` counts neighbouring cells. Consult individual function docs for parameters and reduction modes.
"""

from . import RgIter, fd, ls, nbrg, rg, rg_iter, rgstr

__all__ = [ "RgIter", "fd", "ls", "rg", "rg_iter", "nbrg", "rgstr" ]

__pyskill_params__ = {'walk_params': ('glob', 'include', 'exclude', 'hidden', 'min_depth', 'max_filesize',
    'follow_links', 'same_file_system', 'path_re', 'skip_path_re', 'skip_dir', 'skip_dir_re')}
