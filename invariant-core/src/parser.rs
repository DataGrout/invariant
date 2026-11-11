//! Multi-language parser using tree-sitter

use crate::types::{Error, Result};
use tree_sitter::{Node, Parser as TSParser, Tree};

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    /// Rust
    Rust,
    /// Python
    Python,
    /// TypeScript
    TypeScript,
    /// TSX (TypeScript with JSX)
    Tsx,
    /// JavaScript
    JavaScript,
    /// Go
    Go,
    /// Elixir
    Elixir,
    /// Ruby
    Ruby,
    /// PHP (HTML-intermixed grammar, so templates/WordPress files lens too)
    Php,
    /// Prolog (ISO + SWI + ProbLog superset)
    Prolog,
    /// C#
    CSharp,
    /// T-SQL (SQL Server, Azure SQL), with our own grammar
    TSql,
    /// PowerShell
    PowerShell,
    /// Java
    Java,
    /// Kotlin
    Kotlin,
    /// .NET project and solution files (`.csproj`, `.fsproj`, `.vbproj`,
    /// `Directory.*.props`, `.sln`): read as XML or text, not tree-sitter.
    MsBuild,
}

impl Language {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Language::Rust),
            "py" | "pyw" => Some(Language::Python),
            "ts" | "mts" | "cts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "go" => Some(Language::Go),
            "ex" | "exs" => Some(Language::Elixir),
            "rb" | "rake" | "gemspec" => Some(Language::Ruby),
            "php" | "phtml" => Some(Language::Php),
            "pl" | "plt" | "pro" | "prolog" => Some(Language::Prolog),
            "cs" | "csx" => Some(Language::CSharp),
            "sql" => Some(Language::TSql),
            "ps1" | "psm1" => Some(Language::PowerShell),
            "java" => Some(Language::Java),
            "kt" | "kts" => Some(Language::Kotlin),
            "csproj" | "fsproj" | "vbproj" | "props" | "targets" | "sln" | "slnx" => {
                Some(Language::MsBuild)
            }
            _ => None,
        }
    }

    /// Get file extensions for this language
    pub fn extensions(&self) -> &[&str] {
        match self {
            Language::Rust => &["rs"],
            Language::Python => &["py", "pyw"],
            Language::TypeScript => &["ts", "mts", "cts"],
            Language::Tsx => &["tsx"],
            Language::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Language::Go => &["go"],
            Language::Elixir => &["ex", "exs"],
            Language::Ruby => &["rb", "rake", "gemspec"],
            Language::Php => &["php", "phtml"],
            Language::Prolog => &["pl", "plt", "pro", "prolog"],
            Language::CSharp => &["cs", "csx"],
            Language::TSql => &["sql"],
            Language::PowerShell => &["ps1", "psm1"],
            Language::Java => &["java"],
            Language::Kotlin => &["kt", "kts"],
            Language::MsBuild => &[
                "csproj", "fsproj", "vbproj", "props", "targets", "sln", "slnx",
            ],
        }
    }

    /// Get language name
    pub fn name(&self) -> &str {
        match self {
            Language::Rust => "rust",
            Language::Python => "python",
            Language::TypeScript | Language::Tsx => "typescript",
            Language::JavaScript => "javascript",
            Language::Go => "go",
            Language::Elixir => "elixir",
            Language::Ruby => "ruby",
            Language::Php => "php",
            Language::Prolog => "prolog",
            Language::CSharp => "csharp",
            Language::TSql => "tsql",
            Language::PowerShell => "powershell",
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::MsBuild => "msbuild",
        }
    }
}

/// Multi-language parser
pub struct Parser {
    rust_parser: TSParser,
    python_parser: TSParser,
    typescript_parser: TSParser,
    tsx_parser: TSParser,
    javascript_parser: TSParser,
    go_parser: TSParser,
    elixir_parser: TSParser,
    ruby_parser: TSParser,
    php_parser: TSParser,
    prolog_parser: TSParser,
    csharp_parser: TSParser,
    tsql_parser: TSParser,
    powershell_parser: TSParser,
    java_parser: TSParser,
    kotlin_parser: TSParser,
}

impl Parser {
    /// Create a new parser with all supported languages
    pub fn new() -> Result<Self> {
        let mut rust_parser = TSParser::new();
        rust_parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Rust language: {}", e)))?;

        let mut python_parser = TSParser::new();
        python_parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Python language: {}", e)))?;

        let mut typescript_parser = TSParser::new();
        typescript_parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .map_err(|e| Error::Parse(format!("Failed to set TypeScript language: {}", e)))?;

        let mut tsx_parser = TSParser::new();
        tsx_parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
            .map_err(|e| Error::Parse(format!("Failed to set TSX language: {}", e)))?;

        let mut javascript_parser = TSParser::new();
        javascript_parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set JavaScript language: {}", e)))?;

        let mut go_parser = TSParser::new();
        go_parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Go language: {}", e)))?;

        let mut elixir_parser = TSParser::new();
        elixir_parser
            .set_language(&tree_sitter_elixir::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Elixir language: {}", e)))?;

        let mut ruby_parser = TSParser::new();
        ruby_parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Ruby language: {}", e)))?;

        let mut php_parser = TSParser::new();
        php_parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .map_err(|e| Error::Parse(format!("Failed to set PHP language: {}", e)))?;

        let mut prolog_parser = TSParser::new();
        prolog_parser
            .set_language(&tree_sitter_prolog::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Prolog language: {}", e)))?;

        let mut csharp_parser = TSParser::new();
        csharp_parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set C# language: {}", e)))?;

        let mut tsql_parser = TSParser::new();
        tsql_parser
            .set_language(&tree_sitter_tsql::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set T-SQL language: {}", e)))?;

        let mut powershell_parser = TSParser::new();
        powershell_parser
            .set_language(&tree_sitter_powershell::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set PowerShell language: {}", e)))?;

        let mut java_parser = TSParser::new();
        java_parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Java language: {}", e)))?;

        let mut kotlin_parser = TSParser::new();
        kotlin_parser
            .set_language(&tree_sitter_kotlin_sg::LANGUAGE.into())
            .map_err(|e| Error::Parse(format!("Failed to set Kotlin language: {}", e)))?;

        Ok(Self {
            rust_parser,
            python_parser,
            typescript_parser,
            tsx_parser,
            javascript_parser,
            go_parser,
            elixir_parser,
            ruby_parser,
            php_parser,
            prolog_parser,
            csharp_parser,
            tsql_parser,
            powershell_parser,
            java_parser,
            kotlin_parser,
        })
    }

    /// Parse code into a tree-sitter AST
    pub fn parse(&mut self, code: &str, language: Language) -> Result<Tree> {
        let parser = match language {
            Language::Rust => &mut self.rust_parser,
            Language::Python => &mut self.python_parser,
            Language::TypeScript => &mut self.typescript_parser,
            Language::Tsx => &mut self.tsx_parser,
            Language::JavaScript => &mut self.javascript_parser,
            Language::Go => &mut self.go_parser,
            Language::Elixir => &mut self.elixir_parser,
            Language::Ruby => &mut self.ruby_parser,
            Language::Php => &mut self.php_parser,
            Language::Prolog => &mut self.prolog_parser,
            Language::CSharp => &mut self.csharp_parser,
            Language::TSql => &mut self.tsql_parser,
            Language::PowerShell => &mut self.powershell_parser,
            Language::Java => &mut self.java_parser,
            Language::Kotlin => &mut self.kotlin_parser,
            Language::MsBuild => {
                return Err(Error::Parse(
                    "project files are read as XML, not parsed with tree-sitter".to_string(),
                ))
            }
        };

        parser
            .parse(code, None)
            .ok_or_else(|| Error::Parse("Failed to parse code".to_string()))
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new().expect("Failed to create parser")
    }
}

/// Get node text
pub fn node_text<'a>(node: &Node<'_>, code: &'a [u8]) -> &'a str {
    node.utf8_text(code).unwrap_or("")
}

/// Get node line number (1-indexed)
pub fn node_line(node: &Node<'_>) -> usize {
    node.start_position().row + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::Tsx));
        assert_eq!(Language::from_extension("jsx"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
        assert_eq!(Language::from_extension("ex"), Some(Language::Elixir));
        assert_eq!(Language::from_extension("exs"), Some(Language::Elixir));
        assert_eq!(Language::from_extension("unknown"), None);
    }

    #[test]
    fn test_parser_creation() {
        let parser = Parser::new();
        assert!(parser.is_ok());
    }

    #[test]
    fn test_parse_python() {
        let mut parser = Parser::new().unwrap();
        let code = "def hello():\n    print('hello')";
        let tree = parser.parse(code, Language::Python);
        assert!(tree.is_ok());
        assert!(!tree.unwrap().root_node().has_error());
    }

    #[test]
    fn test_parse_rust() {
        let mut parser = Parser::new().unwrap();
        let code = "fn main() { println!(\"hello\"); }";
        let tree = parser.parse(code, Language::Rust);
        assert!(tree.is_ok());
        assert!(!tree.unwrap().root_node().has_error());
    }

    #[test]
    fn test_parse_tsx() {
        let mut parser = Parser::new().unwrap();
        let code = "const App = () => <div>Hello</div>;";
        let tree = parser.parse(code, Language::Tsx);
        assert!(tree.is_ok());
        assert!(!tree.unwrap().root_node().has_error());
    }

    #[test]
    fn test_parse_elixir() {
        let mut parser = Parser::new().unwrap();
        let code = "defmodule Hello do\n  def greet, do: :ok\nend";
        let tree = parser.parse(code, Language::Elixir);
        assert!(tree.is_ok());
    }

    #[test]
    fn test_parse_ruby() {
        let mut parser = Parser::new().unwrap();
        let code = "class Greeter\n  def hello(name)\n    puts \"Hello #{name}\"\n  end\nend";
        let tree = parser.parse(code, Language::Ruby);
        assert!(tree.is_ok());
        assert!(!tree.unwrap().root_node().has_error());
    }

    #[test]
    fn test_parse_prolog() {
        let mut parser = Parser::new().unwrap();
        let code = "room_connected(A, B) :- relation(A, connects_to, B).";
        let tree = parser.parse(code, Language::Prolog);
        assert!(tree.is_ok());
        assert!(!tree.unwrap().root_node().has_error());
    }

    #[test]
    fn test_language_prolog_from_extension() {
        assert_eq!(Language::from_extension("pl"), Some(Language::Prolog));
        assert_eq!(Language::from_extension("plt"), Some(Language::Prolog));
        assert_eq!(Language::from_extension("prolog"), Some(Language::Prolog));
    }

    #[test]
    fn test_language_ruby_from_extension() {
        assert_eq!(Language::from_extension("rb"), Some(Language::Ruby));
        assert_eq!(Language::from_extension("rake"), Some(Language::Ruby));
        assert_eq!(Language::from_extension("gemspec"), Some(Language::Ruby));
    }

    #[test]
    fn test_language_php_from_extension() {
        assert_eq!(Language::from_extension("php"), Some(Language::Php));
        assert_eq!(Language::from_extension("phtml"), Some(Language::Php));
    }

    #[test]
    fn test_parse_php() {
        let mut parser = Parser::new().unwrap();
        let code = "<?php\nfunction greet($name) {\n    return \"Hello $name\";\n}\n";
        let tree = parser.parse(code, Language::Php);
        assert!(tree.is_ok());
        assert!(!tree.unwrap().root_node().has_error());
    }

    #[test]
    fn test_parse_php_html_intermixed() {
        let mut parser = Parser::new().unwrap();
        let code = "<html><body><?php echo wp_title(); ?></body></html>";
        let tree = parser.parse(code, Language::Php);
        assert!(tree.is_ok());
        assert!(!tree.unwrap().root_node().has_error());
    }
}
