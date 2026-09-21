use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::sync::atomic::Ordering;

use grep_matcher::{Captures, Matcher};
use grep_regex::RegexMatcher;
use ignore::{DirEntry, WalkState};
use serde::Deserialize;

use crate::RgApiError;
use crate::search::{SearchLine, compile_regex, search_text};
use crate::walk::{PathFilters, StreamIter, entry_err, file_root_flags, normalize_root, rel_path, spawn_walk};

static HEADING_RE: LazyLock<RegexMatcher> = LazyLock::new(|| RegexMatcher::new(r"^#{1,6} \w").unwrap());

/// Heading level of the first nonblank line after skipping `#|` directives; zero unless it matches `^#{1,6} \w`.
pub fn heading_level(source: &str) -> usize {
    let line = source.lines().find(|l| !l.trim().is_empty() && !l.starts_with("#|")).unwrap_or("");
    if !HEADING_RE.is_match(line.as_bytes()).unwrap_or(false) { return 0; }
    line.bytes().take_while(|&b| b == b'#').count()
}

/// A heading and its descendants; a non-heading selects itself. Zero levels represent non-heading cells.
pub fn section_range(levels: &[usize], idx: usize) -> std::ops::Range<usize> {
    if levels[idx] == 0 { return idx..idx + 1; }
    let end = (idx + 1..levels.len()).find(|&i| levels[i] > 0 && levels[i] <= levels[idx]).unwrap_or(levels.len());
    idx..end
}

/// Enclosing heading indices, outermost first, excluding the addressed cell.
pub fn ancestor_indices(levels: &[usize], idx: usize) -> Vec<usize> {
    let mut level = if levels[idx] == 0 { 7 } else { levels[idx] };
    let mut parents = Vec::new();
    for i in (0..idx).rev() {
        if levels[i] > 0 && levels[i] < level {
            parents.push(i);
            level = levels[i];
        }
    }
    parents.reverse();
    parents
}

/// Group 1 of every `` sigil`body` `` match in `text`, in order of appearance. `sigil` and `body` are regex fragments.
fn sigil_caps(text: &str, sigil: &str, body: &str) -> Vec<String> {
    let re = RegexMatcher::new(&format!("{sigil}`({body})`")).expect("sigil pattern compiles");
    let mut caps = re.new_captures().expect("captures allocate");
    let mut res = Vec::new();
    re.captures_iter(text.as_bytes(), &mut caps, |c| {
        if let Some(m) = c.get(1) { res.push(text[m.start()..m.end()].to_string()); }
        true
    }).expect("regex search is infallible");
    res
}

/// Names written as `` &`name` `` or `` &`[a, b]` `` in `text`. A name holds word characters and dots.
fn tool_names(text: &str) -> Vec<String> {
    let groups = sigil_caps(text, "&", r"[\w.]+|\[[\w.,\s]+\]");
    groups.iter().flat_map(|g| g.split(['[', ']', ','])).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// nbformat multiline text, a string or a list of strings, as one string.
fn nb_text(v: &serde_json::Value) -> String {
    match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Array(a) => a.iter().filter_map(|o| o.as_str()).collect(), _ => String::new() }
}

/// The sigil references in one notebook cell, in order of appearance.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CellRefs {
    /// Each `expr` written as `` $`expr` ``
    pub vars: Vec<String>,
    /// Each `cmd` written as `` !`cmd` ``
    pub cmds: Vec<String>,
    /// Each name written as `` &`name` `` or `` &`[a, b]` ``
    pub tools: Vec<String>,
}

/// The sigil references in `cell`, an nbformat cell. A prompt cell has `solveit_ai: true` in its metadata.
/// `vars` and `cmds` come from a prompt cell's source. `tools` comes from the source of a prompt or Markdown cell.
/// For every other cell, `tools` comes from the `text/markdown` data of its `display_data` and `execute_result` outputs.
/// A prompt cell's outputs are never read.
pub fn cell_refs(cell: &serde_json::Value) -> CellRefs {
    let src = nb_text(&cell["source"]);
    let prompt = cell["metadata"]["solveit_ai"] == true;
    let mut res = CellRefs::default();
    if prompt { (res.vars, res.cmds) = (sigil_caps(&src, r"\$", "[^`]+"), sigil_caps(&src, "!", "[^`]+")); }
    if prompt || cell["cell_type"] == "markdown" { res.tools = tool_names(&src); return res; }
    for o in cell["outputs"].as_array().into_iter().flatten() {
        if matches!(o["output_type"].as_str(), Some("display_data" | "execute_result")) { res.tools.extend(tool_names(&nb_text(&o["data"]["text/markdown"]))); }
    }
    res
}

#[derive(Debug, Clone)]
pub struct NbOptions {
    pub root: PathBuf,
    pub pattern: String,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
    pub exts: Vec<String>,
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
    pub cell_context: usize,
    pub multiline: bool,
}

/// One emitted cell (a match, or context for a match).
pub struct NbCell {
    pub path: String,
    pub cell_index: usize,
    pub cell_id: String,
    pub cell_type: String,
    pub kind: &'static str, // "match" | "context"
    pub source: String,
    pub matches: Vec<SearchLine>,
}

// Lean notebook model: only the fields we search; outputs/metadata are skipped by serde
// without being allocated, which is the whole memory win over materializing the JSON in Python.
#[derive(Deserialize)]
struct RawNb { #[serde(default)] cells: Vec<RawCell> }

#[derive(Deserialize)]
struct RawCell {
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(default)]
    cell_type: Option<String>,
    #[serde(default)]
    source: Source,
}

impl RawCell {
    fn id_string(&self, index: usize) -> String {
        match &self.id { Some(serde_json::Value::String(s)) => s.clone(), Some(v) => v.to_string(), None => index.to_string() }
    }
}

// nbformat `source` is a list of lines or a single string (or absent/null).
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum Source {
    Lines(Vec<String>),
    Text(String),
    #[default]
    Empty,
}

impl Source { fn text(&self) -> String { match self { Source::Lines(v) => v.concat(), Source::Text(s) => s.clone(), Source::Empty => String::new() } } }

fn process_file(disp: String, bytes: &[u8], matcher: &RegexMatcher, cell_context: usize, multiline: bool) -> Result<Vec<NbCell>, RgApiError> {
    // Not a parseable notebook (bad JSON, or JSON that isn't a notebook): skip, like a binary file.
    let nb: RawNb = match serde_json::from_slice(bytes) { Ok(nb) => nb, Err(_) => return Ok(Vec::new()) };
    let n = nb.cells.len();
    let mut info = Vec::with_capacity(n);
    let mut matched: Vec<(usize, Vec<SearchLine>)> = Vec::new();
    for (i, cell) in nb.cells.iter().enumerate() {
        let src = cell.source.text();
        let hits = search_text(disp.clone(), &src, matcher.clone(), 0, 0, multiline)?;
        if !hits.is_empty() { matched.push((i, hits)); }
        info.push((cell.id_string(i), cell.cell_type.clone().unwrap_or_default(), src));
    }
    if matched.is_empty() { return Ok(Vec::new()); }
    let mut emit: BTreeMap<usize, bool> = BTreeMap::new(); // index -> is_match
    for (i, _) in &matched { emit.insert(*i, true); }
    if cell_context > 0 {
        for (i, _) in &matched { for j in i.saturating_sub(cell_context)..(i + cell_context + 1).min(n) { emit.entry(j).or_insert(false); } }
    }
    let mut matched: HashMap<usize, Vec<SearchLine>> = matched.into_iter().collect();
    let mut out = Vec::with_capacity(emit.len());
    for (i, is_match) in emit {
        let (cid, ctype, src) = &info[i];
        let (kind, matches) = if is_match { ("match", matched.remove(&i).unwrap_or_default()) } else { ("context", Vec::new()) };
        out.push(NbCell { path: disp.clone(), cell_index: i, cell_id: cid.clone(), cell_type: ctype.clone(), kind, source: src.clone(), matches });
    }
    Ok(out)
}

fn compile_nb_regex(pattern: &str, case_sensitive: Option<bool>, smart_case: bool, multiline: bool) -> Result<RegexMatcher, RgApiError> {
    compile_regex(pattern, case_sensitive, smart_case, multiline).map_err(|e| {
        if !multiline && e.to_string().contains("not allowed in a regex") {
            RgApiError::new(format!("{e}; pass multiline=True to let the pattern match across lines within a cell"))
        } else { e }
    })
}

pub fn nb_search_file(
    path: &Path,
    display_path: String,
    pattern: &str,
    case_sensitive: Option<bool>,
    smart_case: bool,
    cell_context: usize,
    multiline: bool,
) -> Result<Vec<NbCell>, RgApiError> {
    let matcher = compile_nb_regex(pattern, case_sensitive, smart_case, multiline)?;
    let bytes = match std::fs::read(path) { Ok(b) => b, Err(_) => return Ok(Vec::new()) };
    process_file(display_path, &bytes, &matcher, cell_context, multiline)
}

fn nb_entry(
    entry: Result<DirEntry, ignore::Error>,
    root: &Path,
    filters: &PathFilters,
    matcher: &RegexMatcher,
    cell_context: usize,
    multiline: bool,
    max_depth: Option<usize>,
) -> Result<Vec<NbCell>, RgApiError> {
    let dent = match entry { Ok(dent) => dent, Err(err) => return entry_err(err, max_depth).map_or(Ok(Vec::new()), Err) };
    let path = dent.path();
    let Some(ft) = dent.file_type() else { return Ok(Vec::new()); };
    if !ft.is_file() { return Ok(Vec::new()); }
    let rel = rel_path(root, path);
    if !filters.path_allowed(Path::new(&rel)) { return Ok(Vec::new()); }
    let bytes = match std::fs::read(path) { Ok(b) => b, Err(_) => return Ok(Vec::new()) };
    process_file(rel, &bytes, matcher, cell_context, multiline)
}

pub type NbIter = StreamIter<NbCell>;

pub fn nb_iter(opts: &NbOptions) -> Result<NbIter, RgApiError> {
    let (ignore, hidden) = file_root_flags(&opts.root, opts.ignore, opts.hidden);
    let root = normalize_root(&opts.root)?;
    let filters = Arc::new(PathFilters::new(
        &opts.includes,
        &opts.excludes,
        &opts.exts,
        opts.path_re.as_deref(),
        opts.skip_path_re.as_deref(),
        &opts.skip_dirs,
        opts.skip_dir_re.as_deref(),
    )?);
    let matcher = compile_nb_regex(&opts.pattern, opts.case_sensitive, opts.smart_case, opts.multiline)?;
    let cell_context = opts.cell_context;
    let multiline = opts.multiline;
    let max_depth = opts.max_depth;
    Ok(spawn_walk(
        root,
        ignore,
        hidden,
        opts.max_depth,
        opts.min_depth,
        opts.max_filesize,
        opts.follow_links,
        opts.same_file_system,
        filters,
        move |dent, root, filters, tx, cancel| match nb_entry(dent, root, filters, &matcher, cell_context, multiline, max_depth) {
            Ok(cells) => {
                for cell in cells { if cancel.load(Ordering::Relaxed) || tx.send(Ok(cell)).is_err() { return WalkState::Quit; } }
                WalkState::Continue
            }
            Err(err) => {
                let _ = tx.send(Err(err));
                WalkState::Quit
            }
        },
    ))
}

pub fn nb_search(opts: &NbOptions) -> Result<Vec<NbCell>, RgApiError> { nb_iter(opts)?.collect() }
