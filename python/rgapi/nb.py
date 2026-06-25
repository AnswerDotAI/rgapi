"Notebook-aware search: run `rg`-style search over `.ipynb` files, returning matched cells instead of raw JSON lines."

from pathlib import Path
from fastcore.meta import delegates

from . import _core, fd, _filters, _fs_path, _listify


def _preview(text, width=120):
    text = text.rstrip("\n").replace("\n", "\\n")
    return text if len(text) <= width else text[:width] + "…"


class NbCell:
    "A notebook cell that matched (or provides context for) a search."
    def __init__(self, path, cell_index, cell_id, cell_type, kind, source, matches):
        self.path,self.cell_index,self.cell_id = path,cell_index,cell_id
        self.cell_type,self.kind,self.source,self.matches = cell_type,kind,source,matches

    def asdict(self):
        return dict(path=self.path, cell_index=self.cell_index, cell_id=self.cell_id,
            cell_type=self.cell_type, kind=self.kind, source=self.source,
            matches=[m.asdict() for m in self.matches])

    def __repr__(self):
        return (f"NbCell(path={self.path!r}, cell_id={self.cell_id!r}, cell_type={self.cell_type!r}, "
                f"kind={self.kind!r}, matches={len(self.matches)})")

    def __str__(self):
        sep = ":" if self.kind == "match" else "-"
        return f"{self.path}:{self.cell_id}{sep}{_preview(self.source)}"

    def _repr_pretty_(self, p, cycle): p.text("..." if cycle else str(self))


class NbResults(list):
    "List of `NbCell` rows with rg-style text display."
    def __str__(self): return "\n".join(map(str, self))
    def _repr_pretty_(self, p, cycle): p.text("..." if cycle else str(self))


def _rows_to_cells(rows):
    return [NbCell(p, ci, cid, ct, kind, src, list(matches)) for p,ci,cid,ct,kind,src,matches in rows]


def search_nb(
    pattern:str,                  # Regex pattern to search for
    path,                         # Notebook file to search (expands `~`)
    cell_context:int=0,           # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None,# True/False forces case; None allows `smart_case`
    smart_case:bool=False,        # Match `rg --smart-case` behavior
    display_path:str=None         # Path stored in results; defaults to `path`
) -> NbResults:
    "Search one `.ipynb` file's cell sources, returning matched cells."
    disp = display_path if display_path is not None else str(path)
    rows = _core.nb_search_file(pattern, _fs_path(path), disp, case_sensitive=case_sensitive,
        smart_case=smart_case, cell_context=cell_context)
    res = NbResults(_rows_to_cells(rows))
    res.sort(key=lambda c: c.cell_index)
    return res


@delegates(fd, but=["ext", "files", "dirs"])
def nbrg(
    pattern:str,                  # Regex pattern to search for
    root:str=".",                 # Directory to search (expands `~`)
    cell_context:int=0,           # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None,# True/False forces case; None allows `smart_case`
    smart_case:bool=False,        # Match `rg --smart-case` behavior
    **kwargs
) -> NbResults:
    "Search `.ipynb` cell sources under `root` in parallel, returning matched cells."
    includes, excludes = _filters(kwargs.pop("glob", None), kwargs.pop("include", None), kwargs.pop("exclude", None), "ipynb")
    rows = _core.nb_search(pattern, _fs_path(root), includes, excludes,
        kwargs.pop("hidden", False), kwargs.pop("ignore", True), kwargs.pop("max_depth", None),
        kwargs.pop("min_depth", None), kwargs.pop("max_filesize", None), kwargs.pop("follow_links", False),
        kwargs.pop("same_file_system", False), kwargs.pop("path_re", None), kwargs.pop("skip_path_re", None),
        _listify(kwargs.pop("skip_dir", None)), kwargs.pop("skip_dir_re", None),
        case_sensitive, smart_case, cell_context)
    res = NbResults(_rows_to_cells(rows))
    res.sort(key=lambda c: (c.path, c.cell_index))
    return res
