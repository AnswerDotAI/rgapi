import asyncio, time
from contextlib import aclosing

import pytest
from fastcore.aio import run_sync

from rgapi import PathResults, fd, fda, rg, rga, rga_iter
from test_rgapi import make_tree


def srt(rows): return sorted(map(str, rows))

def _more(tmp_path):
    make_tree(tmp_path)
    (tmp_path / "more.py").write_text("TODO one\nTODO two\n")


def test_async_results_match_sync(tmp_path):
    _more(tmp_path)
    found = run_sync(fda(tmp_path))
    assert type(found) is PathResults and sorted(found) == sorted(fd(tmp_path))
    assert sorted(run_sync(fda(tmp_path, glob="*.py"))) == sorted(fd(tmp_path, glob="*.py"))
    assert srt(run_sync(rga("TODO", tmp_path))) == srt(rg("TODO", tmp_path))
    assert run_sync(rga("TODO", tmp_path, count=True)) == rg("TODO", tmp_path, count=True)
    assert sorted(run_sync(rga("TODO", tmp_path, paths=True))) == sorted(rg("TODO", tmp_path, paths=True))
    assert run_sync(rga("TODO", tmp_path, max_results=1)).stop_reason == "max_results"
    assert run_sync(rga("TODO", tmp_path, timeout_ms=0)).stop_reason == "timeout"
    with pytest.raises(ValueError): run_sync(rga("(bad", tmp_path))


def test_rga_iter(tmp_path):
    _more(tmp_path)
    async def all_rows(): return [row async for row in rga_iter("TODO", tmp_path)]
    assert srt(run_sync(all_rows())) == srt(rg("TODO", tmp_path))
    async def first_then_close():
        async with aclosing(rga_iter("TODO", tmp_path)) as it:
            async for row in it: return row
    assert run_sync(first_then_close()).kind == "match"
    async def bad(): return [r async for r in rga_iter("(bad", tmp_path)]
    with pytest.raises(ValueError): run_sync(bad())


def test_cancellation_no_hang(tmp_path):
    (tmp_path / "big.txt").write_text("match line\n" * 200_000)
    async def go():
        t0 = time.monotonic()
        try: await asyncio.wait_for(rga("match", tmp_path), 0.05)
        except asyncio.TimeoutError: pass
        return time.monotonic() - t0
    assert run_sync(go()) < 5


def _nb_tree(tmp_path):
    from test_rgapi import write_nb, _cell
    write_nb(tmp_path / "a.ipynb", [_cell("code", "TODO alpha\n"), _cell("code", "clean\n")])
    write_nb(tmp_path / "b.ipynb", [_cell("code", "x = 1\n"), _cell("markdown", "TODO beta\n")])

def _key(c): return (c.path, c.cell_id)

def test_nb_stream_and_async(tmp_path):
    from rgapi import NbResults, nbrg, nbrg_iter, nbrga, nbrga_iter
    _nb_tree(tmp_path)
    sync = nbrg("TODO", tmp_path)
    assert len(sync) == 2
    assert sorted(map(_key, nbrg_iter("TODO", tmp_path))) == sorted(map(_key, sync))
    got = run_sync(nbrga("TODO", tmp_path))
    assert type(got) is NbResults and sorted(map(_key, got)) == sorted(map(_key, sync))
    assert run_sync(nbrga("TODO", tmp_path, count=True)) == nbrg("TODO", tmp_path, count=True)
    assert run_sync(nbrga("TODO", tmp_path, max_results=1)).stop_reason == "max_results"
    assert run_sync(nbrga("TODO", tmp_path, timeout_ms=0)).stop_reason == "timeout"
    async def all_cells(): return [c async for c in nbrga_iter("TODO", tmp_path)]
    assert sorted(map(_key, run_sync(all_cells()))) == sorted(map(_key, sync))
    async def first_then_close():
        async with aclosing(nbrga_iter("TODO", tmp_path)) as it:
            async for c in it: return c
    assert run_sync(first_then_close()).kind == "match"
    async def bad(): return [c async for c in nbrga_iter("(bad", tmp_path)]
    with pytest.raises(ValueError): run_sync(bad())
