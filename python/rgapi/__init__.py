from . import _core

Regex = _core.Regex
SearchLine = _core.SearchLine
RgIter = _core.RgIter
def compile(
    pattern, # Regex pattern to compile
    case_sensitive=None, # True/False forces case; None allows `smart_case`
    smart_case=False # Match `rg --smart-case` behavior
):
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
    root=".", # Directory to walk
    hidden=False, # Include hidden files and directories
    ignore=True, # Respect `.gitignore` and other ignore files
    max_depth=None, # Maximum directory depth to descend
    min_depth=None, # Minimum depth for returned paths
    max_filesize=None, # Skip files larger than this many bytes
    follow_links=False, # Follow symbolic links while walking
    same_file_system=False, # Do not cross filesystem boundaries
    path_re=None, # Regex that returned relative paths must match
    skip_path_re=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re=None, # Directory regex used to prune traversal
    files=True, # Include files in results
    dirs=False # Include directories in results
):
    "Walk a directory and return relative file and/or directory paths."
    return _core.walk(root, hidden, ignore, max_depth, min_depth, max_filesize, follow_links,
        same_file_system, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs)


def fd(
    root=".", # Directory to walk
    pattern=None, # Substring that relative paths must contain
    glob=None, # Include glob or globs; alias for `include`
    include=None, # Include glob or globs, e.g. `*.py`
    exclude=None, # Exclude glob or globs, e.g. `test_*.py`
    ext=None, # Extension or extensions to include, without needing `*.`
    hidden=False, # Include hidden files and directories
    ignore=True, # Respect `.gitignore` and other ignore files
    max_depth=None, # Maximum directory depth to descend
    min_depth=None, # Minimum depth for returned paths
    max_filesize=None, # Skip files larger than this many bytes
    follow_links=False, # Follow symbolic links while walking
    same_file_system=False, # Do not cross filesystem boundaries
    path_re=None, # Regex that returned relative paths must match
    skip_path_re=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re=None, # Directory regex used to prune traversal
    files=True, # Include files in results
    dirs=False # Include directories in results
):
    "Find paths with fd-style filters and gitignore support."
    include, exclude = _filters(glob, include, exclude, ext)
    return _core.find(root, pattern, include, exclude, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs)


def _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
    follow_links, same_file_system, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
    before_context, after_context, context):
    include, exclude = _filters(glob, include, exclude, ext)
    before_context, after_context = _context(context, before_context, after_context)
    return (pattern, root, include, exclude, hidden, ignore, max_depth, min_depth, max_filesize, follow_links, same_file_system,
        path_re, skip_path_re, _listify(skip_dir), skip_dir_re, case_sensitive, smart_case, before_context, after_context)


def rg(
    pattern, # Regex pattern to search for
    root=".", # Directory to search
    glob=None, # Include glob or globs; alias for `include`
    include=None, # Include glob or globs, e.g. `*.py`
    exclude=None, # Exclude glob or globs, e.g. `test_*.py`
    ext=None, # Extension or extensions to include, without needing `*.`
    hidden=False, # Include hidden files and directories
    ignore=True, # Respect `.gitignore` and other ignore files
    max_depth=None, # Maximum directory depth to descend
    min_depth=None, # Minimum depth for returned/searched files
    max_filesize=None, # Skip files larger than this many bytes
    follow_links=False, # Follow symbolic links while walking
    same_file_system=False, # Do not cross filesystem boundaries
    path_re=None, # Regex that searched relative paths must match
    skip_path_re=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re=None, # Directory regex used to prune traversal
    case_sensitive=None, # True/False forces case; None allows `smart_case`
    smart_case=False, # Match `rg --smart-case` behavior
    before_context=0, # Lines of context before each match, like `rg -B`
    after_context=0, # Lines of context after each match, like `rg -A`
    context=0, # Sets both before and after context, like `rg -C`
    paths=False, # Return unique matched paths instead of rows
    count=False # Return total match span count instead of rows
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
    pattern, # Regex pattern to search for
    root=".", # Directory to search
    glob=None, # Include glob or globs; alias for `include`
    include=None, # Include glob or globs, e.g. `*.py`
    exclude=None, # Exclude glob or globs, e.g. `test_*.py`
    ext=None, # Extension or extensions to include, without needing `*.`
    hidden=False, # Include hidden files and directories
    ignore=True, # Respect `.gitignore` and other ignore files
    max_depth=None, # Maximum directory depth to descend
    min_depth=None, # Minimum depth for returned/searched files
    max_filesize=None, # Skip files larger than this many bytes
    follow_links=False, # Follow symbolic links while walking
    same_file_system=False, # Do not cross filesystem boundaries
    path_re=None, # Regex that searched relative paths must match
    skip_path_re=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re=None, # Directory regex used to prune traversal
    case_sensitive=None, # True/False forces case; None allows `smart_case`
    smart_case=False, # Match `rg --smart-case` behavior
    before_context=0, # Lines of context before each match, like `rg -B`
    after_context=0, # Lines of context after each match, like `rg -A`
    context=0 # Sets both before and after context, like `rg -C`
):
    "Search files lazily, yielding `SearchLine` rows."
    args = _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
        before_context, after_context, context)
    return _core.rg_iter(*args)


def search_text(
    matcher, # Compiled `Regex` from `compile()`
    text, # Text to search
    path="<text>", # Path label stored in results
    before_context=0, # Lines of context before each match
    after_context=0, # Lines of context after each match
    context=0 # Sets both before and after context, like `rg -C`
):
    "Search an in-memory string with a compiled matcher."
    before_context, after_context = _context(context, before_context, after_context)
    return SearchResults(_core.search_text(matcher, text, path, before_context, after_context))


def search_path(
    matcher, # Compiled `Regex` from `compile()`
    path, # File path to search
    display_path=None, # Path stored in results; defaults to `path`
    before_context=0, # Lines of context before each match
    after_context=0, # Lines of context after each match
    context=0 # Sets both before and after context, like `rg -C`
):
    "Search one file with a compiled matcher."
    before_context, after_context = _context(context, before_context, after_context)
    return SearchResults(_core.search_path(matcher, path, display_path, before_context, after_context))


__all__ = [
    "Regex",
    "RgIter",
    "SearchLine",
    "SearchResults",
    "compile",
    "fd",
    "rg",
    "rg_iter",
    "search_path",
    "search_text",
    "walk",
]
