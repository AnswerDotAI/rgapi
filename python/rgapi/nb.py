"Notebook-aware search: run `rg`-style search over `.ipynb` files, returning matched cells instead of raw JSON lines."

from pathlib import Path
from fastcore.meta import delegates

from contextlib import aclosing

from . import _core, _walk_args, _fs_path, _acall, _abatches, _mk_results, _preview, _Results



class NbCell:
    "A notebook cell that matched (or provides context for) a search."
    def __init__(self, path, cell_index, cell_id, cell_type, kind, source, matches, maxlen=120):
        self.path,self.cell_index,self.cell_id = path,cell_index,cell_id
        self.cell_type,self.kind,self.source,self.matches,self.maxlen = cell_type,kind,source,matches,maxlen

    def asdict(self):
        return dict(path=self.path, cell_index=self.cell_index, cell_id=self.cell_id, cell_type=self.cell_type, kind=self.kind, source=self.source,
            matches=[m.asdict() for m in self.matches])

    def __repr__(self):
        return (f"NbCell(path={self.path!r}, cell_id={self.cell_id!r}, cell_type={self.cell_type!r}, "
            f"kind={self.kind!r}, matches={len(self.matches)})")

    def __str__(self):
        sep = ":" if self.kind == "match" else "-"
        return f"{self.path}:{self.cell_id}{sep}{_preview(self.source, self.maxlen)}"

    def _repr_pretty_(self, p, cycle): p.text("..." if cycle else str(self))


class NbResults(_Results):
    "List of `NbCell` rows with rg-style text display."
    def __str__(self): return "\n".join(map(str, self))


def _row_to_cell(row, maxlen=120):
    p,ci,cid,ct,kind,src,matches = row
    return NbCell(p, ci, cid, ct, kind, src, list(matches), maxlen)
def _rows_to_cells(rows, maxlen=120): return [_row_to_cell(row, maxlen) for row in rows]


def search_nb(
    pattern:str,                  # Regex pattern to search for
    path,                         # Notebook file to search (expands `~`)
    cell_context:int=0,           # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None,# True/False forces case; None allows `smart_case`
    smart_case:bool=False,        # Match `rg --smart-case` behavior
    display_path:str=None,        # Path stored in results; defaults to `path`
    maxlen:int=120,                # Maximum source characters per displayed cell
) -> NbResults:
    "Search one `.ipynb` file's cell sources, returning matched cells."
    disp = display_path if display_path is not None else str(path)
    rows = _core.nb_search_file(pattern, _fs_path(path), disp, case_sensitive=case_sensitive,
        smart_case=smart_case, cell_context=cell_context)
    res = NbResults(_rows_to_cells(rows, maxlen))
    res.sort(key=lambda c: c.cell_index)
    return res


def _nb_post(rows, max_results, count, timed_out, maxlen):
    "Reduce collected notebook rows to the requested `nbrg`/`nbrga` result form"
    res = _rows_to_cells(rows, maxlen)
    res.sort(key=lambda c: (c.path, c.cell_index))
    if count: return len(res)
    capped = max_results is not None and len(res) > max_results
    return _mk_results(NbResults, res[:max_results] if capped else res, capped, timed_out)


@delegates(_walk_args, but=["ext"])
def nbrg(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    cell_context:int=0, # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    max_results:int|None=None, # Return at most this many cells
    count:bool=False, # Return the number of matching cells instead of results
    timeout_ms:int|None=None, # Cancel the search after this long and return partial results
    maxlen:int=120, # Maximum source characters per displayed cell
    **kwargs
) -> NbResults:
    "Search `.ipynb` cell sources under `root` in parallel, returning matched cells."
    assert not (count and max_results), "count and max_results are mutually exclusive"
    assert not (count and timeout_ms is not None), "count and timeout_ms are mutually exclusive"
    rows, timed_out = _core.nb_search(pattern, _fs_path(root), *_walk_args(ext="ipynb", **kwargs),
        case_sensitive, smart_case, cell_context, timeout_ms)
    return _nb_post(rows, max_results, count, timed_out, maxlen)


@delegates(_walk_args, but=["ext"])
def nbrg_iter(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    cell_context:int=0, # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    maxlen:int=120, # Maximum source characters per displayed cell
    **kwargs
):
    "Search `.ipynb` cell sources lazily, yielding `NbCell` rows as they are found."
    it = _core.nb_iter(pattern, _fs_path(root), *_walk_args(ext="ipynb", **kwargs), case_sensitive, smart_case, cell_context)
    return (_row_to_cell(row, maxlen) for row in it)


@delegates(_walk_args, but=["ext"])
async def nbrga(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    cell_context:int=0, # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    max_results:int|None=None, # Return at most this many cells
    count:bool=False, # Return the number of matching cells instead of results
    timeout_ms:int|None=None, # Cancel the search after this long and return partial results
    maxlen:int=120, # Maximum source characters per displayed cell
    **kwargs
) -> NbResults:
    "Async `nbrg`: search notebooks on Rust threads without blocking the event loop."
    assert not (count and max_results), "count and max_results are mutually exclusive"
    assert not (count and timeout_ms is not None), "count and timeout_ms are mutually exclusive"
    rows, timed_out = await _acall(_core.nb_search_async, pattern, _fs_path(root),
        *_walk_args(ext="ipynb", **kwargs), case_sensitive, smart_case, cell_context, timeout_ms)
    return _nb_post(rows, max_results, count, timed_out, maxlen)


@delegates(_walk_args, but=["ext"])
async def nbrga_iter(
    pattern:str, # Regex pattern to search for
    root:str|Path=".", # Directory to search (expands `~`)
    cell_context:int=0, # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None, # True/False forces case; None allows `smart_case`
    smart_case:bool=False, # Match `rg --smart-case` behavior
    batch_max:int=512, # Largest batch of cells delivered to the event loop at once
    maxlen:int=120, # Maximum source characters per displayed cell
    **kwargs
):
    "Async `nbrg_iter`: yield `NbCell` rows as they are found; early exit cancels the search."
    async with aclosing(_abatches(_core.nb_iter_async, batch_max, pattern, _fs_path(root),
        *_walk_args(ext="ipynb", **kwargs), case_sensitive, smart_case, cell_context)) as batches:
        async for rows in batches:
            for row in rows: yield _row_to_cell(row, maxlen)
