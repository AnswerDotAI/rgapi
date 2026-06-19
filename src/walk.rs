use std::path::{Path, PathBuf};
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
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            pattern: None,
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
            files: true,
            dirs: false,
        }
    }
}

pub fn find(opts: &FindOptions) -> Result<Vec<String>, RgApiError> {
    let root = normalize_root(&opts.root)?;
    let filters = Arc::new(PathFilters::new(
        &opts.includes,
        &opts.excludes,
        opts.path_re.as_deref(),
        opts.skip_path_re.as_deref(),
        &opts.skip_dirs,
        opts.skip_dir_re.as_deref(),
    )?);
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
    let (tx, rx) = mpsc::channel();
    let pattern = opts.pattern.clone();
    let files = opts.files;
    let dirs = opts.dirs;
    walker.build_parallel().run(|| {
        let tx = tx.clone();
        let root = root.clone();
        let filters = filters.clone();
        let pattern = pattern.clone();
        Box::new(move |entry| {
            match find_entry(entry, &root, &filters, pattern.as_deref(), files, dirs) {
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

fn find_entry(
    entry: Result<DirEntry, ignore::Error>,
    root: &Path,
    filters: &PathFilters,
    pattern: Option<&str>,
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
    if let Some(pat) = pattern {
        if !rel.contains(pat) {
            return Ok(None);
        }
    }
    if !filters.path_allowed(&rel) {
        return Ok(None);
    }
    Ok(Some(rel))
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
    path_re: Option<RegexMatcher>,
    skip_path_re: Option<RegexMatcher>,
    skip_dirs: Option<GlobSet>,
    skip_dir_re: Option<RegexMatcher>,
}

impl PathFilters {
    pub(crate) fn new(
        includes: &[String],
        excludes: &[String],
        path_re: Option<&str>,
        skip_path_re: Option<&str>,
        skip_dirs: &[String],
        skip_dir_re: Option<&str>,
    ) -> Result<Self, RgApiError> {
        Ok(Self {
            includes: build_globs(includes)?,
            excludes: build_globs(excludes)?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn basename_globs_match_nested_paths() {
        let glob = build_globs(&["*.py".to_string()]).unwrap().unwrap();
        assert!(glob.is_match(Path::new("app.py")));
        assert!(glob.is_match(Path::new("src/app.py")));
        assert!(!glob.is_match(Path::new("src/app.rs")));
    }

    #[test]
    fn path_filters_include_exclude_and_regex_paths() {
        let empty = Vec::new();
        let filters = PathFilters::new(
            &["*.py".to_string()],
            &["skip.py".to_string()],
            Some(r"src/"),
            Some(r"test_"),
            &empty,
            None,
        )
        .unwrap();
        assert!(filters.path_allowed("src/app.py"));
        assert!(!filters.path_allowed("src/skip.py"));
        assert!(!filters.path_allowed("src/app.rs"));
        assert!(!filters.path_allowed("app.py"));
        assert!(!filters.path_allowed("src/test_app.py"));
    }

    #[test]
    fn relative_paths_use_slashes() {
        let root = Path::new("project");
        let path = root.join("src").join("app.py");
        assert_eq!(rel_path(root, &path), "src/app.py");
    }
}
