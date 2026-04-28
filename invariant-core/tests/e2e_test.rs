//! End-to-end test: verifies the full extraction pipeline for each language
//! produces well-formed facts with correct structure.

use invariant_core::{Analyzer, Language};

fn facts_by_predicate<'a>(facts: &'a [String], pred: &str) -> Vec<&'a String> {
    facts.iter().filter(|f| f.starts_with(pred)).collect()
}

#[test]
fn e2e_python_calculator() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
import math
from decimal import Decimal

class Calculator:
    def __init__(self):
        self.history = []

    def add(self, a, b):
        result = a + b
        self._record(result)
        return result

    def multiply(self, a, b):
        result = a * b
        self._record(result)
        return result

    def _record(self, value):
        self.history.append(value)

    def __repr__(self):
        return f"Calculator(history={self.history})"

def compute_tax(amount, rate):
    base = Decimal(str(amount))
    return float(base * Decimal(str(rate)))

def unused_helper():
    pass
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "src/calculator.py", "abc123")
        .unwrap();

    assert!(
        result.summary.modules >= 2,
        "file module + Calculator class"
    );
    assert!(result.summary.functions >= 7, "6 methods + 2 functions");
    assert!(result.summary.loc > 0);

    let modules = facts_by_predicate(&result.facts, "module(");
    assert!(
        modules.len() >= 2,
        "expected file + class modules, got {}",
        modules.len()
    );

    let functions = facts_by_predicate(&result.facts, "function(");
    assert!(
        functions.len() >= 7,
        "expected 7+ function facts, got {}",
        functions.len()
    );

    let record_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'_record'"))
        .unwrap();
    assert!(
        record_fact.contains("protected"),
        "_record should be protected"
    );

    let add_fact = result.facts.iter().find(|f| f.contains("'add'")).unwrap();
    assert!(add_fact.contains("public"), "add should be public");

    let repr_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'__repr__'"))
        .unwrap();
    assert!(
        repr_fact.contains("public"),
        "__repr__ (dunder) should be public"
    );

    let deps = facts_by_predicate(&result.facts, "depends_on(");
    assert!(deps.len() >= 2, "math + decimal imports expected");

    let calls = facts_by_predicate(&result.facts, "calls_external(");
    assert!(!calls.is_empty(), "should detect function calls");

    for fact in &result.facts {
        assert!(fact.ends_with('.'), "malformed fact: {}", fact);
    }

    let sha_facts = result
        .facts
        .iter()
        .filter(|f| f.contains("'abc123'"))
        .count();
    assert!(sha_facts > 0, "commit SHA should appear in facts");
}

#[test]
fn e2e_rust_server() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
use std::collections::HashMap;
use std::sync::Arc;

pub struct AppState {
    db: HashMap<String, String>,
}

impl AppState {
    pub fn new() -> Self {
        Self { db: HashMap::new() }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.db.get(key)
    }

    fn validate_key(&self, key: &str) -> bool {
        !key.is_empty()
    }
}

pub fn handle(key: &str) -> String {
    format!("handled: {}", key)
}
"#;
    let result = analyzer
        .lens_code(code, Language::Rust, "src/server.rs", "def456")
        .unwrap();

    assert!(
        result.summary.functions >= 4,
        "new, get, validate_key, handle"
    );

    let validate_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'validate_key'"))
        .unwrap();
    assert!(
        validate_fact.contains("private"),
        "validate_key should be private"
    );

    let handle_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'handle'"))
        .unwrap();
    assert!(
        handle_fact.contains("public"),
        "pub fn handle should be public"
    );

    let deps = facts_by_predicate(&result.facts, "depends_on(");
    assert!(deps.len() >= 2, "HashMap + Arc use decls expected");

    assert!(
        result.facts.iter().any(|f| f.contains("'AppState'")),
        "AppState struct should appear"
    );
}

#[test]
fn e2e_typescript_api() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
import { Request, Response } from 'express';
import { UserService } from './services';

class ApiController {
    async getUser(req: Request, res: Response): Promise<void> {
        const user = await this.userService.findById(req.params.id);
        res.json(user);
    }
}

function formatResponse(data: any): string {
    return JSON.stringify({ success: true, data });
}
"#;
    let result = analyzer
        .lens_code(code, Language::TypeScript, "lib/api.ts", "ts_sha")
        .unwrap();

    assert!(result.summary.functions >= 2, "getUser + formatResponse");
    assert!(
        result.facts.iter().any(|f| f.contains("'ApiController'")),
        "class should be extracted"
    );
    assert!(result.summary.dependencies >= 2, "express + ./services");
}

#[test]
fn e2e_go_handler() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
package handler

import (
    "encoding/json"
    "fmt"
    "net/http"
)

func NewHandler() *Handler {
    return &Handler{}
}

func (h *Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {}

func unusedUtility(x int) int {
    return x * 2
}
"#;
    let result = analyzer
        .lens_code(code, Language::Go, "handler.go", "go_sha")
        .unwrap();

    assert!(result.summary.functions >= 3);

    let new_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'NewHandler'"))
        .unwrap();
    assert!(
        new_fact.contains("public"),
        "NewHandler should be public (uppercase)"
    );

    let unused_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'unusedUtility'"))
        .unwrap();
    assert!(
        unused_fact.contains("private"),
        "unusedUtility should be private (lowercase)"
    );

    assert!(result.summary.dependencies >= 3, "json, fmt, net/http");
}

#[test]
fn e2e_ruby_service() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
require 'json'
require_relative 'base_service'

module MyApp
  class UserService
    include Enumerable

    def initialize(repo)
      @repo = repo
    end

    def find(id)
      @repo.get(id)
    end

    def list_all
      @repo.all
    end

    private

    def validate(attrs)
      attrs.key?(:name) && attrs.key?(:email)
    end

    def normalize(attrs)
      attrs.transform_keys(&:to_s)
    end
  end
end
"#;
    let result = analyzer
        .lens_code(code, Language::Ruby, "lib/my_app/user_service.rb", "rb_sha")
        .unwrap();

    assert!(
        result.summary.functions >= 5,
        "initialize, find, list_all, validate, normalize expected, got {}",
        result.summary.functions
    );

    // Private methods should be detected
    let validate_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'validate'") && f.starts_with("function("));
    assert!(validate_fact.is_some(), "validate should be extracted");
    assert!(
        validate_fact.unwrap().contains("private"),
        "validate should be private (after `private` block)"
    );

    // Public methods should be detected
    let find_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'find'") && f.starts_with("function("));
    assert!(find_fact.is_some(), "find should be extracted");
    assert!(
        find_fact.unwrap().contains("public"),
        "find should be public"
    );

    // Dependencies
    let deps = facts_by_predicate(&result.facts, "depends_on(");
    assert!(
        deps.len() >= 2,
        "require 'json' + require_relative expected"
    );

    // Module/class structure
    assert!(
        result.facts.iter().any(|f| f.contains("'UserService'")),
        "UserService class should appear"
    );

    // Well-formed facts
    for fact in &result.facts {
        assert!(fact.ends_with('.'), "malformed fact: {}", fact);
    }
}

#[test]
fn e2e_cross_language_consistency() {
    let mut analyzer = Analyzer::new().unwrap();

    let py = analyzer
        .lens_code("def hello(): pass", Language::Python, "h.py", "sha")
        .unwrap();
    let rs = analyzer
        .lens_code("pub fn hello() {}", Language::Rust, "h.rs", "sha")
        .unwrap();
    let ts = analyzer
        .lens_code("function hello() {}", Language::TypeScript, "h.ts", "sha")
        .unwrap();
    let go = analyzer
        .lens_code("package m\nfunc Hello() {}", Language::Go, "h.go", "sha")
        .unwrap();
    let rb = analyzer
        .lens_code("def hello; end", Language::Ruby, "h.rb", "sha")
        .unwrap();
    let ex = analyzer
        .lens_code(
            "defmodule M do\n  def hello, do: :ok\nend",
            Language::Elixir,
            "h.ex",
            "sha",
        )
        .unwrap();

    for (lang, result) in [
        ("py", &py),
        ("rs", &rs),
        ("ts", &ts),
        ("go", &go),
        ("rb", &rb),
        ("ex", &ex),
    ] {
        let func_facts = facts_by_predicate(&result.facts, "function(");
        assert!(
            !func_facts.is_empty(),
            "{} should produce function facts",
            lang
        );

        let f = func_facts[0];
        let comma_count = f.matches(',').count();
        assert_eq!(
            comma_count, 6,
            "{}: function fact should have 7 args (6 commas), got {} in: {}",
            lang, comma_count, f
        );
    }
}

#[test]
fn e2e_elixir_module() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
defmodule MyApp.Users do
  import Ecto.Query
  alias MyApp.Repo

  def list_users do
    Repo.all(User)
  end

  def get_user(id) do
    Repo.get(User, id)
  end

  defp changeset(user, attrs) do
    user
    |> cast(attrs, [:name, :email])
    |> validate_required([:name, :email])
  end
end
"#;
    let result = analyzer
        .lens_code(code, Language::Elixir, "lib/my_app/users.ex", "elx_sha")
        .unwrap();

    assert!(
        result.summary.functions >= 3,
        "list_users, get_user, changeset expected"
    );

    let changeset_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'changeset'") && f.starts_with("function("));
    assert!(changeset_fact.is_some(), "changeset should be extracted");
    assert!(
        changeset_fact.unwrap().contains("private"),
        "defp changeset should be private"
    );

    let list_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'list_users'") && f.starts_with("function("));
    assert!(list_fact.is_some(), "list_users should be extracted");
    assert!(
        list_fact.unwrap().contains("public"),
        "def list_users should be public"
    );

    let deps = facts_by_predicate(&result.facts, "depends_on(");
    assert!(deps.len() >= 2, "import + alias expected");

    for fact in &result.facts {
        assert!(fact.ends_with('.'), "malformed fact: {}", fact);
    }
}
