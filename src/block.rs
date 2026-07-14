use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use grep_regex::RegexMatcher;
use ignore::{DirEntry, WalkState};

use crate::search::{compile_regex, format_lnhash, search_text, RgOptions, SearchLine};
use crate::walk::{
    entry_err, file_root_flags, normalize_root, rel_path, spawn_walk, PathFilters, StreamIter,
};
use crate::RgApiError;

/// One blank-line-delimited block containing a match, or context for one.
pub struct SearchBlock {
    pub path: String,
    pub block_index: usize,
    pub start_line: u64,
    pub end_line: u64,
    pub start_lnhash: String,
    pub end_lnhash: String,
    pub kind: &'static str, // "match" | "context"
    pub source: String,
    pub matches: Vec<SearchLine>,
}

struct BlockInfo {
    start_line: u64,
    end_line: u64,
    source: String,
}

fn split_blocks(text: &str) -> Vec<BlockInfo> {
    let mut blocks = Vec::new();
    let mut start = None;
    let mut lines = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line_no = i as u64 + 1;
        if line.trim().is_empty() {
            if let Some(first) = start.take() {
                blocks.push(BlockInfo {
                    start_line: first,
                    end_line: line_no - 1,
                    source: lines.join("\n"),
                });
                lines.clear();
            }
        } else {
            if start.is_none() {
                start = Some(line_no);
            }
            lines.push(line);
        }
    }
    if let Some(first) = start {
        blocks.push(BlockInfo {
            start_line: first,
            end_line: text.lines().count() as u64,
            source: lines.join("\n"),
        });
    }
    blocks
}

fn process_file(
    disp: String,
    bytes: &[u8],
    matcher: &RegexMatcher,
    before_context: usize,
    after_context: usize,
) -> Result<Vec<SearchBlock>, RgApiError> {
    if bytes.contains(&0) {
        return Ok(Vec::new());
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return Ok(Vec::new()),
    };
    let blocks = split_blocks(text);
    let hits = search_text(disp.clone(), text, matcher.clone(), 0, 0)?;
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let mut matched: HashMap<usize, Vec<SearchLine>> = HashMap::new();
    for hit in hits {
        if let Some((i, _)) = blocks
            .iter()
            .enumerate()
            .find(|(_, b)| b.start_line <= hit.line_number && hit.line_number <= b.end_line)
        {
            matched.entry(i).or_default().push(hit);
        }
    }
    let mut emit: BTreeMap<usize, bool> = BTreeMap::new();
    for i in matched.keys() {
        emit.insert(*i, true);
        let start = i.saturating_sub(before_context);
        let end = (i + after_context + 1).min(blocks.len());
        for j in start..end {
            emit.entry(j).or_insert(false);
        }
    }
    Ok(emit
        .into_iter()
        .map(|(i, is_match)| {
            let block = &blocks[i];
            SearchBlock {
                path: disp.clone(),
                block_index: i,
                start_line: block.start_line,
                end_line: block.end_line,
                start_lnhash: format_lnhash(block.start_line, block.source.lines().next().unwrap()),
                end_lnhash: format_lnhash(block.end_line, block.source.lines().last().unwrap()),
                kind: if is_match { "match" } else { "context" },
                source: block.source.clone(),
                matches: matched.remove(&i).unwrap_or_default(),
            }
        })
        .collect())
}

fn block_entry(
    entry: Result<DirEntry, ignore::Error>,
    root: &Path,
    filters: &PathFilters,
    matcher: &RegexMatcher,
    before_context: usize,
    after_context: usize,
    max_depth: Option<usize>,
) -> Result<Vec<SearchBlock>, RgApiError> {
    let dent = match entry {
        Ok(dent) => dent,
        Err(err) => return entry_err(err, max_depth).map_or(Ok(Vec::new()), Err),
    };
    let path = dent.path();
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
        Ok(bytes) => bytes,
        Err(_) => return Ok(Vec::new()),
    };
    process_file(rel, &bytes, matcher, before_context, after_context)
}

pub type BlockIter = StreamIter<SearchBlock>;

pub fn block_iter(opts: &RgOptions) -> Result<BlockIter, RgApiError> {
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
    let matcher = compile_regex(&opts.pattern, opts.case_sensitive, opts.smart_case)?;
    let (before_context, after_context, max_depth) =
        (opts.before_context, opts.after_context, opts.max_depth);
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
        move |dent, root, filters, tx, cancel| match block_entry(
            dent,
            root,
            filters,
            &matcher,
            before_context,
            after_context,
            max_depth,
        ) {
            Ok(blocks) => {
                for block in blocks {
                    if cancel.load(Ordering::Relaxed) || tx.send(Ok(block)).is_err() {
                        return WalkState::Quit;
                    }
                }
                WalkState::Continue
            }
            Err(err) => {
                let _ = tx.send(Err(err));
                WalkState::Quit
            }
        },
    ))
}
