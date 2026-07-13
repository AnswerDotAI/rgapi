import asyncio
from contextlib import aclosing

from os import fspath
from pathlib import Path
from fastcore.meta import delegates

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

class _Results(list):
    "List with `stop_reason`/`complete` truncation tracking and text display."
    stop_reason = None # `None` when complete; `"max_results"` or `"timeout"` when truncated
    @property
    def complete(self): return self.stop_reason is None
    def _repr_pretty_(self, p, cycle): p.text("..." if cycle else str(self))

class SearchResults(_Results):
    "List of `SearchLine` rows with rg-style text display."
    def __str__(self): return "\n".join(map(str, self))

class PathResults(_Results):
    "List of relative paths with line-per-path display."
    def __str__(self): return "\n".join(self)

def _preview(text, maxlen=120):
    text = text.rstrip("\n").replace("\n", "\\n")
    return text if len(text) <= maxlen else text[:maxlen] + "…"


def _listify(value):
    if value is None: return []
    if isinstance(value, str): return [value]
    return list(value)
def _fs_path(path): return str(Path(path).expanduser())
def _display_path(path): return None if path is None else fspath(path)
def _filters(glob=None, include=None, exclude=None, ext=None):
    exts = [f"*.{str(s).lstrip('.')}" for s in _listify(ext)]
    return _listify(include) + _listify(glob), _listify(exclude), exts


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
) -> PathResults:
    "Walk a directory and return relative file and/or directory paths."
    return PathResults(_core.walk(_fs_path(root), hidden, ignore, max_depth, min_depth, max_filesize, follow_links,
        same_file_system, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs))


def _walk_args(
    glob=None, # Include glob or globs; alias for `include`
    include=None, # Include glob or globs, e.g. `*.py`
    exclude=None, # Exclude glob or globs, e.g. `test_*.py`
    ext=None, # Extension or extensions to require, without needing `*.`; ANDs with `include`/`glob`
    hidden:bool=False, # Include hidden files and directories
    ignore:bool=True, # Respect `.gitignore` and other ignore files
    max_depth:int|None=None, # Maximum directory depth to descend
    min_depth:int|None=None, # Minimum depth for returned/searched paths
    max_filesize:int|None=None, # Skip files larger than this many bytes
    follow_links:bool=False, # Follow symbolic links while walking
    same_file_system:bool=False, # Do not cross filesystem boundaries
    path_re:str|None=None, # Regex that relative paths must match
    skip_path_re:str|None=None, # Regex for relative paths to skip
    skip_dir=None, # Directory glob or globs to prune
    skip_dir_re:str|None=None, # Directory regex used to prune traversal
):
    "Walk/filter positional tail for `_core` calls; delegators pass their `**kwargs` here whole"
    include, exclude, exts = _filters(glob, include, exclude, ext)
    return (include, exclude, exts, hidden, ignore, max_depth, min_depth, max_filesize,
        follow_links, same_file_system, path_re, skip_path_re, _listify(skip_dir), skip_dir_re)

@delegates(_walk_args)
def fd(
    root:str|Path=".", # Directory to walk (expands `~`)
    pattern:str|None=None, # Smart-case regex matched against each basename
    files:bool=True, # Include files in results
    dirs:bool=False, # Include directories in results
    **kwargs
) -> PathResults:
    "Find paths with fd-style filters and gitignore support."
    return PathResults(_core.find(_fs_path(root), pattern, *_walk_args(**kwargs), files, dirs))


async def _acall(fn, *args):
    "Run a `_core` async op: settle a Future from its callback; cancel the op if abandoned"
    loop = asyncio.get_running_loop()
    fut = loop.create_future()
    def cb(res, err):
        def settle():
            if fut.done(): return
            if err is None: fut.set_result(res)
            else: fut.set_exception(ValueError(err))
        try: loop.call_soon_threadsafe(settle)
        except RuntimeError: pass
    h = fn(cb, *args)
    try: return await fut
    finally:
        if not fut.done() or fut.cancelled(): h.cancel()


@delegates(fd)
async def fda(
    root:str|Path=".", # Directory to walk (expands `~`)
    pattern:str|None=None, # Smart-case regex matched against each basename
    files:bool=True, # Include files in results
    dirs:bool=False, # Include directories in results
    **kwargs
) -> PathResults:
    "Async `fd`: find paths on Rust threads without blocking the event loop."
    return PathResults(await _acall(_core.find_async, _fs_path(root), pattern, *_walk_args(**kwargs), files, dirs))



def _cap_rows(rows, n):
    "First `n` match rows from `rows` (with their context rows), and whether more matches existed"
    if n is None: return rows, False
    res,seen,pending = [],0,[]
    for row in rows:
        if row.kind == "match":
            seen += 1
            if seen > n: return res, True
            res += pending
            pending = []
            res.append(row)
        elif row.kind == "before": pending.append(row)
        else: res.append(row)
    return res, False


def _mk_results(cls, items, capped, timed_out):
    res = cls(items)
    if capped: res.stop_reason = "max_results"
    elif timed_out: res.stop_reason = "timeout"
    return res


def _rg_post(rows, paths, count, max_results, timed_out):
    "Reduce collected rows to the requested `rg`/`rga` result form"
    if count: return sum(len(r.matches) for r in rows if r.kind == "match")
    if not paths: return _mk_results(SearchResults, *_cap_rows(rows, max_results), timed_out)
    seen,res,capped = set(),[],False
    for row in rows:
        if row.kind != "match" or row.path in seen: continue
        if max_results is not None and len(res) == max_results:
            capped = True
            break
        seen.add(row.path)
        res.append(row.path)
    return _mk_results(PathResults, res, capped, timed_out)



@delegates(_walk_args)
def rg(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    before_context:int=0, # Lines of context before each match, like `rg -B`
    after_context:int=0, # Lines of context after each match, like `rg -A`
    context:int=0, # Sets both before and after context, like `rg -C`
    paths:bool=False, # Return unique matched paths instead of rows
    count:bool=False, # Return total match span count instead of rows
    max_results:int|None=None, # Stop after this many matching rows; context rows of kept matches are included
    lnhash:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
    timeout_ms:int|None=None, # Cancel the search after this long and return partial results
    summary:bool=False, # Return one newline-escaped line per blank-line-delimited block?
    maxlen:int=120, # Maximum source characters per displayed block
    **kwargs
):
    "Search files and return `SearchResults`, matched paths, or a count; `lnhash=True` shows exhash-style addresses."
    assert not (paths and count), "paths and count are mutually exclusive"
    assert not (count and max_results), "count and max_results are mutually exclusive"
    assert not (count and timeout_ms is not None), "count and timeout_ms are mutually exclusive"
    assert not (summary and count), "summary and count are mutually exclusive"
    assert not (summary and paths), "summary and paths are mutually exclusive"
    before_context, after_context = _context(context, before_context, after_context)
    args = (pattern, _fs_path(root), *_walk_args(**kwargs), case_sensitive, smart_case, before_context, after_context)
    if summary:
        rows,timed_out = _core.block_search(*args, timeout_ms)
        return _block_post(rows, max_results, before_context, after_context, timed_out, maxlen, lnhash)
    if count: return sum(len(row.matches) for row in _core.rg_iter(*args) if row.kind == "match")
    if paths and timeout_ms is None:
        seen, res, capped = set(), [], False
        for row in _core.rg_iter(*args):
            if row.kind != "match" or row.path in seen: continue
            seen.add(row.path)
            res.append(row.path)
            if len(res) == max_results:
                capped = True
                break
        return _mk_results(PathResults, res, capped, False)
    rows, timed_out = _core.rg(*args, lnhash, timeout_ms)
    return _rg_post(rows, paths, False, max_results, timed_out)


@delegates(_walk_args)
def rg_iter(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    before_context:int=0, # Lines of context before each match, like `rg -B`
    after_context:int=0, # Lines of context after each match, like `rg -A`
    context:int=0, # Sets both before and after context, like `rg -C`
    lnhash:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
    **kwargs
) -> RgIter:
    "Search files lazily, yielding `SearchLine` rows; `lnhash=True` shows exhash-style addresses."
    before_context, after_context = _context(context, before_context, after_context)
    return _core.rg_iter(pattern, _fs_path(root), *_walk_args(**kwargs),
        case_sensitive, smart_case, before_context, after_context, lnhash)


@delegates(_walk_args)
async def rga(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    before_context:int=0, # Lines of context before each match, like `rg -B`
    after_context:int=0, # Lines of context after each match, like `rg -A`
    context:int=0, # Sets both before and after context, like `rg -C`
    paths:bool=False, # Return unique matched paths instead of rows
    count:bool=False, # Return total match span count instead of rows
    max_results:int|None=None, # Stop after this many matching rows; context rows of kept matches are included
    lnhash:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
    timeout_ms:int|None=None, # Cancel the search after this long and return partial results
    summary:bool=False, # Return one newline-escaped line per blank-line-delimited block?
    maxlen:int=120, # Maximum source characters per displayed block
    **kwargs
):
    "Async `rg`: search on Rust threads without blocking the event loop."
    assert not (paths and count), "paths and count are mutually exclusive"
    assert not (count and max_results), "count and max_results are mutually exclusive"
    assert not (count and timeout_ms is not None), "count and timeout_ms are mutually exclusive"
    assert not (summary and count), "summary and count are mutually exclusive"
    assert not (summary and paths), "summary and paths are mutually exclusive"
    before_context, after_context = _context(context, before_context, after_context)
    args = (pattern, _fs_path(root), *_walk_args(**kwargs), case_sensitive, smart_case, before_context, after_context)
    if summary:
        rows,timed_out = await _acall(_core.block_search_async, *args, timeout_ms)
        return _block_post(rows, max_results, before_context, after_context, timed_out, maxlen, lnhash)
    rows, timed_out = await _acall(_core.rg_async, *args, lnhash, timeout_ms)
    return _rg_post(rows, paths, count, max_results, timed_out)



async def _abatches(fn, *args):
    "Drive a `_core` *_iter_async op, yielding delivered row batches; cancel the op on early exit"
    loop = asyncio.get_running_loop()
    q = asyncio.Queue()
    def cb(rows, err):
        try: loop.call_soon_threadsafe(q.put_nowait, (rows, err))
        except RuntimeError: pass
    h = fn(cb, *args)
    try:
        while True:
            rows, err = await q.get()
            if err is not None: raise ValueError(err)
            if rows is None: return
            yield rows
    finally: h.cancel()


@delegates(_walk_args)
async def rga_iter(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    before_context:int=0, # Lines of context before each match, like `rg -B`
    after_context:int=0, # Lines of context after each match, like `rg -A`
    context:int=0, # Sets both before and after context, like `rg -C`
    lnhash:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
    batch_max:int=512, # Largest batch of rows delivered to the event loop at once
    **kwargs
):
    "Async `rg_iter`: yield `SearchLine` rows as they are found; early exit cancels the search."
    before_context, after_context = _context(context, before_context, after_context)
    async with aclosing(_abatches(_core.rg_iter_async, batch_max, pattern, _fs_path(root), *_walk_args(**kwargs),
        case_sensitive, smart_case, before_context, after_context, lnhash)) as batches:
        async for rows in batches:
            for row in rows: yield row


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


from .block import BlockResults, SearchBlock, _block_post
from .nb import NbCell, NbResults, nbrg, nbrg_iter, nbrga, nbrga_iter, search_nb

__all__ = [ "RgIter", "fd", "fda", "rg", "rga", "rg_iter", "rga_iter", "nbrg", "nbrg_iter", "nbrga", "nbrga_iter" ]
