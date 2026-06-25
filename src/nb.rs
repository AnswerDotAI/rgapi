use std::collections::{BTreeMap, HashMap};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

use grep_regex::RegexMatcher;
use ignore::{DirEntry, WalkBuilder, WalkState};
use serde::Deserialize;

use crate::search::{compile_regex, search_text, SearchLine};
use crate::walk::{configure_walker, filter_dirs, normalize_root, rel_path, resolve_root, PathFilters};
use crate::RgApiError;

#[derive(Debug, Clone)]
pub struct NbOptions {
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
    pub cell_context: usize,
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
struct RawNb {
    #[serde(default)]
    cells: Vec<RawCell>,
}

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
        match &self.id {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => index.to_string(),
        }
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

impl Source {
    fn text(&self) -> String {
        match self {
            Source::Lines(v) => v.concat(),
            Source::Text(s) => s.clone(),
            Source::Empty => String::new(),
        }
    }
}

fn process_file(
    disp: String,
    bytes: &[u8],
    matcher: &RegexMatcher,
    cell_context: usize,
) -> Result<Vec<NbCell>, RgApiError> {
    // Not a parseable notebook (bad JSON, or JSON that isn't a notebook): skip, like a binary file.
    let nb: RawNb = match serde_json::from_slice(bytes) {
        Ok(nb) => nb,
        Err(_) => return Ok(Vec::new()),
    };
    let n = nb.cells.len();
    let mut info = Vec::with_capacity(n);
    let mut matched: Vec<(usize, Vec<SearchLine>)> = Vec::new();
    for (i, cell) in nb.cells.iter().enumerate() {
        let src = cell.source.text();
        let hits = search_text(disp.clone(), &src, matcher.clone(), 0, 0)?;
        if !hits.is_empty() {
            matched.push((i, hits));
        }
        info.push((cell.id_string(i), cell.cell_type.clone().unwrap_or_default(), src));
    }
    if matched.is_empty() {
        return Ok(Vec::new());
    }
    let mut emit: BTreeMap<usize, bool> = BTreeMap::new(); // index -> is_match
    for (i, _) in &matched {
        emit.insert(*i, true);
    }
    if cell_context > 0 {
        for (i, _) in &matched {
            for j in i.saturating_sub(cell_context)..(i + cell_context + 1).min(n) {
                emit.entry(j).or_insert(false);
            }
        }
    }
    let mut matched: HashMap<usize, Vec<SearchLine>> = matched.into_iter().collect();
    let mut out = Vec::with_capacity(emit.len());
    for (i, is_match) in emit {
        let (cid, ctype, src) = &info[i];
        let (kind, matches) = if is_match {
            ("match", matched.remove(&i).unwrap_or_default())
        } else {
            ("context", Vec::new())
        };
        out.push(NbCell {
            path: disp.clone(),
            cell_index: i,
            cell_id: cid.clone(),
            cell_type: ctype.clone(),
            kind,
            source: src.clone(),
            matches,
        });
    }
    Ok(out)
}

pub fn nb_search_file(
    path: &Path,
    display_path: String,
    pattern: &str,
    case_sensitive: Option<bool>,
    smart_case: bool,
    cell_context: usize,
) -> Result<Vec<NbCell>, RgApiError> {
    let matcher = compile_regex(pattern, case_sensitive, smart_case)?;
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Ok(Vec::new()),
    };
    process_file(display_path, &bytes, &matcher, cell_context)
}

fn nb_entry(
    entry: Result<DirEntry, ignore::Error>,
    root: &Path,
    filters: &PathFilters,
    matcher: &RegexMatcher,
    cell_context: usize,
) -> Result<Vec<NbCell>, RgApiError> {
    let dent = entry.map_err(|e| RgApiError::new(e.to_string()))?;
    let path = dent.path();
    if path == root {
        return Ok(Vec::new());
    }
    let Some(ft) = dent.file_type() else {
        return Ok(Vec::new());
    };
    if !ft.is_file() {
        return Ok(Vec::new());
    }
    let rel = rel_path(root, path);
    if !filters.path_allowed(&rel) {
        return Ok(Vec::new());
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Ok(Vec::new()),
    };
    process_file(rel, &bytes, matcher, cell_context)
}

pub fn nb_search(opts: &NbOptions) -> Result<Vec<NbCell>, RgApiError> {
    let (root_in, includes, max_depth, ignore, hidden) =
        resolve_root(&opts.root, &opts.includes, opts.max_depth, opts.ignore, opts.hidden);
    let root = normalize_root(&root_in)?;
    let filters = Arc::new(PathFilters::new(
        &includes,
        &opts.excludes,
        opts.path_re.as_deref(),
        opts.skip_path_re.as_deref(),
        &opts.skip_dirs,
        opts.skip_dir_re.as_deref(),
    )?);
    let matcher = compile_regex(&opts.pattern, opts.case_sensitive, opts.smart_case)?;
    let mut walker = WalkBuilder::new(&root);
    configure_walker(
        &mut walker,
        ignore,
        hidden,
        max_depth,
        opts.min_depth,
        opts.max_filesize,
        opts.follow_links,
        opts.same_file_system,
    );
    filter_dirs(&mut walker, &root, filters.clone());
    let cell_context = opts.cell_context;
    let (tx, rx) = mpsc::channel();
    walker.build_parallel().run(|| {
        let tx = tx.clone();
        let root = root.clone();
        let filters = filters.clone();
        let matcher = matcher.clone();
        Box::new(move |entry| {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                nb_entry(entry, &root, &filters, &matcher, cell_context)
            }))
            .unwrap_or_else(|_| {
                Err(RgApiError::new(
                    "internal error during notebook search (this is a bug, please report it)",
                ))
            });
            match outcome {
                Ok(cells) => {
                    for cell in cells {
                        if tx.send(Ok(cell)).is_err() {
                            return WalkState::Quit;
                        }
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err));
                    return WalkState::Quit;
                }
            }
            WalkState::Continue
        })
    });
    drop(tx);
    rx.into_iter().collect()
}
