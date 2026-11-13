//! rgapi - Python-friendly wrappers around ripgrep's walking and searching crates.

use std::fmt::Display;
mod block;
mod matcher;
mod nb;
mod search;
mod walk;

pub use block::{BlockIter, SearchBlock, block_iter};
pub use matcher::{RegexMatcher, compile_regex};
pub use nb::{CellRefs, NbCell, NbIter, NbOptions, ancestor_indices, cell_refs, heading_level, nb_iter, nb_search, nb_search_file, section_range};
pub use search::{MatchSpan, RgIter, RgOptions, SearchError, SearchKind, SearchLine, rg, rg_iter, search_path, search_text, spans_for};
pub use walk::{FindIter, FindOptions, StreamIter, WalkOptions, find, find_iter, find_iter_with, resolve_roots};

/// An rgapi failure. `is_skipped` marks an error naming one path that a walk couldn't read, such as a directory without read permission.
/// The walk sends that error and continues, as ripgrep does. Rust's collecting functions, such as `find` and `StreamIter::collect_timeout`, skip these and return any other error.
#[derive(Debug, Clone)]
pub struct RgApiError { msg: String, skipped: bool }

impl RgApiError {
    pub(crate) fn new(msg: impl Into<String>) -> Self { Self { msg: msg.into(), skipped: false } }
    pub(crate) fn skipped(msg: impl Display) -> Self { Self { msg: msg.to_string(), skipped: true } }
    pub(crate) fn unreadable(path: &std::path::Path, err: impl Display) -> Self { Self::skipped(format!("{}: {err}", path.display())) }
    pub fn is_skipped(&self) -> bool { self.skipped }
}

impl Display for RgApiError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.msg) } }

impl std::error::Error for RgApiError {}

impl From<std::io::Error> for RgApiError { fn from(err: std::io::Error) -> Self { Self::new(err.to_string()) } }
