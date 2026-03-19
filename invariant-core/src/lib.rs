//! Invariant Core - Semantic code analysis library
//!
//! Fast, multi-language structural code analysis using tree-sitter for parsing.
//! Extracts Prolog-compatible facts from source code and uploads them to
//! DataGrout Invariant for server-side semantic enrichment and querying.
//!
//! # Architecture
//!
//! - **Parser**: Tree-sitter based multi-language AST extraction
//! - **Analyzer**: Extracts structural facts (functions, modules, imports, calls)
//! - **Facts**: Prolog-compatible fact generation and formatting
//! - **Bridge**: Conduit SDK client for DataGrout Invariant integration
//!
//! # Example
//!
//! ```rust,no_run
//! use invariant_core::{Analyzer, Language};
//!
//! fn main() -> anyhow::Result<()> {
//!     let mut analyzer = Analyzer::new()?;
//!
//!     let code = r#"
//!         def calculate_total(items):
//!             return sum(item.price for item in items)
//!     "#;
//!
//!     let result = analyzer.lens_code(code, Language::Python, "calc.py", "abc123")?;
//!
//!     println!("Extracted {} facts", result.facts.len());
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod analyzer;
pub mod bridge;
pub mod config;
pub mod facts;
pub mod parser;
pub mod types;

pub use analyzer::Analyzer;
pub use config::Config;
pub use facts::{Fact, FactValue};
pub use parser::{Language, Parser};
pub use types::*;

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
