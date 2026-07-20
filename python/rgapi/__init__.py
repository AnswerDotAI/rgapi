import asyncio, os
from contextlib import aclosing
from datetime import datetime
from functools import cached_property
from stat import S_ISDIR, S_ISLNK, filemode

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

MAX_REPR = 200 # Most rows shown by a `PathResults` repr

def _hsize(n):
    "Human-readable size, `ls -lh` style"
    for u in "BKMGTP":
        if n < 1024 or u == "P": break
        n /= 1024
    return f"{n:.0f}" if u == "B" else f"{n:.1f}{u}"

class FileEntry(str):
    "Relative path that lazily stats itself for `size`/`mtime`/`is_dir`/`link_target` and `ls -l`-style display"
    def __new__(cls, path, root=".", show_target=False):
        self = super().__new__(cls, path)
        self.root,self.show_target = os.path.abspath(root),show_target
        return self
    @cached_property
    def stat(self):
        "Cached `os.lstat` result; `None` if the path has vanished"
        try: return os.lstat(os.path.join(self.root, self))
        except OSError: return None
    @property
    def size(self): return None if self.stat is None else self.stat.st_size
    @property
    def mtime(self): return None if self.stat is None else datetime.fromtimestamp(self.stat.st_mtime)
    @property
    def is_dir(self): return self.stat is not None and S_ISDIR(self.stat.st_mode)
    @cached_property
    def link_target(self):
        "`os.readlink` result for a symlink; `None` otherwise"
        if self.stat is None or not S_ISLNK(self.stat.st_mode): return None
        try: return os.readlink(os.path.join(self.root, self))
        except OSError: return None
    def _line(self):
        if self.stat is None: return f"{'?':10} {'?':>7} {'?':16} {self}"
        tgt = f" -> {self.link_target}" if self.show_target and self.link_target is not None else ""
        return f"{filemode(self.stat.st_mode)} {_hsize(self.stat.st_size):>7} {self.mtime:%Y-%m-%d %H:%M} {self}{tgt}"
    def _repr_markdown_(self): return f"`{self._line()}`"

def _entry_root(root):
    "Stat base for `FileEntry`: a file root's entries are named relative to its parent"
    return os.path.dirname(root) if os.path.isfile(root) else root

def _fe(paths, root, show_target=False):
    root = _entry_root(root)
    return (FileEntry(p, root, show_target) for p in paths)

class PathResults(_Results):
    "List of relative `FileEntry` paths; repr is `ls -l`-style, `str()` is line-per-path"
    def __str__(self): return "\n".join(self)
    def __repr__(self):
        res = [p._line() if isinstance(p, FileEntry) else str(p) for p in self[:MAX_REPR]]
        if len(self) > MAX_REPR: res.append(f"… {len(self)-MAX_REPR:,} more")
        if self.stop_reason is not None: res.append(f"… truncated: {self.stop_reason}")
        return "\n".join(res)
    def _repr_pretty_(self, p, cycle): p.text("..." if cycle else repr(self))

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
    skip_dir:str|list|None=None, # Directory glob or globs to prune
    skip_dir_re:str|None=None, # Directory regex used to prune traversal
    files:bool=True, # Include files in results
    dirs:bool=False # Include directories in results
) -> PathResults:
    "Walk a directory and return relative file and/or directory paths."
    rt = _fs_path(root)
    return PathResults(_fe(_core.walk(rt, hidden, ignore, max_depth, min_depth, max_filesize, follow_links,
        same_file_system, path_re, skip_path_re, _listify(skip_dir), skip_dir_re, files, dirs), rt))


def _walk_args(
    glob:str|list|None=None, # Include glob or globs; alias for `include`
    include:str|list|None=None, # Include glob or globs, e.g. `*.py`
    exclude:str|list|None=None, # Exclude glob or globs, e.g. `test_*.py`
    ext:str|list|None=None, # Extension or extensions to require, without needing `*.`; ANDs with `include`/`glob`
    hidden:bool=False, # Include hidden files and directories
    ignore:bool=True, # Respect `.gitignore` and other ignore files
    max_depth:int|None=None, # Maximum directory depth to descend
    min_depth:int|None=None, # Minimum depth for returned/searched paths
    max_filesize:int|None=None, # Skip files larger than this many bytes
    follow_links:bool=False, # Follow symbolic links while walking
    same_file_system:bool=False, # Do not cross filesystem boundaries
    path_re:str|None=None, # Regex that relative paths must match
    skip_path_re:str|None=None, # Regex for relative paths to skip
    skip_dir:str|list|None=None, # Directory glob or globs to prune
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
    show_target:bool=False, # Append `-> target` to symlink rows in the display
    **kwargs
) -> PathResults:
    "Find paths with fd-style filters and gitignore support."
    rt = _fs_path(root)
    return PathResults(_fe(_core.find(rt, pattern, *_walk_args(**kwargs), files, dirs), rt, show_target))


@delegates(fd)
def ls(
    root:str|Path=".", # Directory to list (expands `~`)
    pattern:str|None=None, # Smart-case regex matched against each basename
    hidden:bool=False, # Include hidden files and directories, like `ls -a`
    dirs:bool=True, # Include directories in results
    max_depth:int|None=1, # Directory depth to list; 1 lists just `root`
    ignore:bool=False, # Respect `.gitignore` and other ignore files
    **kwargs
) -> PathResults:
    "List a directory like `ls`: one level, directories included, ignore rules off, sorted by name."
    res = fd(root, pattern, hidden=hidden, dirs=dirs, max_depth=max_depth, ignore=ignore, **kwargs)
    out = PathResults(sorted(res))
    out.stop_reason = res.stop_reason
    return out

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
    rt = _fs_path(root)
    return PathResults(_fe(await _acall(_core.find_async, rt, pattern, *_walk_args(**kwargs), files, dirs), rt))



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


def _rg_post(rows, paths, count, max_results, timed_out, root):
    "Reduce collected rows to the requested `rg`/`rga` result form"
    if count: return sum(len(r.matches) for r in rows if r.kind == "match")
    if not paths: return _mk_results(SearchResults, *_cap_rows(rows, max_results), timed_out)
    seen,res,capped,er = set(),[],False,_entry_root(root)
    for row in rows:
        if row.kind != "match" or row.path in seen: continue
        if max_results is not None and len(res) == max_results:
            capped = True
            break
        seen.add(row.path)
        res.append(FileEntry(row.path, er))
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
    lnhashs:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
    timeout_ms:int|None=None, # Cancel the search after this long and return partial results
    summary:bool=False, # Return one newline-escaped line per blank-line-delimited block?
    maxlen:int=120, # Maximum source characters per displayed block
    **kwargs
):
    "Search files and return `SearchResults`, matched paths, or a count; `lnhashs=True` shows exhash-style addresses."
    assert not (paths and count), "paths and count are mutually exclusive"
    assert not (count and max_results), "count and max_results are mutually exclusive"
    assert not (count and timeout_ms is not None), "count and timeout_ms are mutually exclusive"
    assert not (summary and count), "summary and count are mutually exclusive"
    assert not (summary and paths), "summary and paths are mutually exclusive"
    before_context, after_context = _context(context, before_context, after_context)
    rt = _fs_path(root)
    args = (pattern, rt, *_walk_args(**kwargs), case_sensitive, smart_case, before_context, after_context)
    if summary:
        rows,timed_out = _core.block_search(*args, timeout_ms)
        return _block_post(rows, max_results, before_context, after_context, timed_out, maxlen, lnhashs)
    if count: return sum(len(row.matches) for row in _core.rg_iter(*args) if row.kind == "match")
    if paths and timeout_ms is None:
        seen, res, capped, er = set(), [], False, _entry_root(rt)
        for row in _core.rg_iter(*args):
            if row.kind != "match" or row.path in seen: continue
            seen.add(row.path)
            res.append(FileEntry(row.path, er))
            if len(res) == max_results:
                capped = True
                break
        return _mk_results(PathResults, res, capped, False)
    rows, timed_out = _core.rg(*args, lnhashs, timeout_ms)
    return _rg_post(rows, paths, False, max_results, timed_out, rt)


@delegates(_walk_args)
def rg_iter(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    before_context:int=0, # Lines of context before each match, like `rg -B`
    after_context:int=0, # Lines of context after each match, like `rg -A`
    context:int=0, # Sets both before and after context, like `rg -C`
    lnhashs:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
    **kwargs
) -> RgIter:
    "Search files lazily, yielding `SearchLine` rows; `lnhashs=True` shows exhash-style addresses."
    before_context, after_context = _context(context, before_context, after_context)
    return _core.rg_iter(pattern, _fs_path(root), *_walk_args(**kwargs),
        case_sensitive, smart_case, before_context, after_context, lnhashs)


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
    lnhashs:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
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
    rt = _fs_path(root)
    args = (pattern, rt, *_walk_args(**kwargs), case_sensitive, smart_case, before_context, after_context)
    if summary:
        rows,timed_out = await _acall(_core.block_search_async, *args, timeout_ms)
        return _block_post(rows, max_results, before_context, after_context, timed_out, maxlen, lnhashs)
    rows, timed_out = await _acall(_core.rg_async, *args, lnhashs, timeout_ms)
    return _rg_post(rows, paths, count, max_results, timed_out, rt)



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
    lnhashs:bool=False, # Show `lineno|hash|` addresses instead of line numbers in row display
    batch_max:int=512, # Largest batch of rows delivered to the event loop at once
    **kwargs
):
    "Async `rg_iter`: yield `SearchLine` rows as they are found; early exit cancels the search."
    before_context, after_context = _context(context, before_context, after_context)
    async with aclosing(_abatches(_core.rg_iter_async, batch_max, pattern, _fs_path(root), *_walk_args(**kwargs),
        case_sensitive, smart_case, before_context, after_context, lnhashs)) as batches:
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

__all__ = [ "RgIter", "fd", "fda", "ls", "rg", "rga", "rg_iter", "rga_iter", "nbrg", "nbrg_iter", "nbrga", "nbrga_iter" ]
