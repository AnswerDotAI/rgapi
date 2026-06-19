//! rgapi - Python-friendly wrappers around ripgrep's walking and searching crates.

mod search;
mod walk;

#[cfg(feature = "pyo3")]
mod python;

pub use search::{
    compile_regex, rg, rg_iter, search_path, search_text, MatchSpan, RgIter, RgOptions, SearchKind,
    SearchLine,
};
pub use walk::{find, FindOptions};

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
