"Notebook-aware search: run `rg`-style search over `.ipynb` files, returning matched cells instead of raw JSON lines."

import json
from pathlib import Path
from fastcore.meta import delegates

from . import compile, fd, rg, search_text


def _cell_source(cell):
    src = cell.get("source", "")
    return "".join(src) if isinstance(src, list) else src

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


def search_nb(
    matcher,                 # Compiled `Regex` from `compile()`
    path,                    # Notebook file to search (expands `~`)
    cell_context:int=0,      # Cells of context to include before/after each matching cell
    display_path:str=None    # Path stored in results; defaults to `path`
) -> NbResults:
    "Search one `.ipynb` file's cell sources with a compiled matcher."
    disp = display_path if display_path is not None else str(path)
    try: nb = json.loads(Path(path).expanduser().read_text())
    except (ValueError, OSError, UnicodeDecodeError): return NbResults()
    cells = nb.get("cells")
    if not isinstance(cells, list): return NbResults()
    info,matched = {},{}
    for i,cell in enumerate(cells):
        if not isinstance(cell, dict): continue
        src = _cell_source(cell)
        info[i] = (str(cell.get("id", i)), cell.get("cell_type", ""), src)
        hits = [m for m in search_text(matcher, src, path=disp) if m.kind == "match"]
        if hits: matched[i] = hits
    if not matched: return NbResults()
    emit = {i:"match" for i in matched}
    if cell_context:
        for i in matched:
            for j in range(max(0, i-cell_context), min(len(cells), i+cell_context+1)): emit.setdefault(j, "context")
    res = NbResults()
    for i in sorted(emit):
        if i not in info: continue
        cid,ctype,src = info[i]
        res.append(NbCell(disp, i, cid, ctype, emit[i], src, matched.get(i, [])))
    return res


@delegates(fd, but=["ext", "files", "dirs"])
def nbrg(
    pattern:str,                  # Regex pattern to search for
    root:str=".",                 # Directory to search (expands `~`)
    prefilter:bool=False,         # Pre-narrow notebooks with `rg` before per-cell search (may miss escape-affected patterns)
    cell_context:int=0,           # Cells of context to include before/after each matching cell
    case_sensitive:bool|None=None,# True/False forces case; None allows `smart_case`
    smart_case:bool=False,        # Match `rg --smart-case` behavior
    **kwargs
) -> NbResults:
    "Search `.ipynb` cell sources under `root`, returning matched cells."
    matcher = compile(pattern, case_sensitive=case_sensitive, smart_case=smart_case)
    if prefilter: paths = rg(pattern, root, ext="ipynb", case_sensitive=case_sensitive, smart_case=smart_case, paths=True, **kwargs)
    else: paths = fd(root, ext="ipynb", **kwargs)
    base = Path(root).expanduser()
    res = NbResults()
    for rel in sorted(paths): res += search_nb(matcher, base/rel, cell_context=cell_context, display_path=rel)
    return res
