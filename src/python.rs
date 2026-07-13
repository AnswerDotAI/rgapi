use grep_matcher::Matcher;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

use crate::search::spans_for;
use crate::{
    block_iter as block_iter_core, compile_regex, find, find_cancelable, nb_iter as nb_iter_core,
    nb_search_file, rg_iter as rg_iter_core, search_path as search_path_core,
    search_text as search_text_core, FindOptions, NbCell, NbIter, NbOptions, RgIter, RgOptions,
    SearchBlock, SearchLine, StreamIter,
};
use std::path::Path;

#[pyclass(name = "SearchLine", eq, skip_from_py_object)]
#[derive(Clone)]
struct SearchLinePy {
    #[pyo3(get)]
    kind: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    line_number: u64,
    #[pyo3(get)]
    lnhash: String,
    #[pyo3(get)]
    line: String,
    #[pyo3(get)]
    matches: Vec<(usize, usize)>,
    display_lnhash: bool,
}
impl PartialEq for SearchLinePy {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.path == other.path
            && self.line_number == other.line_number
            && self.lnhash == other.lnhash
            && self.line == other.line
            && self.matches == other.matches
    }
}

fn preview(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    format!("{}…", text.chars().take(width).collect::<String>())
}

#[pymethods]
impl SearchLinePy {
    fn asdict(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("kind", &self.kind)?;
        dict.set_item("path", &self.path)?;
        dict.set_item("line_number", self.line_number)?;
        dict.set_item("lnhash", &self.lnhash)?;
        dict.set_item("line", &self.line)?;
        dict.set_item("matches", self.matches.clone())?;
        Ok(dict.unbind())
    }

    fn __repr__(&self) -> String {
        format!(
            "SearchLine(kind={:?}, path={:?}, line_number={}, lnhash={:?}, line={:?}, matches={:?})",
            self.kind, self.path, self.line_number, self.lnhash, self.line, self.matches
        )
    }
    fn __str__(&self) -> String {
        let sep = if self.kind == "match" { ":" } else { "-" };
        if self.display_lnhash {
            format!(
                "{}{}{}{}",
                self.path,
                sep,
                self.lnhash,
                preview(&self.line, 120)
            )
        } else {
            format!(
                "{}{}{}{}{}",
                self.path,
                sep,
                self.line_number,
                sep,
                preview(&self.line, 120)
            )
        }
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
    display_lnhash: bool,
}
#[pymethods]
impl RgIterPy {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<SearchLinePy>> {
        let display_lnhash = slf.display_lnhash;
        Ok(next_stream_py(py, &mut slf.inner)?.map(|l| search_line_py(l, display_lnhash)))
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
fn check_signals_or_cancel<T>(py: Python<'_>, iter: &StreamIter<T>) -> PyResult<()> {
    if let Err(err) = py.check_signals() {
        iter.cancel();
        return Err(err);
    }
    Ok(())
}

fn next_stream_py<T: Send>(py: Python<'_>, iter: &mut StreamIter<T>) -> PyResult<Option<T>> {
    loop {
        check_signals_or_cancel(py, iter)?;
        let res = py.detach(|| iter.next_timeout(Duration::from_millis(50)));
        check_signals_or_cancel(py, iter)?;
        match res {
            Ok(Ok(line)) => return Ok(Some(line)),
            Ok(Err(err)) => return Err(PyValueError::new_err(err.to_string())),
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
            Err(RecvTimeoutError::Timeout) => continue,
        }
    }
}

fn collect_stream_py<T: Send, P>(
    py: Python<'_>,
    mut iter: StreamIter<T>,
    conv: impl Fn(T) -> P,
    timeout_ms: Option<u64>,
) -> PyResult<(Vec<P>, bool)> {
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    let mut res = Vec::new();
    loop {
        check_signals_or_cancel(py, &iter)?;
        let mut wait = Duration::from_millis(50);
        if let Some(d) = deadline {
            let left = d.saturating_duration_since(Instant::now());
            if left.is_zero() {
                iter.cancel();
                return Ok((res, true));
            }
            wait = wait.min(left);
        }
        let next = py.detach(|| iter.next_timeout(wait));
        check_signals_or_cancel(py, &iter)?;
        match next {
            Ok(Ok(line)) => res.push(conv(line)),
            Ok(Err(err)) => return Err(PyValueError::new_err(err.to_string())),
            Err(RecvTimeoutError::Disconnected) => return Ok((res, false)),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
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
fn find_opts(
    root: &str,
    pattern: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
) -> FindOptions {
    FindOptions {
        root: PathBuf::from(root),
        pattern,
        includes: include.unwrap_or_default(),
        excludes: exclude.unwrap_or_default(),
        exts: exts.unwrap_or_default(),
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
        panic_probe: false,
    }
}

#[pyfunction(name = "walk")]
#[pyo3(signature = (root=".", hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, files=true, dirs=false))]
fn walk_py(
    py: Python<'_>,
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
    let opts = find_opts(
        root,
        None,
        None,
        None,
        None,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        files,
        dirs,
    );
    py.detach(|| find(&opts))
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction(name = "find")]
#[pyo3(signature = (root=".", pattern=None, include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, files=true, dirs=false))]
fn find_py(
    py: Python<'_>,
    root: &str,
    pattern: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    let opts = find_opts(
        root,
        pattern,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        files,
        dirs,
    );
    py.detach(|| find(&opts))
        .map_err(|e| PyValueError::new_err(e.to_string()))
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

fn rg_opts(
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
) -> RgOptions {
    RgOptions {
        root: PathBuf::from(root),
        pattern,
        includes: include.unwrap_or_default(),
        excludes: exclude.unwrap_or_default(),
        exts: exts.unwrap_or_default(),
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
        panic_probe: false,
    }
}

#[pyfunction(name = "rg")]
#[pyo3(signature = (pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0, lnhash=false, timeout_ms=None))]
fn rg_py(
    py: Python<'_>,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    lnhash: bool,
    timeout_ms: Option<u64>,
) -> PyResult<(Vec<SearchLinePy>, bool)> {
    let opts = rg_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    );
    let iter = rg_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    collect_stream_py(py, iter, move |l| search_line_py(l, lnhash), timeout_ms)
}

#[pyfunction(name = "block_search")]
#[pyo3(signature = (pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0, timeout_ms=None))]
#[allow(clippy::too_many_arguments)]
fn block_search_py(
    py: Python<'_>,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    timeout_ms: Option<u64>,
) -> PyResult<(Vec<BlockRow>, bool)> {
    let opts = rg_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    );
    let iter = block_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    collect_stream_py(py, iter, block_row, timeout_ms)
}
#[pyfunction(name = "rg_iter")]
#[pyo3(signature = (pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0, lnhash=false))]
fn rg_iter_py(
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    lnhash: bool,
) -> PyResult<RgIterPy> {
    let opts = rg_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    );
    rg_iter_core(&opts)
        .map(|inner| RgIterPy {
            inner,
            display_lnhash: lnhash,
        })
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyclass(name = "AsyncHandle")]
struct AsyncHandlePy {
    cancel: Arc<AtomicBool>,
}

#[pymethods]
impl AsyncHandlePy {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[pyfunction(name = "find_async")]
#[pyo3(signature = (cb, root=".", pattern=None, include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, files=true, dirs=false))]
fn find_async_py(
    cb: Py<PyAny>,
    root: &str,
    pattern: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
) -> AsyncHandlePy {
    let opts = find_opts(
        root,
        pattern,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        files,
        dirs,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let res = find_cancelable(&opts, Some(&flag));
        Python::attach(|py| {
            let _ = match res {
                Ok(paths) => cb.call1(py, (paths, None::<String>)),
                Err(err) => cb.call1(py, (None::<Vec<String>>, err.to_string())),
            };
        });
    });
    AsyncHandlePy { cancel }
}

fn drain_stream<T>(
    iter: &mut StreamIter<T>,
    flag: &Arc<AtomicBool>,
    deadline: Option<Instant>,
) -> (Vec<T>, bool, Option<String>) {
    let mut rows = Vec::new();
    let (mut timed_out, mut err) = (false, None);
    loop {
        if flag.load(Ordering::Relaxed) {
            break;
        }
        let mut wait = Duration::from_millis(50);
        if let Some(d) = deadline {
            let left = d.saturating_duration_since(Instant::now());
            if left.is_zero() {
                timed_out = true;
                iter.cancel();
                break;
            }
            wait = wait.min(left);
        }
        match iter.next_timeout(wait) {
            Ok(Ok(line)) => rows.push(line),
            Ok(Err(e)) => {
                err = Some(e.to_string());
                iter.cancel();
                break;
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    (rows, timed_out, err)
}

fn stream_async<T, C>(
    cb: Py<PyAny>,
    mut iter: StreamIter<T>,
    deadline: Option<Instant>,
    conv: C,
) -> AsyncHandlePy
where
    T: Send + 'static,
    C: Fn(Python<'_>, Vec<T>, bool) -> PyResult<Py<PyAny>> + Send + 'static,
{
    let cancel = iter.cancel_flag();
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let (rows, timed_out, err) = drain_stream(&mut iter, &flag, deadline);
        drop(iter);
        Python::attach(|py| {
            let _ = match err {
                Some(e) => cb.call1(py, (py.None(), e)),
                None => match conv(py, rows, timed_out) {
                    Ok(res) => cb.call1(py, (res, None::<String>)),
                    Err(_) => cb.call1(py, (py.None(), "internal conversion error".to_string())),
                },
            };
        });
    });
    AsyncHandlePy { cancel }
}

fn stream_iter_async<T, C>(
    cb: Py<PyAny>,
    mut iter: StreamIter<T>,
    batch_max: usize,
    conv: C,
) -> AsyncHandlePy
where
    T: Send + 'static,
    C: Fn(Python<'_>, Vec<T>) -> PyResult<Py<PyAny>> + Send + 'static,
{
    let cancel = iter.cancel_flag();
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let mut err: Option<String> = None;
        loop {
            if flag.load(Ordering::Relaxed) {
                break;
            }
            let first = match iter.next_timeout(Duration::from_millis(50)) {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => {
                    err = Some(e.to_string());
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => continue,
            };
            let mut batch = vec![first];
            while batch.len() < batch_max {
                match iter.next_timeout(Duration::ZERO) {
                    Ok(Ok(line)) => batch.push(line),
                    Ok(Err(e)) => {
                        err = Some(e.to_string());
                        break;
                    }
                    Err(_) => break,
                }
            }
            let failed = Python::attach(|py| match conv(py, batch) {
                Ok(res) => cb.call1(py, (res, None::<String>)).is_err(),
                Err(_) => true,
            });
            if failed || err.is_some() {
                break;
            }
        }
        iter.cancel();
        drop(iter);
        Python::attach(|py| {
            let _ = cb.call1(py, (py.None(), err));
        });
    });
    AsyncHandlePy { cancel }
}

#[pyfunction(name = "rg_async")]
#[pyo3(signature = (cb, pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0, lnhash=false, timeout_ms=None))]
#[allow(clippy::too_many_arguments)]
fn rg_async_py(
    cb: Py<PyAny>,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    lnhash: bool,
    timeout_ms: Option<u64>,
) -> PyResult<AsyncHandlePy> {
    let opts = rg_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    );
    let iter = rg_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    Ok(stream_async(
        cb,
        iter,
        deadline,
        move |py, rows, timed_out| {
            let rows: Vec<SearchLinePy> = rows
                .into_iter()
                .map(|l| search_line_py(l, lnhash))
                .collect();
            Ok((rows, timed_out).into_pyobject(py)?.into_any().unbind())
        },
    ))
}

#[pyfunction(name = "block_search_async")]
#[pyo3(signature = (cb, pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0, timeout_ms=None))]
#[allow(clippy::too_many_arguments)]
fn block_search_async_py(
    cb: Py<PyAny>,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    timeout_ms: Option<u64>,
) -> PyResult<AsyncHandlePy> {
    let opts = rg_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    );
    let iter = block_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    Ok(stream_async(cb, iter, deadline, |py, rows, timed_out| {
        let rows: Vec<BlockRow> = rows.into_iter().map(block_row).collect();
        Ok((rows, timed_out).into_pyobject(py)?.into_any().unbind())
    }))
}

#[pyfunction(name = "rg_iter_async")]
#[pyo3(signature = (cb, batch_max, pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, before_context=0, after_context=0, lnhash=false))]
#[allow(clippy::too_many_arguments)]
fn rg_iter_async_py(
    cb: Py<PyAny>,
    batch_max: usize,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    lnhash: bool,
) -> PyResult<AsyncHandlePy> {
    let opts = rg_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        before_context,
        after_context,
    );
    let iter = rg_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(stream_iter_async(cb, iter, batch_max, move |py, rows| {
        let rows: Vec<SearchLinePy> = rows
            .into_iter()
            .map(|l| search_line_py(l, lnhash))
            .collect();
        Ok(rows.into_pyobject(py)?.into_any().unbind())
    }))
}

#[pyfunction(name = "panic_probe")]
#[pyo3(signature = (root=".", walk=false))]
fn panic_probe_py(py: Python<'_>, root: &str, walk: bool) -> PyResult<()> {
    let root = PathBuf::from(root);
    if walk {
        let opts = FindOptions {
            root,
            panic_probe: true,
            ..FindOptions::default()
        };
        find(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    } else {
        let opts = RgOptions {
            root,
            pattern: "panic_probe".to_string(),
            panic_probe: true,
            ..RgOptions::default()
        };
        let iter = rg_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
        collect_stream_py(py, iter, |l| search_line_py(l, false), None)?;
    }
    Ok(())
}

type BlockRow = (
    String,
    usize,
    u64,
    u64,
    String,
    String,
    String,
    String,
    Vec<SearchLinePy>,
);

fn block_row(block: SearchBlock) -> BlockRow {
    (
        block.path,
        block.block_index,
        block.start_line,
        block.end_line,
        block.start_lnhash,
        block.end_lnhash,
        block.kind.to_string(),
        block.source,
        block.matches.into_iter().map(SearchLinePy::from).collect(),
    )
}

type NbRow = (
    String,
    usize,
    String,
    String,
    String,
    String,
    Vec<SearchLinePy>,
);

fn nb_row(cell: NbCell) -> NbRow {
    (
        cell.path,
        cell.cell_index,
        cell.cell_id,
        cell.cell_type,
        cell.kind.to_string(),
        cell.source,
        cell.matches.into_iter().map(SearchLinePy::from).collect(),
    )
}

#[allow(clippy::too_many_arguments)]
fn nb_opts(
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    cell_context: usize,
) -> NbOptions {
    NbOptions {
        root: PathBuf::from(root),
        pattern,
        includes: include.unwrap_or_default(),
        excludes: exclude.unwrap_or_default(),
        exts: exts.unwrap_or_default(),
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
        cell_context,
    }
}

#[pyclass(name = "NbIter", unsendable)]
struct NbIterPy {
    inner: NbIter,
}
#[pymethods]
impl NbIterPy {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<NbRow>> {
        Ok(next_stream_py(py, &mut slf.inner)?.map(nb_row))
    }
    fn cancel(&self) {
        self.inner.cancel();
    }
    fn __repr__(&self) -> String {
        "NbIter(NbCell stream)".to_string()
    }
}

#[pyfunction(name = "nb_search")]
#[pyo3(signature = (pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, cell_context=0, timeout_ms=None))]
#[allow(clippy::too_many_arguments)]
fn nb_search_py(
    py: Python<'_>,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    cell_context: usize,
    timeout_ms: Option<u64>,
) -> PyResult<(Vec<NbRow>, bool)> {
    let opts = nb_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        cell_context,
    );
    let iter = nb_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    collect_stream_py(py, iter, nb_row, timeout_ms)
}

#[pyfunction(name = "nb_iter")]
#[pyo3(signature = (pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, cell_context=0))]
#[allow(clippy::too_many_arguments)]
fn nb_iter_py(
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    cell_context: usize,
) -> PyResult<NbIterPy> {
    let opts = nb_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        cell_context,
    );
    nb_iter_core(&opts)
        .map(|inner| NbIterPy { inner })
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction(name = "nb_search_async")]
#[pyo3(signature = (cb, pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, cell_context=0, timeout_ms=None))]
#[allow(clippy::too_many_arguments)]
fn nb_search_async_py(
    cb: Py<PyAny>,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    cell_context: usize,
    timeout_ms: Option<u64>,
) -> PyResult<AsyncHandlePy> {
    let opts = nb_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        cell_context,
    );
    let iter = nb_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    Ok(stream_async(cb, iter, deadline, |py, rows, timed_out| {
        let rows: Vec<NbRow> = rows.into_iter().map(nb_row).collect();
        Ok((rows, timed_out).into_pyobject(py)?.into_any().unbind())
    }))
}

#[pyfunction(name = "nb_iter_async")]
#[pyo3(signature = (cb, batch_max, pattern, root=".", include=None, exclude=None, exts=None, hidden=false, ignore=true, max_depth=None, min_depth=None, max_filesize=None, follow_links=false, same_file_system=false, path_re=None, skip_path_re=None, skip_dir=None, skip_dir_re=None, case_sensitive=None, smart_case=false, cell_context=0))]
#[allow(clippy::too_many_arguments)]
fn nb_iter_async_py(
    cb: Py<PyAny>,
    batch_max: usize,
    pattern: String,
    root: &str,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exts: Option<Vec<String>>,
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
    cell_context: usize,
) -> PyResult<AsyncHandlePy> {
    let opts = nb_opts(
        pattern,
        root,
        include,
        exclude,
        exts,
        hidden,
        ignore,
        max_depth,
        min_depth,
        max_filesize,
        follow_links,
        same_file_system,
        path_re,
        skip_path_re,
        skip_dir,
        skip_dir_re,
        case_sensitive,
        smart_case,
        cell_context,
    );
    let iter = nb_iter_core(&opts).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(stream_iter_async(cb, iter, batch_max, |py, rows| {
        let rows: Vec<NbRow> = rows.into_iter().map(nb_row).collect();
        Ok(rows.into_pyobject(py)?.into_any().unbind())
    }))
}

#[pyfunction(name = "nb_search_file")]
#[pyo3(signature = (pattern, path, display_path, case_sensitive=None, smart_case=false, cell_context=0))]
fn nb_search_file_py(
    pattern: &str,
    path: &str,
    display_path: String,
    case_sensitive: Option<bool>,
    smart_case: bool,
    cell_context: usize,
) -> PyResult<Vec<NbRow>> {
    let cells = nb_search_file(
        Path::new(path),
        display_path,
        pattern,
        case_sensitive,
        smart_case,
        cell_context,
    )
    .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(cells.into_iter().map(nb_row).collect())
}

impl From<SearchLine> for SearchLinePy {
    fn from(line: SearchLine) -> Self {
        Self {
            kind: line.kind.as_str().to_string(),
            path: line.path,
            line_number: line.line_number,
            lnhash: line.lnhash,
            line: line.line,
            matches: line.matches.into_iter().map(|m| (m.start, m.end)).collect(),
            display_lnhash: false,
        }
    }
}

fn search_line_py(line: SearchLine, display_lnhash: bool) -> SearchLinePy {
    SearchLinePy {
        display_lnhash,
        ..SearchLinePy::from(line)
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
    m.add_function(wrap_pyfunction!(block_search_py, m)?)?;
    m.add_function(wrap_pyfunction!(rg_iter_py, m)?)?;
    m.add_function(wrap_pyfunction!(search_text_py, m)?)?;
    m.add_function(wrap_pyfunction!(search_path_py, m)?)?;
    m.add_function(wrap_pyfunction!(panic_probe_py, m)?)?;
    m.add_function(wrap_pyfunction!(nb_search_py, m)?)?;
    m.add_function(wrap_pyfunction!(nb_search_file_py, m)?)?;
    m.add_class::<NbIterPy>()?;
    m.add_function(wrap_pyfunction!(nb_iter_py, m)?)?;
    m.add_function(wrap_pyfunction!(nb_search_async_py, m)?)?;
    m.add_function(wrap_pyfunction!(nb_iter_async_py, m)?)?;
    m.add_class::<AsyncHandlePy>()?;
    m.add_function(wrap_pyfunction!(find_async_py, m)?)?;
    m.add_function(wrap_pyfunction!(rg_async_py, m)?)?;
    m.add_function(wrap_pyfunction!(block_search_async_py, m)?)?;
    m.add_function(wrap_pyfunction!(rg_iter_async_py, m)?)?;
    Ok(())
}
