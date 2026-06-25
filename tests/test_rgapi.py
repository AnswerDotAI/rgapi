import _thread, json, threading

import pytest

from rgapi import _core
from rgapi import Regex, SearchResults, compile, fd, rg, rg_iter, search_path, search_text, walk


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
    assert repr(res[1]) == 'SearchLine(kind="match", path="src/app.py", line_number=2, line="TODO here", matches=[(0, 4)])'
    assert str(res[0]) == "src/app.py-1-alpha"
    assert str(res[1]) == "src/app.py:2:TODO here"
    assert str(res) == "src/app.py-1-alpha\nsrc/app.py:2:TODO here\nsrc/app.py-3-omega"
    p = Pretty()
    res._repr_pretty_(p, False)
    assert p.texts == [str(res)]
    p = Pretty()
    res[1]._repr_pretty_(p, False)
    assert p.texts == [str(res[1])]
    assert repr(stream) == "RgIter(SearchLine stream)"
    assert str(stream) == repr(stream)


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
    assert path_res[0].asdict() == dict(kind="match", path="display.py", line_number=2, line="TODO here", matches=[(0, 4)])


def write_nb(path, cells):
    nb = {"cells": cells, "metadata": {}, "nbformat": 4, "nbformat_minor": 5}
    path.write_text(json.dumps(nb))

def _cell(cell_type, source, cid=None, outputs=None):
    c = {"cell_type": cell_type, "metadata": {}, "source": source}
    if cell_type == "code":
        c["execution_count"] = None
        c["outputs"] = outputs or []
    if cid is not None: c["id"] = cid
    return c


def test_nbrg_source_only_with_cells(tmp_path):
    from rgapi import nbrg
    write_nb(tmp_path / "nb.ipynb", [
        _cell("code", ["import os\n", "def foo():\n", "    return 1\n"], cid="c1",
              outputs=[{"output_type": "stream", "name": "stdout", "text": ["foo ran\n"]}]),
        _cell("code", ["print('hi')\n"], cid="c2",
              outputs=[{"output_type": "stream", "name": "stdout", "text": ["foo in output\n"]}]),
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
    assert kinds == {"c0": "context", "c1": "match", "c2": "context"}


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
