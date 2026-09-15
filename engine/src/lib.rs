//! type-n-stitch media engine: edit-list math, whisper.cpp parsing and
//! ffmpeg planning. The server is a thin HTTP shell around this crate.

pub mod editlist;
pub mod types;

pub use editlist::*;
pub use types::*;
