//! Typed manifests, relative Markdown loading and validation without runtime construction.
mod parse;
mod types;
mod validate;
pub use crate::error::ConfigError;
pub use parse::load_dir;
pub use types::*;
/// Manifest parsing and validation result.
pub type Result<T> = std::result::Result<T, ConfigError>;
#[cfg(test)]
mod test;
