use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, Sender},
    Arc,
};
use std::time::Duration;

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{
    BinaryDetection, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkError, SinkMatch,
};
use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::walk::{configure_walker, filter_dirs, normalize_root, rel_path, PathFilters};
use crate::RgApiError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Match,
    Before,
    After,
    Context,
}
impl SearchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Before => "before",
            Self::After => "after",
            Self::Context => "context",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLine {
    pub kind: SearchKind,
    pub path: String,
    pub line_number: u64,
    pub line: String,
    pub matches: Vec<MatchSpan>,
}

#[derive(Debug, Clone)]
pub struct RgOptions {
    pub root: PathBuf,
    pub pattern: String,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
    pub path_re: Option<String>,
    pub skip_path_re: Option<String>,
    pub skip_dirs: Vec<String>,
    pub skip_dir_re: Option<String>,
    pub hidden: bool,
    pub ignore: bool,
    pub max_depth: Option<usize>,
    pub min_depth: Option<usize>,
    pub max_filesize: Option<u64>,
    pub follow_links: bool,
    pub same_file_system: bool,
    pub case_sensitive: Option<bool>,
    pub smart_case: bool,
    pub before_context: usize,
    pub after_context: usize,
}

impl Default for RgOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            pattern: String::new(),
            includes: Vec::new(),
            excludes: Vec::new(),
            path_re: None,
            skip_path_re: None,
            skip_dirs: Vec::new(),
            skip_dir_re: None,
            hidden: false,
            ignore: true,
            max_depth: None,
            min_depth: None,
            max_filesize: None,
            follow_links: false,
            same_file_system: false,
            case_sensitive: None,
            smart_case: false,
            before_context: 0,
            after_context: 0,
        }
    }
}

pub fn rg(opts: &RgOptions) -> Result<Vec<SearchLine>, RgApiError> {
    rg_iter(opts)?.collect()
}

pub fn rg_iter(opts: &RgOptions) -> Result<RgIter, RgApiError> {
    let root = normalize_root(&opts.root)?;
    let filters = Arc::new(PathFilters::new(
        &opts.includes,
        &opts.excludes,
        opts.path_re.as_deref(),
        opts.skip_path_re.as_deref(),
        &opts.skip_dirs,
        opts.skip_dir_re.as_deref(),
    )?);
    let matcher = compile_regex(&opts.pattern, opts.case_sensitive, opts.smart_case)?;
    let opts = opts.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_parallel_search(root, opts, filters, matcher, tx, worker_cancel)
    });
    Ok(RgIter {
        rx,
        cancel,
        _worker: worker,
    })
}

pub struct RgIter {
    rx: Receiver<Result<SearchLine, RgApiError>>,
    cancel: Arc<AtomicBool>,
    _worker: std::thread::JoinHandle<()>,
}

impl RgIter {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn next_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Result<SearchLine, RgApiError>, RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }
}

impl Iterator for RgIter {
    type Item = Result<SearchLine, RgApiError>;
    fn next(&mut self) -> Option<Self::Item> {
        self.rx.recv().ok()
    }
}
impl Drop for RgIter {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn run_parallel_search(
    root: PathBuf,
    opts: RgOptions,
    filters: Arc<PathFilters>,
    matcher: RegexMatcher,
    tx: Sender<Result<SearchLine, RgApiError>>,
    cancel: Arc<AtomicBool>,
) {
    let mut walker = WalkBuilder::new(&root);
    configure_walker(
        &mut walker,
        opts.ignore,
        opts.hidden,
        opts.max_depth,
        opts.min_depth,
        opts.max_filesize,
        opts.follow_links,
        opts.same_file_system,
    );
    filter_dirs(&mut walker, &root, filters.clone());
    let before_context = opts.before_context;
    let after_context = opts.after_context;
    walker.build_parallel().run(|| {
        let tx = tx.clone();
        let root = root.clone();
        let filters = filters.clone();
        let matcher = matcher.clone();
        let before_context = before_context;
        let after_context = after_context;
        let cancel = cancel.clone();
        Box::new(move |entry| {
            search_entry(
                entry,
                &root,
                &filters,
                &matcher,
                before_context,
                after_context,
                &tx,
                &cancel,
            )
        })
    });
}

fn search_entry(
    entry: Result<DirEntry, ignore::Error>,
    root: &Path,
    filters: &PathFilters,
    matcher: &RegexMatcher,
    before_context: usize,
    after_context: usize,
    tx: &Sender<Result<SearchLine, RgApiError>>,
    cancel: &Arc<AtomicBool>,
) -> WalkState {
    if is_cancelled(cancel) {
        return WalkState::Quit;
    }
    let dent = match entry {
        Ok(dent) => dent,
        Err(err) => return send_search_error(tx, RgApiError::new(err.to_string())),
    };
    let path = dent.path();
    if path == root {
        return WalkState::Continue;
    }
    let Some(ft) = dent.file_type() else {
        return WalkState::Continue;
    };
    if !ft.is_file() {
        return WalkState::Continue;
    }
    let rel = rel_path(root, path);
    if !filters.path_allowed(&rel) {
        return WalkState::Continue;
    }
    match search_path_cancelable(
        path,
        rel,
        matcher.clone(),
        before_context,
        after_context,
        Some(cancel.clone()),
    ) {
        Ok(lines) => {
            if is_cancelled(cancel) {
                return WalkState::Quit;
            }
            for line in lines {
                if is_cancelled(cancel) || tx.send(Ok(line)).is_err() {
                    return WalkState::Quit;
                }
            }
            WalkState::Continue
        }
        Err(err) => send_search_error(tx, err),
    }
}

fn is_cancelled(cancel: &Arc<AtomicBool>) -> bool {
    cancel.load(Ordering::Relaxed)
}

fn send_search_error(tx: &Sender<Result<SearchLine, RgApiError>>, err: RgApiError) -> WalkState {
    let _ = tx.send(Err(err));
    WalkState::Quit
}

pub fn compile_regex(
    pattern: &str,
    case_sensitive: Option<bool>,
    smart_case: bool,
) -> Result<RegexMatcher, RgApiError> {
    if pattern.is_empty() {
        return Err(RgApiError::new("pattern may not be empty"));
    }
    let mut builder = RegexMatcherBuilder::new();
    builder.line_terminator(Some(b'\n'));
    match case_sensitive {
        Some(true) => {
            builder.case_insensitive(false);
            builder.case_smart(false);
        }
        Some(false) => {
            builder.case_insensitive(true);
            builder.case_smart(false);
        }
        None => {
            builder.case_smart(smart_case);
        }
    }
    builder
        .build(pattern)
        .map_err(|e| RgApiError::new(e.to_string()))
}

pub fn search_path(
    path: &Path,
    display_path: String,
    matcher: RegexMatcher,
    before_context: usize,
    after_context: usize,
) -> Result<Vec<SearchLine>, RgApiError> {
    search_path_cancelable(
        path,
        display_path,
        matcher,
        before_context,
        after_context,
        None,
    )
}

fn search_path_cancelable(
    path: &Path,
    display_path: String,
    matcher: RegexMatcher,
    before_context: usize,
    after_context: usize,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Vec<SearchLine>, RgApiError> {
    let mut builder = SearcherBuilder::new();
    builder
        .line_number(true)
        .before_context(before_context)
        .after_context(after_context)
        .binary_detection(BinaryDetection::quit(0));
    let mut searcher = builder.build();
    let mut out = Vec::new();
    let search_matcher = matcher.clone();
    let sink = CollectSink {
        path: display_path,
        matcher,
        lines: &mut out,
        cancel,
    };
    match searcher.search_path(search_matcher, path, sink) {
        Ok(()) => Ok(out),
        Err(SearchError::InvalidUtf8) => Ok(Vec::new()),
        Err(err) => Err(RgApiError::new(err.to_string())),
    }
}
pub fn search_text(
    display_path: String,
    text: &str,
    matcher: RegexMatcher,
    before_context: usize,
    after_context: usize,
) -> Result<Vec<SearchLine>, RgApiError> {
    search_bytes(
        display_path,
        text.as_bytes(),
        matcher,
        before_context,
        after_context,
    )
}
fn search_bytes(
    display_path: String,
    bytes: &[u8],
    matcher: RegexMatcher,
    before_context: usize,
    after_context: usize,
) -> Result<Vec<SearchLine>, RgApiError> {
    let mut builder = SearcherBuilder::new();
    builder
        .line_number(true)
        .before_context(before_context)
        .after_context(after_context);
    let mut searcher = builder.build();
    let mut out = Vec::new();
    let search_matcher = matcher.clone();
    let sink = CollectSink {
        path: display_path,
        matcher,
        lines: &mut out,
        cancel: None,
    };
    searcher
        .search_slice(search_matcher, bytes, sink)
        .map_err(|e| RgApiError::new(e.to_string()))?;
    Ok(out)
}

struct CollectSink<'a> {
    path: String,
    matcher: RegexMatcher,
    lines: &'a mut Vec<SearchLine>,
    cancel: Option<Arc<AtomicBool>>,
}
impl CollectSink<'_> {
    fn cancelled(&self) -> bool {
        match &self.cancel {
            Some(cancel) => is_cancelled(cancel),
            None => false,
        }
    }
}

impl Sink for CollectSink<'_> {
    type Error = SearchError;

    fn matched(
        &mut self,
        _searcher: &grep_searcher::Searcher,
        mat: &SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        if self.cancelled() {
            return Ok(false);
        }
        let line = bytes_to_line(mat.bytes())?;
        let spans = spans_for(&self.matcher, mat.bytes())?;
        self.lines.push(SearchLine {
            kind: SearchKind::Match,
            path: self.path.clone(),
            line_number: mat.line_number().unwrap_or(0),
            line,
            matches: spans,
        });
        Ok(!self.cancelled())
    }

    fn context(
        &mut self,
        _searcher: &grep_searcher::Searcher,
        ctx: &SinkContext<'_>,
    ) -> Result<bool, Self::Error> {
        if self.cancelled() {
            return Ok(false);
        }
        let kind = match ctx.kind() {
            SinkContextKind::Before => SearchKind::Before,
            SinkContextKind::After => SearchKind::After,
            SinkContextKind::Other => SearchKind::Context,
        };
        self.lines.push(SearchLine {
            kind,
            path: self.path.clone(),
            line_number: ctx.line_number().unwrap_or(0),
            line: bytes_to_line(ctx.bytes())?,
            matches: Vec::new(),
        });
        Ok(!self.cancelled())
    }
    fn binary_data(
        &mut self,
        _searcher: &grep_searcher::Searcher,
        _binary_byte_offset: u64,
    ) -> Result<bool, Self::Error> {
        self.lines.clear();
        Ok(false)
    }
}

#[derive(Debug)]
pub(crate) enum SearchError {
    Message(String),
    InvalidUtf8,
}
impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => write!(f, "{msg}"),
            Self::InvalidUtf8 => write!(f, "invalid utf-8"),
        }
    }
}
impl std::error::Error for SearchError {}
impl SinkError for SearchError {
    fn error_message<T: std::fmt::Display>(message: T) -> Self {
        Self::Message(message.to_string())
    }
}
fn bytes_to_line(bytes: &[u8]) -> Result<String, SearchError> {
    let s = std::str::from_utf8(bytes).map_err(|_| SearchError::InvalidUtf8)?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}
pub(crate) fn spans_for(
    matcher: &RegexMatcher,
    bytes: &[u8],
) -> Result<Vec<MatchSpan>, SearchError> {
    let mut spans = Vec::new();
    matcher
        .find_iter(bytes, |m| {
            spans.push(MatchSpan {
                start: m.start(),
                end: m.end(),
            });
            true
        })
        .map_err(|e| SearchError::Message(e.to_string()))?;
    Ok(spans)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_rows_keep_kind_enum_and_public_strings() {
        let matcher = compile_regex("TODO", None, true).unwrap();
        let lines = search_text(
            "memory.txt".to_string(),
            "alpha\nTODO here\nomega\n",
            matcher,
            1,
            1,
        )
        .unwrap();
        let got: Vec<_> = lines
            .iter()
            .map(|line| (line.kind.as_str(), line.line_number, line.line.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("before", 1, "alpha"),
                ("match", 2, "TODO here"),
                ("after", 3, "omega")
            ]
        );
        assert_eq!(lines[1].matches, vec![MatchSpan { start: 0, end: 4 }]);
    }
    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rgapi-{}-{name}", std::process::id()))
    }
    #[test]
    fn search_path_skips_binary_files() {
        let path = temp_path("binary.dat");
        std::fs::write(&path, b"TODO before\n\0TODO after\n").unwrap();
        let matcher = compile_regex("TODO", None, true).unwrap();
        let lines = search_path(&path, "binary.dat".to_string(), matcher, 0, 0).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(lines.is_empty());
    }
    #[test]
    fn search_path_skips_invalid_utf8_files() {
        let path = temp_path("invalid.txt");
        std::fs::write(&path, b"TODO\xff\n").unwrap();
        let matcher = compile_regex("TODO", None, true).unwrap();
        let lines = search_path(&path, "invalid.txt".to_string(), matcher, 0, 0).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(lines.is_empty());
    }
}
