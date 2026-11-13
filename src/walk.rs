use mpsc::{RecvTimeoutError, SyncSender};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use grep_matcher::Matcher;
use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::{RegexMatcher, RgApiError};

/// Where to walk and which paths to keep. `FindOptions`, `RgOptions` and `NbOptions` each hold one.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Directories or files to walk. Result paths are relative to the base of the roots. Filters match the same relative paths.
    /// A directory root is its own base. A file root, or a link root that is not followed, has its parent as its base.
    /// Several roots use the common ancestor of their bases. A path under more than one root appears once.
    pub roots: Vec<PathBuf>,
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
    /// Keep the walk inside this folder. A walked root must resolve inside it. With `follow_links`, a link to a file or folder
    /// that resolves outside it is not returned, read or entered.
    pub confine: Option<PathBuf>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self { roots: vec![PathBuf::from(".")], includes: Vec::new(), excludes: Vec::new(), exts: Vec::new(), path_re: None, skip_path_re: None,
            skip_dirs: Vec::new(), skip_dir_re: None, hidden: false, ignore: true, max_depth: None, min_depth: None, max_filesize: None, follow_links: false,
            same_file_system: false, confine: None }
    }
}

#[derive(Debug, Clone)]
pub struct FindOptions {
    pub walk: WalkOptions,
    pub pattern: Option<String>,
    pub files: bool,
    pub dirs: bool,
    /// Return special entries (FIFOs, sockets, devices) so callers can handle or reject them.
    pub special_files: bool,
    pub panic_probe: bool,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self { walk: WalkOptions::default(), pattern: None, files: true, dirs: false, special_files: false, panic_probe: false }
    }
}

pub fn find(opts: &FindOptions) -> Result<Vec<PathBuf>, RgApiError> { find_iter(opts)?.collect() }

pub type FindIter = StreamIter<PathBuf>;

pub fn find_iter(opts: &FindOptions) -> Result<FindIter, RgApiError> { find_iter_with(opts, false) }

/// Like `find_iter`, except that `walk_root_links` walks a root link to a directory instead of returning the link.
/// Paths under that root are reported under the link.
pub fn find_iter_with(opts: &FindOptions, walk_root_links: bool) -> Result<FindIter, RgApiError> {
    let walk = &opts.walk;
    let (roots, base) = resolve_roots(walk, false, walk_root_links)?;
    let filters = Arc::new(PathFilters::new(walk)?);
    let pattern = opts.pattern.as_deref().map(|p| RegexMatcher::path(p, true)).transpose()?;
    // The walker follows every root it is given. A link root returned as itself must not reach it.
    let (links, roots): (Vec<_>, Vec<_>) = roots.into_iter().partition(|r| r.is_symlink() && !walk.follow_links && !(walk_root_links && r.is_dir()));
    let mut ready = Vec::new();
    if walk.min_depth.unwrap_or(0) == 0 {
        for link in links {
            let rel = relative_path(&base, &link);
            if find_matches(rel, &filters, pattern.as_ref())? { ready.push(rel.to_path_buf()); }
        }
    }
    let (files, dirs, special_files, panic_probe, max_depth) = (opts.files, opts.dirs, opts.special_files, opts.panic_probe, walk.max_depth);
    Ok(spawn_walk(roots, base, walk, filters, ready, move |dent, base, filters, tx, cancel| {
        if panic_probe { panic!("rgapi: deliberate panic for tests (panic_probe)"); }
        match find_entry(dent, base, filters, pattern.as_ref(), files, dirs, special_files, max_depth) {
            Ok(Some(path)) => { if cancel.load(Ordering::Relaxed) || tx.send(Ok(path)).is_err() { return WalkState::Quit; } WalkState::Continue }
            Ok(None) => WalkState::Continue, Err(err) => send_error(tx, err)
        }
    }))
}

pub struct StreamIter<T> { rx: mpsc::Receiver<Result<T, RgApiError>>, cancel: Arc<AtomicBool>, worker: Option<JoinHandle<()>> }

impl<T> StreamIter<T> {
    pub fn cancel(&self) { self.cancel.store(true, Ordering::Relaxed); }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> { self.cancel.clone() }

    /// Cancel and wait for the walk's workers to finish. Drain queued sends before
    /// joining so a full result channel cannot deadlock shutdown. Unlike Drop,
    /// this guarantees no background walk remains when it returns. Filesystem
    /// calls already in progress must return first; run this off an async executor.
    pub fn cancel_and_join(mut self) -> Result<(), RgApiError> {
        self.cancel();
        while self.rx.recv().is_ok() {}
        if let Some(worker) = self.worker.take() { worker.join().map_err(|_| RgApiError::new("search worker panicked"))?; }
        Ok(())
    }

    pub fn next_timeout(&mut self, timeout: Duration) -> Result<Result<T, RgApiError>, RecvTimeoutError> { self.rx.recv_timeout(timeout) }

    /// Collect all items, stopping at `timeout_ms`; the bool is true when the deadline stopped it.
    pub fn collect_timeout(mut self, timeout_ms: Option<u64>) -> Result<(Vec<T>, bool), RgApiError> {
        let Some(ms) = timeout_ms else { return Ok((self.collect::<Result<Vec<_>, _>>()?, false)); };
        let deadline = Instant::now() + Duration::from_millis(ms);
        let mut res = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() { return Ok((res, true)); }
            match self.next_timeout(left) {
                Ok(Ok(item)) => res.push(item), Ok(Err(err)) => return Err(err), Err(RecvTimeoutError::Disconnected) => return Ok((res, false)),
                Err(RecvTimeoutError::Timeout) => return Ok((res, true))
            }
        }
    }
}

impl<T> Iterator for StreamIter<T> {
    type Item = Result<T, RgApiError>;
    fn next(&mut self) -> Option<Self::Item> { self.rx.recv().ok() }
}

impl<T> Drop for StreamIter<T> { fn drop(&mut self) { self.cancel(); } }

#[cfg(test)]
mod close_tests {
    use super::*;
    use std::thread;

    #[test]
    fn cancel_and_join_drains_full_channel_and_waits_for_worker() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (started, ready) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = done.clone();
        let worker = thread::spawn(move || {
            tx.send(Ok(1)).unwrap();
            started.send(()).unwrap();
            // This blocks while the channel is full; close must drain, not just join.
            tx.send(Ok(2)).unwrap();
            assert!(worker_cancel.load(Ordering::Acquire));
            worker_done.store(true, Ordering::Release);
        });
        ready.recv().unwrap();
        StreamIter { rx, cancel: cancel.clone(), worker: Some(worker) }.cancel_and_join().unwrap();
        assert!(cancel.load(Ordering::Acquire));
        assert!(done.load(Ordering::Acquire));
    }
}

pub(crate) fn spawn_walk<T, F>(roots: Vec<PathBuf>, base: PathBuf, walk: &WalkOptions, filters: Arc<PathFilters>, ready: Vec<T>, entry: F) -> StreamIter<T>
where T: Send + 'static, F: Fn(Result<DirEntry, ignore::Error>, &Path, &PathFilters, &SyncSender<Result<T, RgApiError>>, &Arc<AtomicBool>) -> WalkState + Send
        + Sync + Clone + 'static {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (tx, rx) = mpsc::sync_channel(8192);
    let walk = walk.clone();
    let seen = roots.iter().any(|a| roots.iter().any(|b| a != b && a.starts_with(b))).then(|| Arc::new(Mutex::new(HashSet::new())));
    let worker = thread::spawn(move || {
        for item in ready { if tx.send(Ok(item)).is_err() { return; } }
        for root in &roots {
            if worker_cancel.load(Ordering::Relaxed) { return; }
            let mut walker = WalkBuilder::new(root);
            configure_walker(&mut walker, root, &walk);
            filter_dirs(&mut walker, &base, filters.clone(), tx.clone(), worker_cancel.clone());
            walker.build_parallel().run(|| {
                let (tx, base, filters, cancel, entry, seen) = (tx.clone(), base.clone(), filters.clone(), worker_cancel.clone(), entry.clone(), seen.clone());
                Box::new(move |dent| {
                    if cancel.load(Ordering::Relaxed) { return WalkState::Quit; }
                    if let (Some(seen), Ok(d)) = (&seen, &dent) && !seen.lock().unwrap().insert(d.path().to_path_buf()) { return WalkState::Continue; }
                    catch_unwind(AssertUnwindSafe(|| entry(dent, &base, &filters, &tx, &cancel))).unwrap_or_else(|_| {
                        let _ = tx.send(Err(RgApiError::new("internal error during search (this is a bug, please report it)")));
                        WalkState::Quit
                    })
                })
            });
        }
    });
    StreamIter { rx, cancel, worker: Some(worker) }
}

fn find_entry(
    entry: Result<DirEntry, ignore::Error>, base: &Path, filters: &PathFilters, pattern: Option<&RegexMatcher>, files: bool, dirs: bool, special_files: bool,
    max_depth: Option<usize>) -> Result<Option<PathBuf>, RgApiError> {
    let dent = match entry {
        Ok(dent) => dent,
        Err(err) => {
            if let Some(path) = dangling_link(&err) {
                let rel = relative_path(base, path);
                return Ok(find_matches(rel, filters, pattern)?.then(|| rel.to_path_buf()));
            }
            return entry_err(err, max_depth).map_or(Ok(None), Err);
        }
    };
    let path = dent.path();
    let Some(ft) = dent.file_type() else { return Ok(None); };
    if dent.depth() == 0 && ft.is_dir() { return Ok(None); }
    if ft.is_file() && !files { return Ok(None); }
    if ft.is_dir() && !dirs { return Ok(None); }
    if !ft.is_file() && !ft.is_dir() && !ft.is_symlink() && !special_files { return Ok(None); }
    let rel = relative_path(base, path);
    Ok(find_matches(rel, filters, pattern)?.then(|| rel.to_path_buf()))
}

// With `follow_links` the walker reports a dangling link as an error; it is a dangling link when the path is a symlink whose target is missing.
fn dangling_link(err: &ignore::Error) -> Option<&Path> {
    match err {
        ignore::Error::WithPath { path, .. } => (path.is_symlink() && !path.exists()).then_some(path.as_path()),
        ignore::Error::WithDepth { err, .. } => dangling_link(err), _ => None
    }
}

fn find_matches(path: &Path, filters: &PathFilters, pattern: Option<&RegexMatcher>) -> Result<bool, RgApiError> {
    if let Some(pattern) = pattern { if !re_match(pattern, &path.file_name().unwrap_or_default().to_string_lossy())? { return Ok(false); } }
    filters.path_allowed(path)
}

// An explicitly named file is always searched, like `rg FILE`: for a file root,
// disable ignore rules and include hidden. Nothing is traversed below a file, so
// the flags affect only the root itself.
fn file_root_flags(root: &Path, ignore: bool, hidden: bool) -> (bool, bool) { if root.is_file() { (false, true) } else { (ignore, hidden) } }

// At the max_depth cap the walker opens directories it will never descend into
// (readdir precedes the depth check in `ignore`), so permission failures there are
// harmless: skip them silently (None). Report every other walk error as a skipped path (Some).
pub(crate) fn entry_err(err: ignore::Error, max_depth: Option<usize>) -> Option<RgApiError> {
    let at_cap = max_depth.is_some_and(|m| err.depth().is_some_and(|d| d >= m));
    let denied = err.io_error().is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
    if at_cap && denied { None } else { Some(RgApiError::skipped(err)) }
}

/// Send `err`, then end the walk unless `err` only skipped a path.
pub(crate) fn send_error<T>(tx: &SyncSender<Result<T, RgApiError>>, err: RgApiError) -> WalkState {
    let skipped = err.is_skipped();
    if tx.send(Err(err)).is_ok() && skipped { WalkState::Continue } else { WalkState::Quit }
}

fn normalize_root(path: &Path) -> Result<PathBuf, RgApiError> {
    if path.exists() { Ok(path.canonicalize()?) } else { Err(RgApiError::new(format!("root does not exist: {}", path.display()))) }
}

/// Return the distinct roots and their base. Each root is made absolute. With `canonical`, each root is also canonicalized.
/// With `confine`, a root that is walked, not returned as a link, must resolve inside it.
pub fn resolve_roots(walk: &WalkOptions, canonical: bool, walk_root_links: bool) -> Result<(Vec<PathBuf>, PathBuf), RgApiError> {
    let mut roots = Vec::new();
    for root in &walk.roots {
        let root = if canonical { normalize_root(root)? } else {
            let r = std::path::absolute(root)?;
            r.symlink_metadata()?;
            r
        };
        if !roots.contains(&root) { roots.push(root); }
    }
    let follow = walk.follow_links || walk_root_links;
    let mut bases =
        roots.iter().map(|r| if (follow || !r.is_symlink()) && r.is_dir() { r.clone() } else { r.parent().map_or_else(|| r.clone(), Path::to_path_buf) });
    let mut base = bases.next().unwrap_or_default();
    for b in bases { while !b.starts_with(&base) && base.pop() {} }
    if let Some(confine) = confine_dir(walk)?
        && let Some(r) = roots.iter().find(|r| (follow || !r.is_symlink()) && r.canonicalize().is_ok_and(|c| !c.starts_with(&confine))) {
        return Err(RgApiError::new(format!("root is outside {}: {}", confine.display(), r.display())));
    }
    Ok((roots, base))
}

fn confine_dir(walk: &WalkOptions) -> Result<Option<PathBuf>, RgApiError> {
    walk.confine.as_deref().map(|c| c.canonicalize().map_err(|e| RgApiError::new(format!("confine {}: {e}", c.display())))).transpose()
}

fn relative_path<'a>(base: &Path, path: &'a Path) -> &'a Path {
    let rel = path.strip_prefix(base).unwrap_or(path);
    if rel.as_os_str().is_empty() { Path::new(path.file_name().unwrap_or_default()) } else { rel }
}

fn path_label(path: &Path) -> String { path.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/") }

pub(crate) fn rel_path(base: &Path, path: &Path) -> String { path_label(relative_path(base, path)) }

fn configure_walker(walker: &mut WalkBuilder, root: &Path, walk: &WalkOptions) {
    let (ignore, hidden) = file_root_flags(root, walk.ignore, walk.hidden);
    walker.standard_filters(ignore);
    if ignore { walker.add_custom_ignore_filename(".rgignore"); }
    walker.hidden(!hidden);
    walker.require_git(false);
    walker.max_depth(walk.max_depth);
    walker.min_depth(walk.min_depth);
    walker.max_filesize(walk.max_filesize);
    walker.follow_links(walk.follow_links);
    walker.same_file_system(walk.same_file_system);
}

fn filter_dirs<T: Send + 'static>(
    walker: &mut WalkBuilder, base: &Path, filters: Arc<PathFilters>, tx: SyncSender<Result<T, RgApiError>>, cancel: Arc<AtomicBool>) {
    let base = base.to_path_buf();
    walker.filter_entry(move |entry| {
        if cancel.load(Ordering::Relaxed) { false } else {
            match filters.entry_allowed(&base, entry) { Ok(allowed) => allowed, Err(err) => { let _ = tx.send(Err(err)); false } }
        }
    });
}

pub(crate) struct PathFilters {
    includes: Option<GlobSet>, excludes: Option<GlobSet>, exts: Option<GlobSet>, path_re: Option<RegexMatcher>, skip_path_re: Option<RegexMatcher>,
    skip_dirs: Option<GlobSet>, skip_dir_re: Option<RegexMatcher>, confine: Option<PathBuf>
}

impl PathFilters {
    pub(crate) fn new(walk: &WalkOptions) -> Result<Self, RgApiError> {
        Ok(Self { includes: build_globs(&walk.includes)?, excludes: build_globs(&walk.excludes)?, exts: build_globs(&walk.exts)?,
            path_re: build_path_re(walk.path_re.as_deref())?, skip_path_re: build_path_re(walk.skip_path_re.as_deref())?,
            skip_dirs: build_globs(&walk.skip_dirs)?, skip_dir_re: build_path_re(walk.skip_dir_re.as_deref())?,
            confine: if walk.follow_links { confine_dir(walk)? } else { None } })
    }

    pub(crate) fn path_allowed(&self, path: &Path) -> Result<bool, RgApiError> {
        if let Some(excludes) = &self.excludes && excludes.is_match(path) { return Ok(false); }
        if let Some(skip_path_re) = &self.skip_path_re && re_match(skip_path_re, &path_label(path))? { return Ok(false); }
        if let Some(path_re) = &self.path_re && !re_match(path_re, &path_label(path))? { return Ok(false); }
        if let Some(exts) = &self.exts && !exts.is_match(path) { return Ok(false); }
        if let Some(includes) = &self.includes { return Ok(includes.is_match(path)); }
        Ok(true)
    }

    fn entry_allowed(&self, base: &Path, dent: &DirEntry) -> Result<bool, RgApiError> {
        let path = dent.path();
        if dent.depth() == 0 { return Ok(true); }
        if let Some(confine) = &self.confine && dent.path_is_symlink() && !path.canonicalize().is_ok_and(|p| p.starts_with(confine)) { return Ok(false); }
        let Some(ft) = dent.file_type() else { return Ok(true); };
        if !ft.is_dir() { return Ok(true); }
        let rel = relative_path(base, path);
        if self.excludes.as_ref().is_some_and(|globs| globs.is_match(rel)) { return Ok(false); }
        if let Some(skip_dirs) = &self.skip_dirs && skip_dirs.is_match(rel) { return Ok(false); }
        if let Some(skip_dir_re) = &self.skip_dir_re && re_match(skip_dir_re, &path_label(rel))? { return Ok(false); }
        Ok(true)
    }
}

pub(crate) fn build_globs(globs: &[String]) -> Result<Option<GlobSet>, RgApiError> {
    if globs.is_empty() { return Ok(None); }
    let mut builder = GlobSetBuilder::new();
    for glob in globs { add_glob(&mut builder, glob)?; }
    Ok(Some(builder.build().map_err(|e| RgApiError::new(e.to_string()))?))
}

fn add_glob(builder: &mut GlobSetBuilder, glob: &str) -> Result<(), RgApiError> {
    let pattern = if glob.contains('/') { glob.to_owned() } else { format!("**/{glob}") };
    builder.add(GlobBuilder::new(&pattern).literal_separator(true).build().map_err(|e| RgApiError::new(e.to_string()))?);
    Ok(())
}

fn build_path_re(pattern: Option<&str>) -> Result<Option<RegexMatcher>, RgApiError> { pattern.map(|pattern| RegexMatcher::path(pattern, false)).transpose() }

fn re_match(matcher: &RegexMatcher, rel: &str) -> Result<bool, RgApiError> { matcher.is_match(rel.as_bytes()).map_err(|e| RgApiError::new(e.to_string())) }

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn globs_match_names_or_root_relative_components() {
        for (glob, yes, no) in [
            ("*.py", "src/deep/app.py", "src/app.rs"), ("src/*", "src/app.py", "src/deep/app.py"), ("src/**", "src/deep/app.py", "other/src/app.py"),
            ("tests", "src/tests", "src/tests/app.py"),
        ] {
            let globs = build_globs(&[glob.into()]).unwrap().unwrap();
            assert!(globs.is_match(yes), "{glob}: {yes}");
            assert!(!globs.is_match(no), "{glob}: {no}");
        }
    }

    fn iter_of(items: Vec<u32>, delay_ms: u64) -> StreamIter<u32> {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = thread::spawn(move || {
            for i in items { thread::sleep(Duration::from_millis(delay_ms)); if tx.send(Ok(i)).is_err() { return; } }
        });
        StreamIter { rx, cancel, worker: Some(worker) }
    }

    #[test]
    fn collect_timeout_honors_deadline() {
        let (got, timed_out) = iter_of(vec![1, 2, 3], 0).collect_timeout(None).unwrap();
        assert_eq!(got, vec![1, 2, 3]);
        assert!(!timed_out);

        let (got, timed_out) = iter_of(vec![1, 2, 3], 0).collect_timeout(Some(60_000)).unwrap();
        assert_eq!(got, vec![1, 2, 3]);
        assert!(!timed_out);

        let (got, timed_out) = iter_of(vec![1, 2, 3], 50).collect_timeout(Some(0)).unwrap();
        assert!(got.is_empty());
        assert!(timed_out);

        let (got, timed_out) = iter_of(vec![1, 2, 3], 200).collect_timeout(Some(20)).unwrap();
        assert!(got.is_empty());
        assert!(timed_out);
    }

    #[cfg(unix)]
    #[test]
    fn confine_skips_links_leading_outside() {
        use std::os::unix::fs::symlink;
        let tmp = std::env::temp_dir().join(format!("rgapi-confine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (root, out) = (tmp.join("root"), tmp.join("out"));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        for p in [root.join("in.txt"), root.join("sub/deep.txt"), out.join("far.txt")] { std::fs::write(p, "needle\n").unwrap(); }
        symlink(root.join("sub"), root.join("insub")).unwrap();
        symlink(&out, root.join("outdir")).unwrap();
        symlink(out.join("far.txt"), root.join("far.txt")).unwrap();
        symlink(root.join("missing.txt"), root.join("gone.txt")).unwrap();
        let walk = |roots: Vec<PathBuf>, confine: bool| WalkOptions { roots, follow_links: true, confine: confine.then(|| root.clone()), ..Default::default() };
        let names = |w: WalkOptions| {
            let mut v: Vec<String> = find_iter(&FindOptions { walk: w, ..Default::default() }).unwrap().filter_map(Result::ok).map(|p| path_label(&p)).collect();
            v.sort();
            v
        };
        assert_eq!(names(walk(vec![root.clone()], false)), ["far.txt", "gone.txt", "in.txt", "insub/deep.txt", "outdir/far.txt", "sub/deep.txt"]);
        assert_eq!(names(walk(vec![root.clone()], true)), ["gone.txt", "in.txt", "insub/deep.txt", "sub/deep.txt"]);
        let rg = crate::RgOptions { walk: walk(vec![root.clone()], true), pattern: "needle".into(), ..Default::default() };
        let mut hits: Vec<String> = crate::rg_iter(&rg).unwrap().filter_map(Result::ok).map(|l| l.path).collect();
        hits.sort();
        assert_eq!(hits, ["in.txt", "insub/deep.txt", "sub/deep.txt"]);
        assert!(find_iter(&FindOptions { walk: walk(vec![root.join("outdir")], true), ..Default::default() }).is_err());
        let unfollowed = WalkOptions { roots: vec![root.join("outdir")], confine: Some(root.clone()), ..Default::default() };
        assert_eq!(names(unfollowed), ["outdir"]);
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
