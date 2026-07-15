import _thread, json, threading

import pytest

from rgapi import _core
from rgapi import BlockResults, PathResults, Regex, SearchResults, compile, fd, rg, rg_iter, search_path, search_text, walk


def make_tree(tmp_path):
    (tmp_path / ".gitignore").write_text("ignored.txt\n*.log\n")
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "app.py").write_text("alpha\nTODO here\nomega\n")
    (tmp_path / "src" / "skip.log").write_text("TODO log\n")
    (tmp_path / "ignored.txt").write_text("TODO ignored\n")
    (tmp_path / ".hidden").write_text("TODO hidden\n")
    (tmp_path / "bin.dat").write_bytes(b"TODO\0\n")
    (tmp_path / "bad.txt").write_bytes(b"TODO\xff\n")

class Pretty:
    def __init__(self): self.texts = []
    def text(self, text): self.texts.append(text)


def test_fd_is_relative_and_respects_ignore_hidden_and_globs(tmp_path):
    make_tree(tmp_path)
    found = set(fd(str(tmp_path)))
    assert "src/app.py" in found
    assert "src/skip.log" not in found
    assert "ignored.txt" not in found
    assert ".hidden" not in found
    assert all(not path.startswith(str(tmp_path)) for path in found)
    assert ".hidden" in set(fd(str(tmp_path), hidden=True))
    assert set(fd(str(tmp_path), glob="*.py")) == {"src/app.py"}
    assert set(fd(str(tmp_path), include="*.py")) == {"src/app.py"}
    assert set(fd(str(tmp_path), ext="py")) == {"src/app.py"}
    assert set(fd(str(tmp_path), exclude="*.py")) == {"bad.txt", "bin.dat"}
    assert set(walk(str(tmp_path), files=True, dirs=False)) == found

def test_ext_composes_as_and_with_include(tmp_path):
    from rgapi import nbrg
    make_tree(tmp_path)
    (tmp_path / "src" / "app.rs").write_text("alpha\nTODO here\n")
    write_nb(tmp_path / "one.ipynb", [_cell("code", ["foo = 1\n"], cid="c1")])
    write_nb(tmp_path / "two.ipynb", [_cell("code", ["foo = 2\n"], cid="c2")])
    assert set(fd(tmp_path, ext="py", include="src/*")) == {"src/app.py"}
    assert fd(tmp_path, ext="py", include="*.rs") == []
    assert set(fd(tmp_path, ext=["py", "rs"], include="app*")) == {"src/app.py", "src/app.rs"}
    assert [r.path for r in rg("TODO", tmp_path, ext="rs", include="app*")] == ["src/app.rs"]
    assert rg("TODO", tmp_path, ext="py", glob="*.rs", paths=True) == []
    assert [c.cell_id for c in nbrg("foo", tmp_path, include="one.ipynb")] == ["c1"]
    assert [c.cell_id for c in nbrg("foo", tmp_path, glob="two*")] == ["c2"]

def test_fd_pattern_is_basename_regex_with_smart_case(tmp_path):
    (tmp_path / "nested").mkdir()
    for name in ("App.py", "app.rs", "other.py"): (tmp_path / "nested" / name).touch()
    (tmp_path / "match-dir").mkdir()
    (tmp_path / "match-dir" / "other.txt").touch()

    assert set(fd(tmp_path, pattern=r"^app\.(py|rs)$")) == {"nested/App.py", "nested/app.rs"}
    assert fd(tmp_path, pattern=r"^App\.py$") == ["nested/App.py"]
    assert fd(tmp_path, pattern=r"match-dir") == []
    with pytest.raises(ValueError): fd(tmp_path, pattern=r"(")

def test_pathlike_arguments_and_expanduser(tmp_path, monkeypatch):
    make_tree(tmp_path)
    assert "src/app.py" in fd(tmp_path)
    assert walk(tmp_path, path_re=r"\.py$") == ["src/app.py"]
    assert [r.path for r in rg("TODO", tmp_path, include="*.py")] == ["src/app.py"]
    assert list(rg_iter("TODO", tmp_path, include="*.py")) == rg("TODO", tmp_path, include="*.py")
    matcher = compile("TODO")
    text_label = tmp_path / "memory.txt"
    assert search_text(matcher, "TODO\n", path=text_label)[0].path == str(text_label)
    display = tmp_path / "display.py"
    assert search_path(matcher, tmp_path / "src" / "app.py", display_path=display)[0].path == str(display)

    monkeypatch.setenv("HOME", str(tmp_path))
    assert fd("~", glob="*.py") == ["src/app.py"]


def test_path_filters_prune_dirs_and_follow_links(tmp_path):
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "app.py").write_text("TODO src\n")
    (tmp_path / "src" / "note.txt").write_text("TODO text\n")
    (tmp_path / "skip").mkdir()
    (tmp_path / "skip" / "app.py").write_text("TODO skip\n")
    (tmp_path / "b.py").write_text("TODO b\n")
    (tmp_path / "a.py").write_text("TODO a\n")

    assert set(fd(str(tmp_path), path_re=r"\.py$")) == {"a.py", "b.py", "skip/app.py", "src/app.py"}
    assert fd(str(tmp_path), path_re=r"src/.*\.py$") == ["src/app.py"]
    assert set(fd(str(tmp_path), path_re=r"\.py$", skip_path_re=r"(^|/)b\.py$", skip_dir="skip")) == {"a.py", "src/app.py"}
    assert walk(str(tmp_path), path_re=r"\.txt$") == ["src/note.txt"]
    assert {r.path for r in rg("TODO", str(tmp_path), path_re=r"\.py$", skip_dir_re=r"^skip$")} == {"a.py", "b.py", "src/app.py"}

    link = tmp_path / "linked"
    try: link.symlink_to(tmp_path / "src", target_is_directory=True)
    except OSError: return
    assert fd(str(tmp_path), path_re=r"linked/.*\.py$", follow_links=False) == []
    assert fd(str(tmp_path), path_re=r"linked/.*\.py$", follow_links=True) == ["linked/app.py"]

def test_rgignore_is_honored(tmp_path):
    (tmp_path / ".rgignore").write_text("only_rg.txt\n")
    (tmp_path / "only_rg.txt").write_text("hi\n")
    (tmp_path / "keep.txt").write_text("hi\n")
    assert set(fd(str(tmp_path))) == {"keep.txt"}
    assert set(fd(str(tmp_path), ignore=False)) == {"keep.txt", "only_rg.txt"}

def test_rgignore_can_override_gitignore(tmp_path):
    (tmp_path / ".gitignore").write_text("*/\n")
    (tmp_path / ".rgignore").write_text("!*/\n")
    (tmp_path / "sub").mkdir()
    (tmp_path / "sub" / "app.py").write_text("hi\n")
    assert "sub/app.py" in set(fd(str(tmp_path)))

def test_depth_size_and_filesystem_options(tmp_path):
    (tmp_path / "top.txt").write_text("TODO\n")
    sub = tmp_path / "sub"
    sub.mkdir()
    (sub / "small.txt").write_text("TODO\n")
    (sub / "large.txt").write_text("TODO large\n")

    assert fd(str(tmp_path), max_depth=1) == ["top.txt"]
    assert set(fd(str(tmp_path), min_depth=2)) == {"sub/large.txt", "sub/small.txt"}
    assert set(fd(str(tmp_path), max_filesize=5)) == {"sub/small.txt", "top.txt"}
    assert set(fd(str(tmp_path), same_file_system=True)) == {"sub/large.txt", "sub/small.txt", "top.txt"}
    assert [r.path for r in rg("TODO", str(tmp_path), min_depth=2, max_filesize=5)] == ["sub/small.txt"]


def test_lnhash_matches_stdlib_crc32(tmp_path):
    import zlib
    make_tree(tmp_path)
    row = rg("TODO", str(tmp_path))[0]
    assert row.lnhash == f"{row.line_number}|{zlib.crc32(row.line.encode()) & 0xffff:04x}|"


def test_rg_returns_structured_matches_context_and_relative_paths(tmp_path):
    make_tree(tmp_path)
    res = rg("TODO", str(tmp_path), context=1)
    assert isinstance(res, SearchResults)
    assert [(r.kind, r.path, r.line_number, r.line, r.matches) for r in res] == [
        ("before", "src/app.py", 1, "alpha", []),
        ("match", "src/app.py", 2, "TODO here", [(0, 4)]),
        ("after", "src/app.py", 3, "omega", [])]
    assert rg("TODO", str(tmp_path), include="*.py") == [res[1]]
    assert rg("TODO", str(tmp_path), ext="py") == [res[1]]
    assert rg("TODO", str(tmp_path), exclude="*.py") == []
    assert rg("TODO", str(tmp_path), max_depth=1) == []
    stream = rg_iter("TODO", str(tmp_path), context=1)
    assert iter(stream) is stream
    assert list(stream) == res
    assert list(rg_iter("TODO", str(tmp_path), include="*.py")) == [res[1]]
    assert rg("TODO", str(tmp_path), paths=True) == ["src/app.py"]
    assert rg("TODO", str(tmp_path), count=True) == 1
    try: rg("TODO", str(tmp_path), paths=True, count=True)
    except AssertionError as e: assert "mutually exclusive" in str(e)
    else: assert False
    addr = res[1].lnhash.split("|")
    assert addr[0] == "2" and len(addr[1]) == 4 and addr[2:] == [""]
    assert int(addr[1], 16) >= 0
    expected = (f'SearchLine(kind="match", path="src/app.py", line_number=2, lnhash="{res[1].lnhash}", '
        'line="TODO here", matches=[(0, 4)])')
    assert repr(res[1]) == expected
    assert str(res[0]) == "src/app.py-1-alpha"
    assert str(res[1]) == "src/app.py:2:TODO here"
    assert str(res) == "src/app.py-1-alpha\nsrc/app.py:2:TODO here\nsrc/app.py-3-omega"
    hashed = rg("TODO", str(tmp_path), context=1, lnhash=True)
    assert hashed == res
    assert str(hashed[0]) == f"src/app.py-{hashed[0].lnhash}alpha"
    assert str(hashed[1]) == f"src/app.py:{hashed[1].lnhash}TODO here"
    assert str(hashed) == "\n".join(str(row) for row in hashed)
    assert list(rg_iter("TODO", str(tmp_path), context=1, lnhash=True)) == hashed
    p = Pretty()
    res._repr_pretty_(p, False)
    assert p.texts == [str(res)]
    p = Pretty()
    res[1]._repr_pretty_(p, False)
    assert p.texts == [str(res[1])]
    assert repr(stream) == "RgIter(SearchLine stream)"
    assert str(stream) == repr(stream)


def test_rg_str_truncates_long_lines_to_120_chars(tmp_path):
    long = "x" * 200
    (tmp_path / "a.py").write_text(f"TODO {long}\n")
    short = "TODO é" + "y" * 130  # multibyte char before the cut point
    (tmp_path / "b.py").write_text(short + "\n")
    res = rg("TODO", str(tmp_path))
    line_a = next(r for r in res if r.path == "a.py")
    assert line_a.line == f"TODO {long}"                       # data stays full
    s = str(line_a)
    assert s == f"a.py:1:{('TODO ' + long)[:120]}…"       # display truncated + ellipsis
    assert repr(line_a).endswith(f'line="TODO {long}", matches=[(0, 4)])')
    line_b = next(r for r in res if r.path == "b.py")
    assert str(line_b) == f"b.py:1:{short[:120]}…"        # char-safe, no panic on é



def test_rg_summary_groups_blocks_with_block_context(tmp_path):
    src = "intro one\nintro two\n\nTODO first\nline\nTODO again\n   \nlast\n"
    (tmp_path/"a.py").write_text(src)
    res = rg("TODO", tmp_path, summary=True, context=1, maxlen=20)
    assert type(res) is BlockResults and res.complete
    assert [(b.kind,b.start_line,b.end_line) for b in res] == [
        ("context",1,2), ("match",4,6), ("context",8,8)]
    block = res[1]
    assert block.source == "TODO first\nline\nTODO again"
    assert [m.line_number for m in block.matches] == [4,6]
    assert str(block) == r"a.py:4-6:TODO first\nline\nTO…"
    assert str(res).splitlines()[0] == r"a.py:1-2-intro one\nintro two"
    assert res[0].asdict()["source"] == "intro one\nintro two"
    with pytest.raises(AssertionError): rg("TODO", tmp_path, summary=True, count=True)
    with pytest.raises(AssertionError): rg("TODO", tmp_path, summary=True, paths=True)
    hashed = rg("TODO", tmp_path, summary=True, lnhash=True, maxlen=20)
    hblock = hashed[0]
    assert str(hblock) == f"a.py:{hblock.start_lnhash},{hblock.end_lnhash}:TODO first\\nline\\nTO…"
    assert hblock.asdict()["start_lnhash"] == hblock.start_lnhash

    (tmp_path/"a.py").write_text("before\n\nTODO one\n\nbetween\n\nTODO two\n\nafter\n")
    limited = rg("TODO", tmp_path, summary=True, context=1, max_results=1)
    assert limited.stop_reason == "max_results"
    assert [(b.kind,b.source) for b in limited] == [
        ("context","before"), ("match","TODO one"), ("context","between")]

def test_worker_panic_surfaces_as_error_not_truncation(tmp_path):
    # A panic inside a parallel search/walk worker must raise, not silently end the
    # result stream (which would look like "no matches"). `_core.panic_probe` arms the
    # panic flag and runs the real search/walk machinery so the catch_unwind path is exercised.
    (tmp_path / "a.py").write_text("TODO here\n")
    with pytest.raises(Exception): _core.panic_probe(str(tmp_path))             # search workers
    with pytest.raises(Exception): _core.panic_probe(str(tmp_path), walk=True)  # walk workers


def test_search_path_skips_binary_and_invalid_utf8(tmp_path):
    (tmp_path / "bin.dat").write_bytes(b"TODO before\n\0TODO after\n")
    (tmp_path / "bad.txt").write_bytes(b"TODO\xff\n")
    matcher = compile("TODO")
    assert search_path(matcher, tmp_path / "bin.dat") == []
    assert search_path(matcher, tmp_path / "bad.txt") == []


def test_rg_keyboard_interrupt_cancels(tmp_path):
    (tmp_path / "big.txt").write_text("alpha beta gamma\n" * 1_000_000)
    timer = threading.Timer(0.001, _thread.interrupt_main)
    try:
        with pytest.raises(KeyboardInterrupt):
            timer.start()
            rg("needle_that_is_not_present", str(tmp_path))
    finally: timer.cancel()

def test_direct_regex_and_search_apis(tmp_path):
    make_tree(tmp_path)
    matcher = compile("todo")
    smart_matcher = compile("todo", smart_case=True)
    assert isinstance(matcher, Regex)
    assert not matcher.is_match("TODO")
    assert smart_matcher.is_match("TODO")
    assert matcher.finditer("todo TODO") == [(0, 4)]
    assert smart_matcher.finditer("todo TODO") == [(0, 4), (5, 9)]
    assert repr(matcher) == 'Regex("todo")'
    assert str(matcher) == repr(matcher)
    assert repr(compile("todo", case_sensitive=True)) == 'Regex("todo", case_sensitive=True)'
    assert repr(smart_matcher) == 'Regex("todo", smart_case=True)'
    assert compile("todo", case_sensitive=True).case_sensitive is True
    assert smart_matcher.smart_case is True
    text_res = search_text(smart_matcher, "zero\nTODO here\none\n", path="memory.txt", context=1)
    assert isinstance(text_res, SearchResults)
    assert [(r.kind, r.path, r.line_number, r.line) for r in text_res] == [
        ("before", "memory.txt", 1, "zero"),
        ("match", "memory.txt", 2, "TODO here"),
        ("after", "memory.txt", 3, "one")]
    path_res = search_path(smart_matcher, str(tmp_path / "src" / "app.py"), display_path="display.py")
    assert isinstance(path_res, SearchResults)
    assert [(r.kind, r.path, r.line_number, r.line, r.matches) for r in path_res] == [
        ("match", "display.py", 2, "TODO here", [(0, 4)])]
    assert path_res[0].asdict() == dict(kind="match", path="display.py", line_number=2,
        lnhash=path_res[0].lnhash, line="TODO here", matches=[(0, 4)])


def write_nb(path, cells):
    nb = dict(cells=cells, metadata={}, nbformat=4, nbformat_minor=5)
    path.write_text(json.dumps(nb))

def _cell(cell_type, source, cid=None, outputs=None):
    c = dict(cell_type=cell_type, metadata={}, source=source)
    if cell_type == "code":
        c["execution_count"] = None
        c["outputs"] = outputs or []
    if cid is not None: c["id"] = cid
    return c


def test_nbrg_source_only_with_cells(tmp_path):
    from rgapi import nbrg
    write_nb(tmp_path / "nb.ipynb", [
        _cell("code", ["import os\n", "def foo():\n", "    return 1\n"], cid="c1",
            outputs=[dict(output_type="stream", name="stdout", text=["foo ran\n"])]),
        _cell("code", ["print('hi')\n"], cid="c2",
            outputs=[dict(output_type="stream", name="stdout", text=["foo in output\n"])]),
        _cell("markdown", ["# Title\n", "use foo here\n"], cid="m1"),
    ])
    res = nbrg("foo", str(tmp_path))
    # c2 has 'foo' only in its output, so source-only search must skip it
    assert {c.cell_id for c in res} == {"c1", "m1"}
    c1 = next(c for c in res if c.cell_id == "c1")
    assert c1.cell_type == "code" and c1.kind == "match"
    assert [m.line_number for m in c1.matches] == [2]
    assert c1.matches[0].line == "def foo():"
    assert c1.matches[0].matches == [(4, 7)]
    m1 = next(c for c in res if c.cell_id == "m1")
    assert m1.cell_type == "markdown"
    assert [m.line for m in m1.matches] == ["use foo here"]


def test_nbrg_string_source_and_missing_id(tmp_path):
    from rgapi import nbrg
    write_nb(tmp_path / "nb.ipynb", [_cell("code", "x = 1\nfoo = 2\n")])
    res = nbrg("foo", str(tmp_path))
    assert len(res) == 1
    assert res[0].cell_id == "0"
    assert res[0].matches[0].line == "foo = 2"
    assert res[0].matches[0].line_number == 2


def test_nbrg_multiple_matches_single_cell_appears_once(tmp_path):
    from rgapi import nbrg
    write_nb(tmp_path / "nb.ipynb", [
        _cell("code", ["foo = 1\n", "bar = 2\n", "baz = foo + foo\n"], cid="c1"),
    ])
    res = nbrg("foo", str(tmp_path))
    assert len(res) == 1                                     # cell appears once, not once per match
    assert [c.cell_id for c in res] == ["c1"]
    cell = res[0]
    assert [m.line_number for m in cell.matches] == [1, 3]   # both matching lines kept
    assert cell.matches[1].matches == [(6, 9), (12, 15)]     # two spans on the same line


def test_nbrg_cell_context(tmp_path):
    from rgapi import nbrg
    write_nb(tmp_path / "nb.ipynb", [
        _cell("code", ["a = 1\n"], cid="c0"),
        _cell("code", ["target = 2\n"], cid="c1"),
        _cell("code", ["b = 3\n"], cid="c2"),
        _cell("code", ["c = 4\n"], cid="c3"),
    ])
    res = nbrg("target", str(tmp_path), cell_context=1)
    kinds = {c.cell_id: c.kind for c in res}
    assert kinds == dict(c0="context", c1="match", c2="context")


def test_file_as_root_searches_just_that_file(tmp_path):
    (tmp_path / "a.txt").write_text("hello TODO world\nsecond TODO\n")
    (tmp_path / "sub").mkdir()
    (tmp_path / "sub" / "b.txt").write_text("TODO in sub\n")
    write_nb(tmp_path / "nb.ipynb", [_cell("code", ["read_csv(x)\n"], cid="c1")])
    af = str(tmp_path / "a.txt")
    assert [str(r) for r in rg("TODO", af)] == ["a.txt:1:hello TODO world", "a.txt:2:second TODO"]
    assert rg("TODO", af, paths=True) == ["a.txt"]
    assert rg("TODO", af, count=True) == 2
    assert list(rg_iter("TODO", af)) == rg("TODO", af)
    assert fd(af) == ["a.txt"]
    # explicit file is searched even when gitignored
    (tmp_path / ".gitignore").write_text("a.txt\n")
    assert rg("TODO", af, paths=True) == ["a.txt"]
    from rgapi import nbrg
    assert [c.cell_id for c in nbrg("read_csv", str(tmp_path / "nb.ipynb"))] == ["c1"]
    with pytest.raises(Exception): rg("TODO", str(tmp_path / "nope.txt"))


def test_nbrg_skips_bad_json(tmp_path):
    from rgapi import nbrg
    write_nb(tmp_path / "good.ipynb", [_cell("code", ["read_csv(x)\n"], cid="g1")])
    (tmp_path / "bad.ipynb").write_text("{not valid json")
    a = nbrg("read_csv", str(tmp_path))
    assert {c.cell_id for c in a} == {"g1"}


def test_search_nb_single_and_asdict_str(tmp_path):
    from rgapi import search_nb
    p = tmp_path / "nb.ipynb"
    write_nb(p, [_cell("code", ["def foo():\n", "    return 1\n"], cid="c1")])
    res = search_nb("foo", p, display_path="nb.ipynb")
    assert len(res) == 1
    cell = res[0]
    assert str(cell) == "nb.ipynb:c1:def foo():\\n    return 1"   # cell-oriented: whole cell, newlines escaped
    d = cell.asdict()
    assert d["cell_id"] == "c1" and d["cell_type"] == "code" and d["kind"] == "match"
    assert d["matches"][0]["line"] == "def foo():"


def test_nbcell_str_truncates_and_escapes(tmp_path):
    from rgapi import search_nb
    p = tmp_path / "nb.ipynb"
    write_nb(p, [_cell("code", ["x = '" + "a" * 500 + "'  # foo\n"], cid="c1")])
    s = str(search_nb("foo", p, display_path="nb.ipynb")[0])
    assert "\n" not in s            # newlines escaped to a single display line
    assert s.endswith("…")          # long cell is truncated
    assert s.startswith("nb.ipynb:c1:")
    assert str(search_nb("foo", p, display_path="nb.ipynb", maxlen=10)[0]) == "nb.ipynb:c1:x = 'aaaaa…"


def test_max_results_and_count(tmp_path):
    from rgapi import nbrg
    make_tree(tmp_path)
    (tmp_path / "src" / "more.py").write_text("TODO one\nplain\nTODO two\n")
    res = rg("TODO", str(tmp_path), max_results=2)
    assert [r.kind for r in res] == ["match", "match"]
    assert len(rg("TODO", str(tmp_path))) == 3
    # context lines ride along with kept matches only
    ctx = rg("TODO", str(tmp_path), context=1, max_results=1)
    assert [r.kind for r in ctx].count("match") == 1
    capped = rg("TODO", str(tmp_path), paths=True, max_results=1)
    assert len(capped) == 1 and capped[0] in ("src/app.py", "src/more.py")  # winner is racy: order is not contractual
    assert capped.stop_reason == "max_results"
    try: rg("TODO", str(tmp_path), count=True, max_results=1)
    except AssertionError as e: assert "mutually exclusive" in str(e)
    else: assert False

    write_nb(tmp_path / "nb.ipynb", [
        _cell("code", ["foo = 1\n"], cid="c1"),
        _cell("code", ["foo = 2\n"], cid="c2"),
        _cell("code", ["foo = 3\n"], cid="c3"),
    ])
    assert [c.cell_id for c in nbrg("foo", str(tmp_path), max_results=2)] == ["c1", "c2"]
    assert nbrg("foo", str(tmp_path), count=True) == 3
    try: nbrg("foo", str(tmp_path), max_reslts=2)
    except TypeError as e: assert "max_reslts" in str(e)
    else: assert False


def test_pathresults_and_stop_reason(tmp_path):
    make_tree(tmp_path)
    (tmp_path / "more.py").write_text("TODO one\nTODO two\n")
    found = fd(tmp_path)
    assert type(found) is PathResults and isinstance(found, list)
    assert found.complete and found.stop_reason is None
    assert str(found) == "\n".join(found)
    assert type(walk(tmp_path)) is PathResults
    assert type(rg("TODO", tmp_path, paths=True)) is PathResults

    full = rg("TODO", tmp_path)
    assert full.complete and full.stop_reason is None and len(full) == 3
    res = rg("TODO", tmp_path, max_results=1)
    assert res.stop_reason == "max_results" and not res.complete
    ps = rg("TODO", tmp_path, paths=True, max_results=1)
    assert ps.stop_reason == "max_results" and len(ps) == 1

    res = rg("TODO", tmp_path, timeout_ms=10_000)
    assert res.stop_reason is None and sorted(map(str, res)) == sorted(map(str, full))
    assert rg("TODO", tmp_path, timeout_ms=0).stop_reason == "timeout"
    ps = rg("TODO", tmp_path, paths=True, timeout_ms=10_000)
    assert ps.complete and sorted(ps) == sorted(rg("TODO", tmp_path, paths=True))
    with pytest.raises(AssertionError): rg("TODO", tmp_path, count=True, timeout_ms=1)


def test_nbrg_stop_reason(tmp_path):
    from rgapi import NbResults, nbrg
    write_nb(tmp_path / "nb.ipynb", [_cell("code", "foo a\n"), _cell("code", "foo b\n")])
    res = nbrg("foo", str(tmp_path))
    assert type(res) is NbResults and res.complete and res.stop_reason is None
    res = nbrg("foo", str(tmp_path), max_results=1)
    assert res.stop_reason == "max_results" and not res.complete and len(res) == 1
    assert nbrg("foo", str(tmp_path), timeout_ms=10_000).complete
    assert nbrg("foo", str(tmp_path), timeout_ms=0).stop_reason == "timeout"
    with pytest.raises(AssertionError): nbrg("foo", str(tmp_path), count=True, timeout_ms=1)


def test_fileentry_and_ls(tmp_path):
    import rgapi
    from rgapi import FileEntry, ls
    make_tree(tmp_path)
    found = fd(tmp_path)
    e = next(p for p in found if p == "src/app.py")
    assert type(e) is FileEntry and isinstance(e, str)
    st = e.stat
    assert st is not None and e.stat is st                    # lazy stat, cached
    assert e.size == st.st_size and not e.is_dir
    assert abs(e.mtime.timestamp() - st.st_mtime) < 2
    assert isinstance(walk(tmp_path)[0], FileEntry)
    assert isinstance(rg("TODO", tmp_path, paths=True)[0], FileEntry)

    r = repr(found)                                           # long format by default
    line = next(l for l in r.splitlines() if l.endswith("src/app.py"))
    assert line.startswith("-") and str(st.st_size) in line
    assert str(found) == "\n".join(found)                     # str() stays plain paths
    assert "src/app.py" in e._repr_markdown_()

    gone = FileEntry("nope.txt", str(tmp_path))               # vanished files render, not raise
    assert gone.stat is None and gone.size is None and not gone.is_dir
    assert "?" in repr(PathResults([gone]))

    pr = PathResults(FileEntry(f"f{i}", str(tmp_path)) for i in range(rgapi.MAX_REPR + 50))
    r = repr(pr)
    assert len(r.splitlines()) == rgapi.MAX_REPR + 1 and "50 more" in r
    pr.stop_reason = "timeout"
    assert "timeout" in repr(pr)

    res = ls(tmp_path)
    assert type(res) is PathResults and list(res) == sorted(res)
    assert "src" in res and "ignored.txt" in res              # dirs listed; ignore files not consulted
    assert "src/app.py" not in res and ".hidden" not in res   # one level, hidden off
    assert ".hidden" in ls(tmp_path, hidden=True)


def test_file_root_ignores_siblings_and_depth_cap_permissions(tmp_path):
    f = tmp_path / "f.txt"
    f.write_text("hello x\n")
    (tmp_path / ".gitignore").write_text("f.txt\n.hid.txt\n")
    hid = tmp_path / ".hid.txt"
    hid.write_text("hello x\n")
    locked = tmp_path / "locked"
    locked.mkdir()
    (locked / "g.txt").write_text("x\n")
    locked.chmod(0o000)
    try:
        assert [r.path for r in rg("x", f)] == ["f.txt"]          # sibling perms irrelevant to file root
        assert [r.path for r in rg("x", hid)] == [".hid.txt"]     # hidden+ignored file root still searched
        assert list(fd(f)) == ["f.txt"]                           # fd file root; sibling perms irrelevant
        assert fd(f)[0].size == 8                                 # FileEntry stat resolves correctly
        got = sorted({r.path for r in rg("x", tmp_path, max_depth=1, ignore=False, hidden=True)})
        assert got == [".gitignore", ".hid.txt", "f.txt"]                       # depth-cap dir skipped silently
        assert list(fd(tmp_path, max_depth=1, ignore=False)) == ["f.txt"]
        with pytest.raises(ValueError, match="ermission"):
            rg("x", tmp_path)                                     # uncapped: unreadable dir in tree is fatal
        with pytest.raises(ValueError, match="ermission"):
            rg("x", tmp_path, max_depth=2)                        # cap above the dir: descent needed, fatal
    finally: locked.chmod(0o755)
