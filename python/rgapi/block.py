"Block summaries for `rg(summary=True)`."

from . import _Results, _mk_results, _preview


class SearchBlock:
    "A blank-line-delimited source block containing a match, or context for one."
    def __init__(self, path, block_index, start_line, end_line, start_lnhash, end_lnhash, kind, source, matches, maxlen=120, display_lnhash=False):
        self.path,self.block_index,self.start_line,self.end_line = path,block_index,start_line,end_line
        self.start_lnhash,self.end_lnhash,self.display_lnhash = start_lnhash,end_lnhash,display_lnhash
        self.kind,self.source,self.matches,self.maxlen = kind,source,matches,maxlen

    def asdict(self):
        return dict(path=self.path, block_index=self.block_index, start_line=self.start_line, end_line=self.end_line,
            start_lnhash=self.start_lnhash, end_lnhash=self.end_lnhash, kind=self.kind, source=self.source,
            matches=[m.asdict() for m in self.matches])

    def __repr__(self):
        return (f"SearchBlock(path={self.path!r}, start_line={self.start_line}, end_line={self.end_line}, "
            f"kind={self.kind!r}, matches={len(self.matches)})")

    def __str__(self):
        if self.display_lnhash: lines = self.start_lnhash if self.start_line == self.end_line else f"{self.start_lnhash},{self.end_lnhash}"
        else: lines = str(self.start_line) if self.start_line == self.end_line else f"{self.start_line}-{self.end_line}"
        sep = ":" if self.kind == "match" else "-"
        return f"{self.path}:{lines}{sep}{_preview(self.source, self.maxlen)}"

    def _repr_pretty_(self, p, cycle): p.text("..." if cycle else str(self))


class BlockResults(_Results):
    "List of `SearchBlock` rows with one-line block summary display."
    def __str__(self): return "\n".join(map(str, self))


def _row_to_block(row, maxlen=120, display_lnhash=False):
    path,bi,start,end,start_hash,end_hash,kind,source,matches = row
    return SearchBlock(path, bi, start, end, start_hash, end_hash, kind, source, list(matches), maxlen, display_lnhash)

def _cap_blocks(blocks, max_results, before_context, after_context):
    if max_results is None: return blocks,False
    matches = [b for b in blocks if b.kind == "match"]
    kept = matches[:max_results]
    keys = {(b.path,b.block_index) for b in kept}
    def _keep(b):
        if b.kind == "match": return (b.path,b.block_index) in keys
        return any(b.path == m.path and m.block_index-before_context <= b.block_index <= m.block_index+after_context for m in kept)
    return [b for b in blocks if _keep(b)],len(matches)>max_results

def _block_post(rows, max_results, before_context, after_context, timed_out, maxlen, display_lnhash=False):
    blocks = [_row_to_block(row, maxlen, display_lnhash) for row in rows]
    blocks.sort(key=lambda b: (b.path,b.block_index))
    return _mk_results(BlockResults, *_cap_blocks(blocks, max_results, before_context, after_context), timed_out)
