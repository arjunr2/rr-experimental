//! `decompose-alternative` - A Component Model IR for graph traversal
//!
//! This crate parses WebAssembly Components into a structure that preserves
//! the 12 index spaces and allows easy traversal of references.

pub mod ir;
pub mod parse;

pub use ir::Component;
pub use parse::parse_component;

// Re-export wasmparser from wirm so consumers use the same versions
pub use wirm;
pub use wirm::wasmparser;
