//! rgapi - Python-friendly wrappers around ripgrep's walking and searching crates.

mod block;
mod nb;
mod search;
mod walk;

#[cfg(feature = "python")]
mod python;

pub use block::{BlockIter, SearchBlock, block_iter};
pub use nb::{NbCell, NbIter, NbOptions, nb_iter, nb_search, nb_search_file};
pub use search::{
    MatchSpan, RgIter, RgOptions, SearchKind, SearchLine, compile_regex, rg, rg_iter, search_path,
    search_text,
};
pub use walk::{FindIter, FindOptions, StreamIter, find, find_iter};

#[derive(Debug, Clone)]
pub struct RgApiError {
    msg: String,
}

impl RgApiError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self { msg: msg.into() }
    }
}

impl std::fmt::Display for RgApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for RgApiError {}

impl From<std::io::Error> for RgApiError {
    fn from(err: std::io::Error) -> Self {
        Self::new(err.to_string())
    }
}
