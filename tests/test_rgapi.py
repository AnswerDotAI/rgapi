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


def test_path_filters_prune_dirs_follow_links_and_sort(tmp_path):
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "app.py").write_text("TODO src\n")
    (tmp_path / "src" / "note.txt").write_text("TODO text\n")
    (tmp_path / "skip").mkdir()
    (tmp_path / "skip" / "app.py").write_text("TODO skip\n")
    (tmp_path / "b.py").write_text("TODO b\n")
    (tmp_path / "a.py").write_text("TODO a\n")

    assert fd(str(tmp_path), path_re=r"\.py$", sort=True) == ["a.py", "b.py", "skip/app.py", "src/app.py"]
    assert fd(str(tmp_path), path_re=r"src/.*\.py$") == ["src/app.py"]
    assert fd(str(tmp_path), path_re=r"\.py$", skip_path_re=r"(^|/)b\.py$", skip_dir="skip", sort=True) == ["a.py", "src/app.py"]
    assert walk(str(tmp_path), path_re=r"\.txt$", sort=True) == ["src/note.txt"]
    assert [r.path for r in rg("TODO", str(tmp_path), path_re=r"\.py$", skip_dir_re=r"^skip$", sort=True)] == ["a.py", "b.py", "src/app.py"]

    link = tmp_path / "linked"
    try: link.symlink_to(tmp_path / "src", target_is_directory=True)
    except OSError: return
    assert fd(str(tmp_path), path_re=r"linked/.*\.py$", follow_links=False) == []
    assert fd(str(tmp_path), path_re=r"linked/.*\.py$", follow_links=True, sort=True) == ["linked/app.py"]

def test_depth_size_and_filesystem_options(tmp_path):
    (tmp_path / "top.txt").write_text("TODO\n")
    sub = tmp_path / "sub"
    sub.mkdir()
    (sub / "small.txt").write_text("TODO\n")
    (sub / "large.txt").write_text("TODO large\n")

    assert fd(str(tmp_path), max_depth=1, sort=True) == ["top.txt"]
    assert fd(str(tmp_path), min_depth=2, sort=True) == ["sub/large.txt", "sub/small.txt"]
    assert fd(str(tmp_path), max_filesize=5, sort=True) == ["sub/small.txt", "top.txt"]
    assert fd(str(tmp_path), same_file_system=True, sort=True) == ["sub/large.txt", "sub/small.txt", "top.txt"]
    assert [r.path for r in rg("TODO", str(tmp_path), min_depth=2, max_filesize=5, sort=True)] == ["sub/small.txt"]


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


def test_direct_regex_and_search_apis(tmp_path):
    make_tree(tmp_path)
    matcher = compile("todo")
    assert isinstance(matcher, Regex)
    assert matcher.is_match("TODO")
    assert matcher.finditer("todo TODO") == [(0, 4), (5, 9)]
    assert not compile("todo", case_sensitive=True).is_match("TODO")
    assert repr(matcher) == 'Regex("todo")'
    assert str(matcher) == repr(matcher)
    assert repr(compile("todo", case_sensitive=True)) == 'Regex("todo", case_sensitive=True)'
    assert repr(compile("todo", smart_case=False)) == 'Regex("todo", smart_case=False)'
    assert compile("todo", case_sensitive=True).case_sensitive is True
    assert compile("todo", smart_case=False).smart_case is False
    text_res = search_text(matcher, "zero\nTODO here\none\n", path="memory.txt", context=1)
    assert isinstance(text_res, SearchResults)
    assert [(r.kind, r.path, r.line_number, r.line) for r in text_res] == [
        ("before", "memory.txt", 1, "zero"),
        ("match", "memory.txt", 2, "TODO here"),
        ("after", "memory.txt", 3, "one")]
    path_res = search_path(matcher, str(tmp_path / "src" / "app.py"), display_path="display.py")
    assert isinstance(path_res, SearchResults)
    assert [(r.kind, r.path, r.line_number, r.line, r.matches) for r in path_res] == [
        ("match", "display.py", 2, "TODO here", [(0, 4)])]
    assert path_res[0].asdict() == dict(kind="match", path="display.py", line_number=2, line="TODO here", matches=[(0, 4)])
