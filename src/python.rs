use grep_matcher::Matcher;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

use crate::search::spans_for;
use crate::{
    compile_regex, find, rg_iter as rg_iter_core, search_path as search_path_core,
    search_text as search_text_core, FindOptions, RgIter, RgOptions, SearchLine,
};

#[pyclass(name = "SearchLine", eq, skip_from_py_object)]
#[derive(Clone, PartialEq)]
struct SearchLinePy {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    line_number: u64,
    #[pyo3(get)]
    line: String,
    #[pyo3(get)]
    matches: Vec<(usize, usize)>,
}

#[pymethods]
impl SearchLinePy {
    fn asdict(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("kind", &self.kind)?;
        dict.set_item("path", &self.path)?;
        dict.set_item("line_number", self.line_number)?;
        dict.set_item("line", &self.line)?;
        dict.set_item("matches", self.matches.clone())?;
        Ok(dict.unbind())
    }

    fn __repr__(&self) -> String {
        format!(
            "SearchLine(kind={:?}, path={:?}, line_number={}, line={:?}, matches={:?})",
            self.kind, self.path, self.line_number, self.line, self.matches
        )
    }
    fn __str__(&self) -> String {
        let sep = if self.kind == "match" { ":" } else { "-" };
        format!(
            "{}{}{}{}{}",
            self.path, sep, self.line_number, sep, self.line
        )
    }
    fn _repr_pretty_(&self, p: &Bound<'_, PyAny>, cycle: bool) -> PyResult<()> {
        let text = if cycle {
            "...".to_string()
        } else {
            self.__str__()
        };
        p.call_method1("text", (text,))?;
        Ok(())
    }
}
#[pyclass(name = "RgIter", unsendable)]
struct RgIterPy {
    inner: RgIter,
}
#[pymethods]
impl RgIterPy {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<SearchLinePy>> {
        next_rg_line_py(py, &mut slf.inner)
    }
    fn cancel(&self) {
        self.inner.cancel();
    }
    fn __repr__(&self) -> String {
        "RgIter(SearchLine stream)".to_string()
    }
    fn __str__(&self) -> String {
        self.__repr__()
    }
    fn _repr_pretty_(&self, p: &Bound<'_, PyAny>, cycle: bool) -> PyResult<()> {
        let text = if cycle {
            "...".to_string()
        } else {
            self.__str__()
        };
        p.call_method1("text", (text,))?;
        Ok(())
    }
}
fn check_signals_or_cancel(py: Python<'_>, iter: &RgIter) -> PyResult<()> {
    if let Err(err) = py.check_signals() {
        iter.cancel();
        return Err(err);
    }
    Ok(())
}

fn next_rg_line_py(py: Python<'_>, iter: &mut RgIter) -> PyResult<Option<SearchLinePy>> {
    loop {
        check_signals_or_cancel(py, iter)?;
        let res = py.detach(|| iter.next_timeout(Duration::from_millis(50)));
        check_signals_or_cancel(py, iter)?;
        match res {
            Ok(Ok(line)) => return Ok(Some(SearchLinePy::from(line))),
            Ok(Err(err)) => return Err(PyValueError::new_err(err.to_string())),
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
            Err(RecvTimeoutError::Timeout) => continue,
        }
    }
}

fn collect_rg_py(py: Python<'_>, mut iter: RgIter) -> PyResult<Vec<SearchLinePy>> {
    let mut res = Vec::new();
    while let Some(line) = next_rg_line_py(py, &mut iter)? {
        res.push(line);
    }
    Ok(res)
}
#[pyclass(name = "Regex", skip_from_py_object)]
#[derive(Clone)]
struct RegexPy {
    #[pyo3(get)]
    pattern: String,
    #[pyo3(get)]
    case_sensitive: Option<bool>,
    #[pyo3(get)]
    smart_case: bool,
    matcher: grep_regex::RegexMatcher,
}
#[pymethods]
impl RegexPy {
    #[new]
    #[pyo3(signature = (pattern, case_sensitive=None, smart_case=false))]
    fn new(pattern: String, case_sensitive: Option<bool>, smart_case: bool) -> PyResult<Self> {
        compile_regex_py(pattern, case_sensitive, smart_case)
    }
    fn is_match(&self, text: &str) -> PyResult<bool> {
        self.matcher
            .is_match(text.as_bytes())
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }
    fn finditer(&self, text: &str) -> PyResult<Vec<(usize, usize)>> {
        spans_for(&self.matcher, text.as_bytes())
            .map(|spans| spans.into_iter().map(|m| (m.start, m.end)).collect())
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }
    fn __repr__(&self) -> String {
        let mut args = vec![format!("{:?}", self.pattern)];
        if let Some(case_sensitive) = self.case_sensitive {
            let value = if case_sensitive { "True" } else { "False" };
            args.push(format!("case_sensitive={value}"));
        }
        if self.smart_case {
            args.push("smart_case=True".to_string());
        }
        format!("Regex({})", args.join(", "))
    }
    fn __str__(&self) -> String {
        self.__repr__()
    }
    fn _repr_pretty_(&self, p: &Bound<'_, PyAny>, cycle: bool) -> PyResult<()> {
        let text = if cycle {
            "...".to_string()
        } else {
            self.__str__()
        };
        p.call_method1("text", (text,))?;
        Ok(())
    }
}
fn compile_regex_py(
    pattern: String,
    case_sensitive: Option<bool>,
    smart_case: bool,
) -> PyResult<RegexPy> {
    let matcher = compile_regex(&pattern, case_sensitive, smart_case)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(RegexPy {
        pattern,
        case_sensitive,
        smart_case,
        matcher,
    })
}
#[pyfunction(name = "compile")]
#[pyo3(signature = (pattern, case_sensitive=None, smart_case=false))]
fn compile_py(
    pattern: String,
    case_sensitive: Option<bool>,
    smart_case: bool,
) -> PyResult<RegexPy> {
    compile_regex_py(pattern, case_sensitive, smart_case)
}
#[pyfunction(name = "walk")]
#[pyo3(signature = (root=".", hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, files=true, dirs=false))]
fn walk_py(
    root: &str,
    hidden: bool,
    ignore: bool,
    max_depth: Option<usize>,
    min_depth: Option<usize>,
    max_filesize: Option<u64>,
    follow_links: bool,
    same_file_system: bool,
    path_re: Option<String>,
    skip_path_re: Option<String>,
    skip_dir: Option<Vec<String>>,
    skip_dir_re: Option<String>,
    files: bool,
    dirs: bool,
) -> PyResult<Vec<String>> {
    let opts = FindOptions {
        root: PathBuf::from(root),
        pattern: None,
        includes: Vec::new(),
        excludes: Vec::new(),
        path_re,
        skip_path_re,
        skip_dirs: skip_dir.unwrap_or_default(),
        skip_dir_re,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        files,
        dirs,
    };
    find(&opts).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction(name = "find")]
#[pyo3(signature = (root=".", pattern=None, include=None, exclude=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, files=true, dirs=false))]
fn find_py(
    root: &str,
    pattern: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    hidden: bool,
    ignore: bool,
    max_depth: Option<usize>,
    min_depth: Option<usize>,
    max_filesize: Option<u64>,
    follow_links: bool,
    same_file_system: bool,
    path_re: Option<String>,
    skip_path_re: Option<String>,
    skip_dir: Option<Vec<String>>,
    skip_dir_re: Option<String>,
    files: bool,
    dirs: bool,
) -> PyResult<Vec<String>> {
    let opts = FindOptions {
        root: PathBuf::from(root),
        pattern,
        includes: include.unwrap_or_default(),
        excludes: exclude.unwrap_or_default(),
        path_re,
        skip_path_re,
        skip_dirs: skip_dir.unwrap_or_default(),
        skip_dir_re,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        files,
        dirs,
    };
    find(&opts).map_err(|e| PyValueError::new_err(e.to_string()))
}
#[pyfunction(name = "search_text")]
#[pyo3(signature = (matcher, text, path="<text>", before_context=0, after_context=0))]
fn search_text_py(
    matcher: PyRef<'_, RegexPy>,
    text: &str,
    path: &str,
    before_context: usize,
    after_context: usize,
) -> PyResult<Vec<SearchLinePy>> {
    search_text_core(
        path.to_string(),
        text,
        matcher.matcher.clone(),
        before_context,
        after_context,
    )
    .map(|lines| lines.into_iter().map(SearchLinePy::from).collect())
    .map_err(|e| PyValueError::new_err(e.to_string()))
}
#[pyfunction(name = "search_path")]
#[pyo3(signature = (matcher, path, display_path=None, before_context=0, after_context=0))]
fn search_path_py(
    matcher: PyRef<'_, RegexPy>,
    path: &str,
    display_path: Option<String>,
    before_context: usize,
    after_context: usize,
) -> PyResult<Vec<SearchLinePy>> {
    let path_buf = PathBuf::from(path);
    let display = display_path.unwrap_or_else(|| path_buf.to_string_lossy().replace('\\', "/"));
    search_path_core(
        &path_buf,
        display,
        matcher.matcher.clone(),
        before_context,
        after_context,
    )
    .map(|lines| lines.into_iter().map(SearchLinePy::from).collect())
    .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction(name = "rg")]
#[pyo3(signature = (pattern, root=".", include=None, exclude=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0))]
fn rg_py(
    py: Python<'_>,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    hidden: bool,
    ignore: bool,
    max_depth: Option<usize>,
    min_depth: Option<usize>,
    max_filesize: Option<u64>,
    follow_links: bool,
    same_file_system: bool,
    path_re: Option<String>,
    skip_path_re: Option<String>,
    skip_dir: Option<Vec<String>>,
    skip_dir_re: Option<String>,
    case_sensitive: Option<bool>,
    smart_case: bool,
    before_context: usize,
    after_context: usize,
) -> PyResult<Vec<SearchLinePy>> {
    let opts = RgOptions {
        root: PathBuf::from(root),
        pattern,
        includes: include.unwrap_or_default(),
        excludes: exclude.unwrap_or_default(),
        path_re,
        skip_path_re,
        skip_dirs: skip_dir.unwrap_or_default(),
        skip_dir_re,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    };
    let iter = rg_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    collect_rg_py(py, iter)
}
#[pyfunction(name = "rg_iter")]
#[pyo3(signature = (pattern, root=".", include=None, exclude=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0))]
fn rg_iter_py(
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    hidden: bool,
    ignore: bool,
    max_depth: Option<usize>,
    min_depth: Option<usize>,
    max_filesize: Option<u64>,
    follow_links: bool,
    same_file_system: bool,
    path_re: Option<String>,
    skip_path_re: Option<String>,
    skip_dir: Option<Vec<String>>,
    skip_dir_re: Option<String>,
    case_sensitive: Option<bool>,
    smart_case: bool,
    before_context: usize,
    after_context: usize,
) -> PyResult<RgIterPy> {
    let opts = RgOptions {
        root: PathBuf::from(root),
        pattern,
        includes: include.unwrap_or_default(),
        excludes: exclude.unwrap_or_default(),
        path_re,
        skip_path_re,
        skip_dirs: skip_dir.unwrap_or_default(),
        skip_dir_re,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    };
    rg_iter_core(&opts)
        .map(|inner| RgIterPy { inner })
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

impl From<SearchLine> for SearchLinePy {
    fn from(line: SearchLine) -> Self {
        Self {
            kind: line.kind.as_str().to_string(),
            path: line.path,
            line_number: line.line_number,
            line: line.line,
            matches: line.matches.into_iter().map(|m| (m.start, m.end)).collect(),
        }
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SearchLinePy>()?;
    m.add_class::<RgIterPy>()?;
    m.add_class::<RegexPy>()?;
    m.add_function(wrap_pyfunction!(compile_py, m)?)?;
    m.add_function(wrap_pyfunction!(walk_py, m)?)?;
    m.add_function(wrap_pyfunction!(find_py, m)?)?;
    m.add_function(wrap_pyfunction!(rg_py, m)?)?;
    m.add_function(wrap_pyfunction!(rg_iter_py, m)?)?;
    m.add_function(wrap_pyfunction!(search_text_py, m)?)?;
    m.add_function(wrap_pyfunction!(search_path_py, m)?)?;
    Ok(())
}
