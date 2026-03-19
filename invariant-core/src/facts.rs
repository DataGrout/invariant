//! Prolog fact generation and formatting
//!
//! This module provides types and utilities for generating Prolog facts from code analysis.

use serde::{Deserialize, Serialize};

/// A Prolog fact value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FactValue {
    /// String value (rendered as '...')
    String(String),

    /// Atom value (rendered as atom)
    Atom(String),

    /// Integer value
    Integer(i64),

    /// Float value
    Float(f64),

    /// List of values
    List(Vec<FactValue>),

    /// Compound term
    Compound(String, Vec<FactValue>),
}

/// A Prolog fact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    /// Predicate name
    pub predicate: String,

    /// Arguments
    pub args: Vec<FactValue>,
}

impl Fact {
    /// Create a new fact
    pub fn new(predicate: impl Into<String>, args: Vec<FactValue>) -> Self {
        Self {
            predicate: predicate.into(),
            args,
        }
    }

    /// Convert fact to Prolog string format
    pub fn to_prolog(&self) -> String {
        let args_str = self
            .args
            .iter()
            .map(|arg| arg.to_prolog())
            .collect::<Vec<_>>()
            .join(", ");

        format!("{}({}).", self.predicate, args_str)
    }
}

impl FactValue {
    /// Convert value to Prolog string format
    pub fn to_prolog(&self) -> String {
        match self {
            FactValue::String(s) => format!("'{}'", escape_prolog_string(s)),
            FactValue::Atom(a) => a.clone(),
            FactValue::Integer(i) => i.to_string(),
            FactValue::Float(f) => f.to_string(),
            FactValue::List(items) => {
                let items_str = items
                    .iter()
                    .map(|v| v.to_prolog())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{}]", items_str)
            }
            FactValue::Compound(name, args) => {
                let args_str = args
                    .iter()
                    .map(|v| v.to_prolog())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", name, args_str)
            }
        }
    }
}

/// Escape special characters in Prolog strings
fn escape_prolog_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Normalize identifier to valid Prolog atom
pub fn normalize_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c.to_lowercase().next().unwrap()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fact_to_prolog() {
        let fact = Fact::new(
            "function",
            vec![
                FactValue::String("my_func".to_string()),
                FactValue::String("MyModule".to_string()),
                FactValue::String("my_func".to_string()),
                FactValue::Integer(2),
                FactValue::Atom("public".to_string()),
                FactValue::Integer(42),
                FactValue::String("abc123".to_string()),
            ],
        );

        assert_eq!(
            fact.to_prolog(),
            "function('my_func', 'MyModule', 'my_func', 2, public, 42, 'abc123')."
        );
    }

    #[test]
    fn test_list_fact() {
        let fact = Fact::new(
            "dependency_cycle",
            vec![FactValue::List(vec![
                FactValue::String("A".to_string()),
                FactValue::String("B".to_string()),
                FactValue::String("C".to_string()),
            ])],
        );

        assert_eq!(fact.to_prolog(), "dependency_cycle(['A', 'B', 'C']).");
    }

    #[test]
    fn test_escape_prolog_string() {
        assert_eq!(escape_prolog_string("hello'world"), "hello\\'world");
        assert_eq!(escape_prolog_string("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_normalize_id() {
        assert_eq!(normalize_id("MyFunction"), "myfunction");
        assert_eq!(normalize_id("my-function"), "my_function");
        assert_eq!(normalize_id("my.module.func"), "my_module_func");
    }
}
