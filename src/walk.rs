use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use globset::{Glob, GlobSet, GlobSetBuilder};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::RgApiError;

#[derive(Debug, Clone)]
pub struct FindOptions {
    pub root: PathBuf,
    pub pattern: Option<String>,
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
    pub files: bool,
    pub dirs: bool,
    pub panic_probe: bool,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            pattern: None,
            includes: Vec::new(),
            excludes: Vec::new(),
            exts: Vec::new(),
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
            files: true,
            dirs: false,
            panic_probe: false,
        }
    }
}

pub fn find(opts: &FindOptions) -> Result<Vec<String>, RgApiError> {
    find_cancelable(opts, None)
}

pub fn find_cancelable(
    opts: &FindOptions,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<Vec<String>, RgApiError> {
    let (root_in, includes, max_depth, ignore, hidden) = resolve_root(
        &opts.root,
        &opts.includes,
        opts.max_depth,
        opts.ignore,
        opts.hidden,
    );
    let root = normalize_root(&root_in)?;
    let filters = Arc::new(PathFilters::new(
        &includes,
        &opts.excludes,
        &opts.exts,
        opts.path_re.as_deref(),
        opts.skip_path_re.as_deref(),
        &opts.skip_dirs,
        opts.skip_dir_re.as_deref(),
    )?);
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
    let (tx, rx) = mpsc::channel();
    let pattern = opts
        .pattern
        .as_deref()
        .map(build_fd_re)
        .transpose()?
        .map(Arc::new);
    let files = opts.files;
    let dirs = opts.dirs;
    let panic_probe = opts.panic_probe;
    walker.build_parallel().run(|| {
        let tx = tx.clone();
        let root = root.clone();
        let filters = filters.clone();
        let pattern = pattern.clone();
        let cancel = cancel.cloned();
        Box::new(move |entry| {
            if let Some(c) = &cancel {
                if c.load(Ordering::Relaxed) {
                    return WalkState::Quit;
                }
            }
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                if panic_probe {
                    panic!("rgapi: deliberate panic for tests (panic_probe)");
                }
                find_entry(entry, &root, &filters, pattern.as_deref(), files, dirs)
            }))
            .unwrap_or_else(|_| {
                Err(RgApiError::new(
                    "internal error during walk (this is a bug, please report it)",
                ))
            });
            match outcome {
                Ok(Some(path)) => {
                    if tx.send(Ok(path)).is_err() {
                        return WalkState::Quit;
                    }
                }
                Ok(None) => {}
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

pub struct StreamIter<T> {
    rx: mpsc::Receiver<Result<T, RgApiError>>,
    cancel: Arc<AtomicBool>,
    _worker: std::thread::JoinHandle<()>,
}

impl<T> StreamIter<T> {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub fn next_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Result<T, RgApiError>, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }
}

impl<T> Iterator for StreamIter<T> {
    type Item = Result<T, RgApiError>;
    fn next(&mut self) -> Option<Self::Item> {
        self.rx.recv().ok()
    }
}

impl<T> Drop for StreamIter<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_walk<T, F>(
    root: PathBuf,
    ignore: bool,
    hidden: bool,
    max_depth: Option<usize>,
    min_depth: Option<usize>,
    max_filesize: Option<u64>,
    follow_links: bool,
    same_file_system: bool,
    filters: Arc<PathFilters>,
    entry: F,
) -> StreamIter<T>
where
    T: Send + 'static,
    F: Fn(
            Result<DirEntry, ignore::Error>,
            &Path,
            &PathFilters,
            &mpsc::SyncSender<Result<T, RgApiError>>,
            &Arc<AtomicBool>,
        ) -> WalkState
        + Send
        + Sync
        + Clone
        + 'static,
{
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (tx, rx) = mpsc::sync_channel(8192);
    let worker = std::thread::spawn(move || {
        let mut walker = WalkBuilder::new(&root);
        configure_walker(
            &mut walker,
            ignore,
            hidden,
            max_depth,
            min_depth,
            max_filesize,
            follow_links,
            same_file_system,
        );
        filter_dirs(&mut walker, &root, filters.clone());
        walker.build_parallel().run(|| {
            let tx = tx.clone();
            let root = root.clone();
            let filters = filters.clone();
            let cancel = worker_cancel.clone();
            let entry = entry.clone();
            Box::new(move |dent| {
                if cancel.load(Ordering::Relaxed) {
                    return WalkState::Quit;
                }
                catch_unwind(AssertUnwindSafe(|| {
                    entry(dent, &root, &filters, &tx, &cancel)
                }))
                .unwrap_or_else(|_| {
                    let _ = tx.send(Err(RgApiError::new(
                        "internal error during search (this is a bug, please report it)",
                    )));
                    WalkState::Quit
                })
            })
        });
    });
    StreamIter {
        rx,
        cancel,
        _worker: worker,
    }
}

fn find_entry(
    entry: Result<DirEntry, ignore::Error>,
    root: &Path,
    filters: &PathFilters,
    pattern: Option<&RegexMatcher>,
    files: bool,
    dirs: bool,
) -> Result<Option<String>, RgApiError> {
    let dent = entry.map_err(|e| RgApiError::new(e.to_string()))?;
    let path = dent.path();
    if path == root {
        return Ok(None);
    }
    let Some(ft) = dent.file_type() else {
        return Ok(None);
    };
    if ft.is_file() && !files {
        return Ok(None);
    }
    if ft.is_dir() && !dirs {
        return Ok(None);
    }
    if !ft.is_file() && !ft.is_dir() {
        return Ok(None);
    }
    let rel = rel_path(root, path);
    if let Some(pattern) = pattern {
        let name = dent.file_name().to_string_lossy();
        if !re_match(pattern, &name) {
            return Ok(None);
        }
    }
    if !filters.path_allowed(&rel) {
        return Ok(None);
    }
    Ok(Some(rel))
}

// If `root` is a file, rewrite the walk to its parent directory matching only that file, so
// passing a filename as `root` searches just that file (like `rg FILE`). Returns the walk root,
// includes, and the max_depth/ignore/hidden to use (depth 1, ignore/hidden off so the named file
// is always found). For a directory, returns the inputs unchanged.
pub(crate) fn resolve_root(
    root: &Path,
    includes: &[String],
    max_depth: Option<usize>,
    ignore: bool,
    hidden: bool,
) -> (PathBuf, Vec<String>, Option<usize>, bool, bool) {
    if root.is_file() {
        if let Some(name) = root.file_name() {
            let parent = match root.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                _ => PathBuf::from("."),
            };
            return (
                parent,
                vec![globset::escape(&name.to_string_lossy())],
                Some(1),
                false,
                true,
            );
        }
    }
    (
        root.to_path_buf(),
        includes.to_vec(),
        max_depth,
        ignore,
        hidden,
    )
}

pub(crate) fn normalize_root(path: &Path) -> Result<PathBuf, RgApiError> {
    if path.exists() {
        Ok(path.canonicalize()?)
    } else {
        Err(RgApiError::new(format!(
            "root does not exist: {}",
            path.display()
        )))
    }
}

pub(crate) fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(crate) fn configure_walker(
    walker: &mut WalkBuilder,
    ignore: bool,
    hidden: bool,
    max_depth: Option<usize>,
    min_depth: Option<usize>,
    max_filesize: Option<u64>,
    follow_links: bool,
    same_file_system: bool,
) {
    walker.standard_filters(ignore);
    if ignore {
        walker.add_custom_ignore_filename(".rgignore");
    }
    walker.hidden(!hidden);
    walker.require_git(false);
    walker.max_depth(max_depth);
    walker.min_depth(min_depth);
    walker.max_filesize(max_filesize);
    walker.follow_links(follow_links);
    walker.same_file_system(same_file_system);
}

pub(crate) fn filter_dirs(walker: &mut WalkBuilder, root: &Path, filters: Arc<PathFilters>) {
    let root = root.to_path_buf();
    walker.filter_entry(move |entry| filters.entry_allowed(&root, entry));
}

pub(crate) struct PathFilters {
    includes: Option<GlobSet>,
    excludes: Option<GlobSet>,
    exts: Option<GlobSet>,
    path_re: Option<RegexMatcher>,
    skip_path_re: Option<RegexMatcher>,
    skip_dirs: Option<GlobSet>,
    skip_dir_re: Option<RegexMatcher>,
}

impl PathFilters {
    pub(crate) fn new(
        includes: &[String],
        excludes: &[String],
        exts: &[String],
        path_re: Option<&str>,
        skip_path_re: Option<&str>,
        skip_dirs: &[String],
        skip_dir_re: Option<&str>,
    ) -> Result<Self, RgApiError> {
        Ok(Self {
            includes: build_globs(includes)?,
            excludes: build_globs(excludes)?,
            exts: build_globs(exts)?,
            path_re: build_path_re(path_re)?,
            skip_path_re: build_path_re(skip_path_re)?,
            skip_dirs: build_globs(skip_dirs)?,
            skip_dir_re: build_path_re(skip_dir_re)?,
        })
    }

    pub(crate) fn path_allowed(&self, rel: &str) -> bool {
        let path = Path::new(rel);
        if let Some(excludes) = &self.excludes {
            if excludes.is_match(path) {
                return false;
            }
        }
        if let Some(skip_path_re) = &self.skip_path_re {
            if re_match(skip_path_re, rel) {
                return false;
            }
        }
        if let Some(path_re) = &self.path_re {
            if !re_match(path_re, rel) {
                return false;
            }
        }
        if let Some(exts) = &self.exts {
            if !exts.is_match(path) {
                return false;
            }
        }
        if let Some(includes) = &self.includes {
            return includes.is_match(path);
        }
        true
    }

    fn entry_allowed(&self, root: &Path, dent: &DirEntry) -> bool {
        let path = dent.path();
        if path == root {
            return true;
        }
        let Some(ft) = dent.file_type() else {
            return true;
        };
        if !ft.is_dir() {
            return true;
        }
        let rel = rel_path(root, path);
        if let Some(skip_dirs) = &self.skip_dirs {
            if skip_dirs.is_match(Path::new(&rel)) {
                return false;
            }
        }
        if let Some(skip_dir_re) = &self.skip_dir_re {
            if re_match(skip_dir_re, &rel) {
                return false;
            }
        }
        true
    }
}

pub(crate) fn build_globs(globs: &[String]) -> Result<Option<GlobSet>, RgApiError> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        add_glob(&mut builder, glob)?;
    }
    Ok(Some(
        builder
            .build()
            .map_err(|e| RgApiError::new(e.to_string()))?,
    ))
}

fn add_glob(builder: &mut GlobSetBuilder, glob: &str) -> Result<(), RgApiError> {
    builder.add(Glob::new(glob).map_err(|e| RgApiError::new(e.to_string()))?);
    if !glob.contains('/') && !glob.contains('\\') {
        builder.add(Glob::new(&format!("**/{glob}")).map_err(|e| RgApiError::new(e.to_string()))?);
    }
    Ok(())
}

fn build_fd_re(pattern: &str) -> Result<RegexMatcher, RgApiError> {
    let mut builder = RegexMatcherBuilder::new();
    builder.case_smart(true);
    builder
        .build(pattern)
        .map_err(|e| RgApiError::new(e.to_string()))
}

fn build_path_re(pattern: Option<&str>) -> Result<Option<RegexMatcher>, RgApiError> {
    pattern
        .map(|pattern| {
            RegexMatcherBuilder::new()
                .build(pattern)
                .map_err(|e| RgApiError::new(e.to_string()))
        })
        .transpose()
}

fn re_match(matcher: &RegexMatcher, rel: &str) -> bool {
    matcher.is_match(rel.as_bytes()).unwrap_or(false)
}
