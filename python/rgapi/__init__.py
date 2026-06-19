from . import _core

Regex = _core.Regex
SearchLine = _core.SearchLine
RgIter = _core.RgIter
compile = _core.compile

class SearchResults(list):
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


def walk(root=".", hidden=False, ignore=True, max_depth=None, min_depth=None, max_filesize=None,
    follow_links=False, same_file_system=False, sort=False, path_re=None, skip_path_re=None, skip_dir=None,
    skip_dir_re=None, files=True, dirs=False):
    return _core.walk(root, hidden, ignore, max_depth, min_depth, max_filesize, follow_links,
        same_file_system, sort, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs)


def fd(root=".", pattern=None, glob=None, include=None, exclude=None, ext=None, hidden=False, ignore=True, max_depth=None,
    min_depth=None, max_filesize=None, follow_links=False, same_file_system=False, sort=False, path_re=None,
    skip_path_re=None, skip_dir=None, skip_dir_re=None, files=True, dirs=False):
    include, exclude = _filters(glob, include, exclude, ext)
    return _core.find(root, pattern, include, exclude, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, sort, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs)


def _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
    follow_links, same_file_system, sort, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
    before_context, after_context, context):
    include, exclude = _filters(glob, include, exclude, ext)
    before_context, after_context = _context(context, before_context, after_context)
    return (pattern, root, include, exclude, hidden, ignore, max_depth, min_depth, max_filesize, follow_links, same_file_system,
        sort, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, case_sensitive, smart_case, before_context, after_context)


def rg(pattern, root=".", glob=None, include=None, exclude=None, ext=None, hidden=False, ignore=True, max_depth=None,
    min_depth=None, max_filesize=None, follow_links=False, same_file_system=False, sort=False, path_re=None,
    skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=True, before_context=0, after_context=0,
    context=0, paths=False, count=False):
    assert not (paths and count), "paths and count are mutually exclusive"
    args = _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, sort, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
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


def rg_iter(pattern, root=".", glob=None, include=None, exclude=None, ext=None, hidden=False, ignore=True, max_depth=None,
    min_depth=None, max_filesize=None, follow_links=False, same_file_system=False, sort=False, path_re=None,
    skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=True, before_context=0,
    after_context=0, context=0):
    args = _rg_args(pattern, root, glob, include, exclude, ext, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, sort, path_re, skip_path_re, skip_dir, skip_dir_re, case_sensitive, smart_case,
        before_context, after_context, context)
    return _core.rg_iter(*args)


def search_text(matcher, text, path="<text>", before_context=0, after_context=0, context=0):
    before_context, after_context = _context(context, before_context, after_context)
    return SearchResults(_core.search_text(matcher, text, path, before_context, after_context))


def search_path(matcher, path, display_path=None, before_context=0, after_context=0, context=0):
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
