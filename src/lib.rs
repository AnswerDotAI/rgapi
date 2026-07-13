//! rgapi - Python-friendly wrappers around ripgrep's walking and searching crates.

mod block;
mod nb;
mod search;
mod walk;

mod python;

pub use block::{block_iter, BlockIter, SearchBlock};
pub use nb::{nb_iter, nb_search, nb_search_file, NbCell, NbIter, NbOptions};
pub use search::{
    compile_regex, rg, rg_iter, search_path, search_text, MatchSpan, RgIter, RgOptions, SearchKind,
    SearchLine,
};
pub use walk::{find, find_cancelable, FindOptions, StreamIter};

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
