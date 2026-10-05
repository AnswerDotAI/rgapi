//! rgapi - Python-friendly wrappers around ripgrep's walking and searching crates.

mod block;
mod nb;
mod search;
mod walk;

pub use block::{BlockIter, SearchBlock, block_iter};
pub use nb::{CellRefs, NbCell, NbIter, NbOptions, ancestor_indices, cell_refs, heading_level, nb_iter, nb_search, nb_search_file, section_range};
pub use search::{MatchSpan, RgIter, RgOptions, SearchError, SearchKind, SearchLine, compile_regex, rg, rg_iter, search_path, search_text, spans_for};
pub use walk::{FindIter, FindOptions, StreamIter, WalkOptions, find, find_iter, find_iter_with, resolve_roots};

#[derive(Debug, Clone)]
pub struct RgApiError { msg: String }

impl RgApiError { pub(crate) fn new(msg: impl Into<String>) -> Self { Self { msg: msg.into() } } }

impl std::fmt::Display for RgApiError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.msg) } }

impl std::error::Error for RgApiError {}

impl From<std::io::Error> for RgApiError { fn from(err: std::io::Error) -> Self { Self::new(err.to_string()) } }
