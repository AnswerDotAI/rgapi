use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
};

use grep_matcher::{LineTerminator, Matcher};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkError, SinkMatch};
use ignore::{DirEntry, WalkState};

use crate::RgApiError;
use crate::walk::{PathFilters, StreamIter, WalkOptions, entry_err, rel_path, resolve_roots, spawn_walk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSpan { pub start: usize, pub end: usize }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Match,
    Before,
    After,
    Context,
}
impl SearchKind {
    pub fn as_str(self) -> &'static str { match self { Self::Match => "match", Self::Before => "before", Self::After => "after", Self::Context => "context" } }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLine {
    pub kind: SearchKind,
    pub path: String,
    pub line_number: u64,
    pub lnhash: String,
    pub line: String,
    pub matches: Vec<MatchSpan>,
}

#[derive(Debug, Clone, Default)]
pub struct RgOptions {
    pub walk: WalkOptions,
    pub pattern: String,
    pub case_sensitive: Option<bool>,
    pub smart_case: bool,
    pub before_context: usize,
    pub after_context: usize,
    pub panic_probe: bool,
}

pub fn rg(opts: &RgOptions) -> Result<Vec<SearchLine>, RgApiError> { rg_iter(opts)?.collect() }

pub fn rg_iter(opts: &RgOptions) -> Result<RgIter, RgApiError> {
    let (roots, base) = resolve_roots(&opts.walk, true, false)?;
    let filters = Arc::new(PathFilters::new(&opts.walk)?);
    let matcher = compile_regex(&opts.pattern, opts.case_sensitive, opts.smart_case, false)?;
    let (before_context, after_context, panic_probe, max_depth) = (opts.before_context, opts.after_context, opts.panic_probe, opts.walk.max_depth);
    Ok(spawn_walk(roots, base, &opts.walk, filters, Vec::new(), move |dent, base, filters, tx, cancel| {
        if panic_probe { panic!("rgapi: deliberate panic for tests (panic_probe)"); }
        search_entry(dent, base, filters, &matcher, before_context, after_context, max_depth, tx, cancel)
    }))
}

pub type RgIter = StreamIter<SearchLine>;

fn search_entry(
    entry: Result<DirEntry, ignore::Error>,
    base: &Path,
    filters: &PathFilters,
    matcher: &RegexMatcher,
    before_context: usize,
    after_context: usize,
    max_depth: Option<usize>,
    tx: &SyncSender<Result<SearchLine, RgApiError>>,
    cancel: &Arc<AtomicBool>,
) -> WalkState {
    if is_cancelled(cancel) { return WalkState::Quit; }
    let dent = match entry {
        Ok(dent) => dent,
        Err(err) => {
            return match entry_err(err, max_depth) { Some(e) => send_search_error(tx, e), None => WalkState::Continue };
        }
    };
    let path = dent.path();
    let Some(ft) = dent.file_type() else { return WalkState::Continue; };
    if !ft.is_file() { return WalkState::Continue; }
    let rel = rel_path(base, path);
    if !filters.path_allowed(Path::new(&rel)) { return WalkState::Continue; }
    match search_path_cancelable(path, rel, matcher.clone(), before_context, after_context, Some(cancel.clone())) {
        Ok(lines) => {
            if is_cancelled(cancel) { return WalkState::Quit; }
            for line in lines { if is_cancelled(cancel) || tx.send(Ok(line)).is_err() { return WalkState::Quit; } }
            WalkState::Continue
        }
        Err(err) => send_search_error(tx, err),
    }
}

fn is_cancelled(cancel: &Arc<AtomicBool>) -> bool { cancel.load(Ordering::Relaxed) }

fn send_search_error(tx: &SyncSender<Result<SearchLine, RgApiError>>, err: RgApiError) -> WalkState {
    let _ = tx.send(Err(err));
    WalkState::Quit
}

pub fn compile_regex(pattern: &str, case_sensitive: Option<bool>, smart_case: bool, multiline: bool) -> Result<RegexMatcher, RgApiError> {
    if pattern.is_empty() { return Err(RgApiError::new("pattern may not be empty")); }
    let mut builder = RegexMatcherBuilder::new();
    builder.multi_line(true).crlf(true);
    if multiline { builder.line_terminator(None); }
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
    builder.build(pattern).map_err(|e| RgApiError::new(e.to_string()))
}

pub(crate) fn format_lnhash(lineno: u64, line: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let hash = crc32fast::hash(line.as_bytes()) as usize;
    format!("{}|{}{}|", lineno, ALPHABET[(hash >> 6) & 63] as char, ALPHABET[hash & 63] as char)
}

pub fn search_path(
    path: &Path,
    display_path: String,
    matcher: RegexMatcher,
    before_context: usize,
    after_context: usize,
) -> Result<Vec<SearchLine>, RgApiError> { search_path_cancelable(path, display_path, matcher, before_context, after_context, None) }

fn search_path_cancelable(
    path: &Path,
    display_path: String,
    matcher: RegexMatcher,
    before_context: usize,
    after_context: usize,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Vec<SearchLine>, RgApiError> {
    let mut builder = SearcherBuilder::new();
    builder.line_number(true).line_terminator(LineTerminator::crlf()).before_context(before_context).after_context(after_context).binary_detection(BinaryDetection::quit(0));
    let mut searcher = builder.build();
    let mut out = Vec::new();
    let search_matcher = matcher.clone();
    let sink = CollectSink { path: display_path, matcher, lines: &mut out, cancel };
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
    multiline: bool,
) -> Result<Vec<SearchLine>, RgApiError> { search_bytes(display_path, text.as_bytes(), matcher, before_context, after_context, multiline) }
fn search_bytes(
    display_path: String,
    bytes: &[u8],
    matcher: RegexMatcher,
    before_context: usize,
    after_context: usize,
    multiline: bool,
) -> Result<Vec<SearchLine>, RgApiError> {
    let mut builder = SearcherBuilder::new();
    builder.line_number(true).line_terminator(LineTerminator::crlf()).before_context(before_context).after_context(after_context).multi_line(multiline);
    let mut searcher = builder.build();
    let mut out = Vec::new();
    let search_matcher = matcher.clone();
    let sink = CollectSink { path: display_path, matcher, lines: &mut out, cancel: None };
    searcher.search_slice(search_matcher, bytes, sink).map_err(|e| RgApiError::new(e.to_string()))?;
    Ok(out)
}

struct CollectSink<'a> {
    path: String,
    matcher: RegexMatcher,
    lines: &'a mut Vec<SearchLine>,
    cancel: Option<Arc<AtomicBool>>,
}
impl CollectSink<'_> { fn cancelled(&self) -> bool { match &self.cancel { Some(cancel) => is_cancelled(cancel), None => false } } }

impl Sink for CollectSink<'_> {
    type Error = SearchError;

    fn matched(&mut self, _searcher: &grep_searcher::Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.cancelled() { return Ok(false); }
        let line = bytes_to_line(mat.bytes())?;
        let line_number = mat.line_number().unwrap_or(0);
        let spans = spans_for(&self.matcher, mat.bytes())?;
        self.lines.push(SearchLine {
            kind: SearchKind::Match,
            path: self.path.clone(),
            line_number,
            lnhash: format_lnhash(line_number, &line),
            line,
            matches: spans,
        });
        Ok(!self.cancelled())
    }

    fn context(&mut self, _searcher: &grep_searcher::Searcher, ctx: &SinkContext<'_>) -> Result<bool, Self::Error> {
        if self.cancelled() { return Ok(false); }
        let kind = match ctx.kind() {
            SinkContextKind::Before => SearchKind::Before,
            SinkContextKind::After => SearchKind::After,
            SinkContextKind::Other => SearchKind::Context,
        };
        let line = bytes_to_line(ctx.bytes())?;
        let line_number = ctx.line_number().unwrap_or(0);
        self.lines.push(SearchLine { kind, path: self.path.clone(), line_number, lnhash: format_lnhash(line_number, &line), line, matches: Vec::new() });
        Ok(!self.cancelled())
    }
    fn binary_data(&mut self, _searcher: &grep_searcher::Searcher, _binary_byte_offset: u64) -> Result<bool, Self::Error> {
        self.lines.clear();
        Ok(false)
    }
}

#[derive(Debug)]
pub(crate) enum SearchError { Message(String), InvalidUtf8 }
impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Message(msg) => write!(f, "{msg}"), Self::InvalidUtf8 => write!(f, "invalid utf-8") }
    }
}
impl std::error::Error for SearchError {}
impl SinkError for SearchError { fn error_message<T: std::fmt::Display>(message: T) -> Self { Self::Message(message.to_string()) } }
fn bytes_to_line(bytes: &[u8]) -> Result<String, SearchError> {
    let s = std::str::from_utf8(bytes).map_err(|_| SearchError::InvalidUtf8)?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}
pub(crate) fn spans_for(matcher: &RegexMatcher, bytes: &[u8]) -> Result<Vec<MatchSpan>, SearchError> {
    let mut spans = Vec::new();
    matcher
        .find_iter(bytes, |m| {
            spans.push(MatchSpan { start: m.start(), end: m.end() });
            true
        })
        .map_err(|e| SearchError::Message(e.to_string()))?;
    Ok(spans)
}
