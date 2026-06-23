from os import fspath
from pathlib import Path

from . import _core

Regex = _core.Regex
SearchLine = _core.SearchLine
RgIter = _core.RgIter
def compile(
    pattern:str, # Regex pattern to compile
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False # Match `rg --smart-case` behavior
) -> Regex:
    "Compile a regex matcher for `search_text`, `search_path`, and direct matching."
    return _core.compile(pattern, case_sensitive=case_sensitive, smart_case=smart_case)

class SearchResults(list):
    "List of `SearchLine` rows with rg-style text display."
    def __str__(self): return "\n".join(map(str, self))
    def _repr_pretty_(self, p, cycle): p.text("..." if cycle else str(self))


def _listify(value):
    if value is None: return []
    if isinstance(value, str): return [value]
    return list(value)
def _fs_path(path): return str(Path(path).expanduser())
def _display_path(path): return None if path is None else fspath(path)
def _filters(glob=None, include=None, exclude=None, ext=None):
    includes = _listify(include) + _listify(glob)
    for suffix in _listify(ext):
        suffix = str(suffix)
        if suffix.startswith("."): suffix = suffix[1:]
        includes.append(f"*.{suffix}")
    return includes, _listify(exclude)


def _context(context, before_context, after_context):
    if context: return context, context
    return before_context, after_context


def walk(
    root:str|Path=".", # Directory to walk (expands `~`)
    hidden:bool=False, # Include hidden files and directories
    ignore:bool=True, # Respect `.gitignore` and other ignore files
    max_depth:int|None=None, # Maximum directory depth to descend
    min_depth:int|None=None, # Minimum depth for returned paths
    max_filesize:int|None=None, # Skip files larger than this many bytes
    follow_links:bool=False, # Follow symbolic links while walking
    same_file_system:bool=False, # Do not cross filesystem boundaries
    path_re:str|None=None, # Regex that returned relative paths must match
    skip_path_re:str|None=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re:str|None=None, # Directory regex used to prune traversal
    files:bool=True, # Include files in results
    dirs:bool=False # Include directories in results
) -> list[str]:
    "Walk a directory and return relative file and/or directory paths."
    return _core.walk(_fs_path(root), hidden, ignore, max_depth, min_depth, max_filesize, follow_links,
        same_file_system, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs)


def fd(
    root:str|Path=".", # Directory to walk (expands `~`)
    pattern:str|None=None, # Substring that relative paths must contain
    glob=None, # Include glob or globs; alias for `include`
    include=None, # Include glob or globs, e.g. `*.py`
    exclude=None, # Exclude glob or globs, e.g. `test_*.py`
    ext=None, # Extension or extensions to include, without needing `*.`
    hidden:bool=False, # Include hidden files and directories
    ignore:bool=True, # Respect `.gitignore` and other ignore files
    max_depth:int|None=None, # Maximum directory depth to descend
    min_depth:int|None=None, # Minimum depth for returned paths
    max_filesize:int|None=None, # Skip files larger than this many bytes
    follow_links:bool=False, # Follow symbolic links while walking
    same_file_system:bool=False, # Do not cross filesystem boundaries
    path_re:str|None=None, # Regex that returned relative paths must match
    skip_path_re:str|None=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re:str|None=None, # Directory regex used to prune traversal
    files:bool=True, # Include files in results
    dirs:bool=False # Include directories in results
) -> list[str]:
    "Find paths with fd-style filters and gitignore support."
    include, exclude = _filters(glob, include, exclude, ext)
    return _core.find(_fs_path(root), pattern, include, exclude, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs)


def _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
    follow_links, same_file_system, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
    before_context, after_context, context):
    include, exclude = _filters(glob, include, exclude, ext)
    before_context, after_context = _context(context, before_context, after_context)
    return (pattern, _fs_path(root), include, exclude, hidden, ignore, max_depth, min_depth, max_filesize, follow_links, same_file_system,
        path_re, skip_path_re, _listify(skip_dir), skip_dir_re, case_sensitive, smart_case, before_context, after_context)


def rg(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    glob=None, # Include glob or globs; alias for `include`
    include=None, # Include glob or globs, e.g. `*.py`
    exclude=None, # Exclude glob or globs, e.g. `test_*.py`
    ext=None, # Extension or extensions to include, without needing `*.`
    hidden:bool=False, # Include hidden files and directories
    ignore:bool=True, # Respect `.gitignore` and other ignore files
    max_depth:int|None=None, # Maximum directory depth to descend
    min_depth:int|None=None, # Minimum depth for returned/searched files
    max_filesize:int|None=None, # Skip files larger than this many bytes
    follow_links:bool=False, # Follow symbolic links while walking
    same_file_system:bool=False, # Do not cross filesystem boundaries
    path_re:str|None=None, # Regex that searched relative paths must match
    skip_path_re:str|None=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re:str|None=None, # Directory regex used to prune traversal
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    before_context:int=0, # Lines of context before each match, like `rg -B`
    after_context:int=0, # Lines of context after each match, like `rg -A`
    context:int=0, # Sets both before and after context, like `rg -C`
    paths:bool=False, # Return unique matched paths instead of rows
    count:bool=False # Return total match span count instead of rows
):
    "Search files and return `SearchResults`, matched paths, or a count."
    assert not (paths and count), "paths and count are mutually exclusive"
    args = _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
        before_context, after_context, context)
    if paths:
        seen, res = set(), []
        for row in _core.rg_iter(*args):
            if row.kind != "match" or row.path in seen: continue
            seen.add(row.path)
            res.append(row.path)
        return res
    if count: return sum(len(row.matches) for row in _core.rg_iter(*args) if row.kind == "match")
    return SearchResults(_core.rg(*args))


def rg_iter(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    glob=None, # Include glob or globs; alias for `include`
    include=None, # Include glob or globs, e.g. `*.py`
    exclude=None, # Exclude glob or globs, e.g. `test_*.py`
    ext=None, # Extension or extensions to include, without needing `*.`
    hidden:bool=False, # Include hidden files and directories
    ignore:bool=True, # Respect `.gitignore` and other ignore files
    max_depth:int|None=None, # Maximum directory depth to descend
    min_depth:int|None=None, # Minimum depth for returned/searched files
    max_filesize:int|None=None, # Skip files larger than this many bytes
    follow_links:bool=False, # Follow symbolic links while walking
    same_file_system:bool=False, # Do not cross filesystem boundaries
    path_re:str|None=None, # Regex that searched relative paths must match
    skip_path_re:str|None=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re:str|None=None, # Directory regex used to prune traversal
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    before_context:int=0, # Lines of context before each match, like `rg -B`
    after_context:int=0, # Lines of context after each match, like `rg -A`
    context:int=0 # Sets both before and after context, like `rg -C`
) -> RgIter:
    "Search files lazily, yielding `SearchLine` rows."
    args = _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
        before_context, after_context, context)
    return _core.rg_iter(*args)


def search_text(
    matcher:Regex, # Compiled `Regex` from `compile()`
    text:str, # Text to search
    path:str|Path="<text>", # Path label stored in results
    before_context:int=0, # Lines of context before each match
    after_context:int=0, # Lines of context after each match
    context:int=0 # Sets both before and after context, like `rg -C`
) -> SearchResults:
    "Search an in-memory string with a compiled matcher."
    before_context, after_context = _context(context, before_context, after_context)
    return SearchResults(_core.search_text(matcher, text, _display_path(path), before_context, after_context))


def search_path(
    matcher:Regex, # Compiled `Regex` from `compile()`
    path:str|Path, # File path to search (expands `~`)
    display_path:str|Path|None=None, # Path stored in results; defaults to `path`
    before_context:int=0, # Lines of context before each match
    after_context:int=0, # Lines of context after each match
    context:int=0 # Sets both before and after context, like `rg -C`
) -> SearchResults:
    "Search one file with a compiled matcher."
    before_context, after_context = _context(context, before_context, after_context)
    return SearchResults(_core.search_path(matcher, _fs_path(path), _display_path(display_path), before_context, after_context))


__all__ = [ "RgIter", "fd", "rg", "rg_iter" ]
