//! Comprehensive integration tests for the multi-language code analyzer.
//!
//! Each language section tests:
//!   - Function extraction (name, arity, visibility, line number)
//!   - Module / class extraction
//!   - Import / dependency extraction
//!   - Call-site extraction
//!   - Prolog fact string format
//!   - Summary statistics

use invariant_core::{Analyzer, Language};

// ─── helpers ─────────────────────────────────────────────────────────────────

fn has_fact(facts: &[String], substr: &str) -> bool {
    facts.iter().any(|f| f.contains(substr))
}

// ─── Python ──────────────────────────────────────────────────────────────────

#[test]
fn test_python_basic_functions() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
def hello():
    pass

def add(x, y):
    return x + y
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "src/math.py", "abc123")
        .unwrap();

    assert_eq!(result.summary.functions, 2, "expected 2 functions");
    assert!(!result.facts.is_empty());
    assert!(has_fact(&result.facts, "'hello'"), "missing hello fact");
    assert!(has_fact(&result.facts, "'add'"), "missing add fact");
}

#[test]
fn test_python_function_arity() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
def zero():
    pass

def three(a, b, c):
    pass
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "test.py", "sha")
        .unwrap();

    let has_zero = result
        .facts
        .iter()
        .any(|f| f.contains("'zero'") && f.contains(", 0,"));
    let has_three = result
        .facts
        .iter()
        .any(|f| f.contains("'three'") && f.contains(", 3,"));
    assert!(has_zero, "missing zero/0 arity fact");
    assert!(has_three, "missing three/3 arity fact");
}

#[test]
fn test_python_visibility() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
def public_func():
    pass

def _protected_func():
    pass

def __private_func():
    pass

def __dunder__(self):
    pass
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "vis.py", "sha")
        .unwrap();

    let has_public = result
        .facts
        .iter()
        .any(|f| f.contains("'public_func'") && f.contains("public"));
    let has_protected = result
        .facts
        .iter()
        .any(|f| f.contains("'_protected_func'") && f.contains("protected"));
    let has_private = result
        .facts
        .iter()
        .any(|f| f.contains("'__private_func'") && f.contains("private"));
    let has_dunder = result
        .facts
        .iter()
        .any(|f| f.contains("'__dunder__'") && f.contains("public"));

    assert!(has_public, "public_func should be public");
    assert!(has_protected, "_protected_func should be protected");
    assert!(has_private, "__private_func should be private");
    assert!(
        has_dunder,
        "__dunder__ is a magic method and should be public"
    );
}

#[test]
fn test_python_class_extracted_as_module() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
class Greeter:
    def greet(self, name):
        return f"Hello, {name}"

    def farewell(self):
        pass
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "greet.py", "sha")
        .unwrap();

    assert!(
        result.summary.modules >= 2,
        "should have at least 2 modules (file + class)"
    );
    assert!(
        has_fact(&result.facts, "'Greeter'"),
        "Greeter class should appear as module"
    );
    assert!(
        has_fact(&result.facts, "'greet'"),
        "greet method should be extracted"
    );
    assert!(
        has_fact(&result.facts, "'farewell'"),
        "farewell method should be extracted"
    );
}

#[test]
fn test_python_imports() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
import os
import sys
from pathlib import Path
from typing import Optional, List
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "imports.py", "sha")
        .unwrap();

    assert!(has_fact(&result.facts, "'os'"), "os import not found");
    assert!(has_fact(&result.facts, "'sys'"), "sys import not found");
    assert!(
        has_fact(&result.facts, "'pathlib'"),
        "pathlib import_from not found"
    );
}

#[test]
fn test_python_function_calls() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
def process(data):
    result = transform(data)
    save(result)
    return result
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "proc.py", "sha")
        .unwrap();

    assert!(result.summary.calls >= 2, "should detect at least 2 calls");
    assert!(
        has_fact(&result.facts, "calls_external"),
        "calls_external facts expected"
    );
}

#[test]
fn test_python_module_fact_format() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = "def f():\n    pass\n";
    let result = analyzer
        .lens_code(code, Language::Python, "pkg/sub/mod.py", "sha1")
        .unwrap();

    let module_fact = result
        .facts
        .iter()
        .find(|f| f.starts_with("module("))
        .unwrap();
    assert!(
        module_fact.contains("'sha1'"),
        "commit sha should be in module fact"
    );
    assert!(module_fact.contains("'pkg"), "module path not in fact");
}

#[test]
fn test_python_loc_counted() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = "def f():\n    pass\n\ndef g():\n    pass\n";
    let result = analyzer
        .lens_code(code, Language::Python, "f.py", "sha")
        .unwrap();
    assert!(result.summary.loc > 0);
}

// ─── Rust ────────────────────────────────────────────────────────────────────

#[test]
fn test_rust_functions() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
pub fn greet(name: &str) -> String {
    format!("Hello, {}", name)
}

fn helper() {
    println!("helper");
}
"#;
    let result = analyzer
        .lens_code(code, Language::Rust, "src/lib.rs", "abc")
        .unwrap();

    assert_eq!(result.summary.functions, 2);
    assert!(has_fact(&result.facts, "'greet'"), "greet not found");
    assert!(has_fact(&result.facts, "'helper'"), "helper not found");
}

#[test]
fn test_rust_visibility() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
pub fn public_fn() {}
fn private_fn() {}
pub(crate) fn crate_fn() {}
"#;
    let result = analyzer
        .lens_code(code, Language::Rust, "vis.rs", "sha")
        .unwrap();

    let has_public = result
        .facts
        .iter()
        .any(|f| f.contains("'public_fn'") && f.contains("public"));
    let has_private = result
        .facts
        .iter()
        .any(|f| f.contains("'private_fn'") && f.contains("private"));
    let has_crate = result
        .facts
        .iter()
        .any(|f| f.contains("'crate_fn'") && f.contains("public"));

    assert!(has_public, "public_fn should be public");
    assert!(has_private, "private_fn should be private");
    assert!(has_crate, "pub(crate) should be public");
}

#[test]
fn test_rust_impl_methods() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
struct Counter {
    count: u32,
}

impl Counter {
    pub fn new() -> Self {
        Self { count: 0 }
    }

    pub fn increment(&mut self) {
        self.count += 1;
    }

    fn reset(&mut self) {
        self.count = 0;
    }
}
"#;
    let result = analyzer
        .lens_code(code, Language::Rust, "counter.rs", "sha")
        .unwrap();

    assert!(
        result.summary.functions >= 3,
        "new, increment, reset expected"
    );
    assert!(has_fact(&result.facts, "'new'"), "new not found");
    assert!(
        has_fact(&result.facts, "'increment'"),
        "increment not found"
    );
    assert!(has_fact(&result.facts, "'reset'"), "reset not found");
}

#[test]
fn test_rust_use_declarations() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
"#;
    let result = analyzer
        .lens_code(code, Language::Rust, "deps.rs", "sha")
        .unwrap();

    assert!(result.summary.dependencies >= 3, "expected 3 dependencies");
    assert!(
        has_fact(&result.facts, "depends_on"),
        "depends_on facts expected"
    );
}

#[test]
fn test_rust_structs_and_enums() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
pub struct Point { x: f64, y: f64 }
pub enum Color { Red, Green, Blue }
pub trait Drawable { fn draw(&self); }
"#;
    let result = analyzer
        .lens_code(code, Language::Rust, "types.rs", "sha")
        .unwrap();

    assert!(has_fact(&result.facts, "'Point'"), "Point struct not found");
    assert!(has_fact(&result.facts, "'Color'"), "Color enum not found");
    assert!(
        has_fact(&result.facts, "'Drawable'"),
        "Drawable trait not found"
    );
}

#[test]
fn test_rust_function_calls() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
fn run() {
    println!("start");
    do_work();
    cleanup();
}

fn do_work() {}
fn cleanup() {}
"#;
    let result = analyzer
        .lens_code(code, Language::Rust, "run.rs", "sha")
        .unwrap();

    assert!(result.summary.calls >= 2, "expected at least 2 calls");
}

// ─── TypeScript / JavaScript ─────────────────────────────────────────────────

#[test]
fn test_typescript_functions() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
function greet(name: string): string {
    return `Hello, ${name}!`;
}

function add(a: number, b: number): number {
    return a + b;
}
"#;
    let result = analyzer
        .lens_code(code, Language::TypeScript, "greet.ts", "sha")
        .unwrap();

    assert!(result.summary.functions >= 2);
    assert!(has_fact(&result.facts, "'greet'"), "greet not found");
    assert!(has_fact(&result.facts, "'add'"), "add not found");
}

#[test]
fn test_typescript_class() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
class UserService {
    private db: Database;

    constructor(db: Database) {
        this.db = db;
    }

    async findUser(id: string): Promise<User> {
        return this.db.find(id);
    }
}
"#;
    let result = analyzer
        .lens_code(code, Language::TypeScript, "user_service.ts", "sha")
        .unwrap();

    assert!(
        has_fact(&result.facts, "'UserService'"),
        "UserService class not found"
    );
}

#[test]
fn test_typescript_imports() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
import { useState, useEffect } from 'react';
import axios from 'axios';
import type { User } from './types';
"#;
    let result = analyzer
        .lens_code(code, Language::TypeScript, "component.ts", "sha")
        .unwrap();

    assert!(result.summary.dependencies >= 3, "expected 3 imports");
    assert!(has_fact(&result.facts, "'react'"), "react import not found");
    assert!(has_fact(&result.facts, "'axios'"), "axios import not found");
}

#[test]
fn test_javascript_function() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
function doSomething(x) {
    return x * 2;
}
"#;
    let result = analyzer
        .lens_code(code, Language::JavaScript, "util.js", "sha")
        .unwrap();

    assert!(result.summary.functions >= 1);
    assert!(
        has_fact(&result.facts, "'doSomething'"),
        "doSomething not found"
    );
}

#[test]
fn test_javascript_class() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
class Animal {
    constructor(name) {
        this.name = name;
    }
    speak() {
        console.log(this.name + ' makes a sound.');
    }
}
"#;
    let result = analyzer
        .lens_code(code, Language::JavaScript, "animal.js", "sha")
        .unwrap();

    assert!(
        has_fact(&result.facts, "'Animal'"),
        "Animal class not found"
    );
}

// ─── Go ──────────────────────────────────────────────────────────────────────

#[test]
fn test_go_functions() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
package main

func Hello() string {
    return "Hello, World!"
}

func privateHelper(n int) int {
    return n + 1
}
"#;
    let result = analyzer
        .lens_code(code, Language::Go, "main.go", "sha")
        .unwrap();

    assert!(result.summary.functions >= 2);
    assert!(has_fact(&result.facts, "'Hello'"), "Hello not found");
    assert!(
        has_fact(&result.facts, "'privateHelper'"),
        "privateHelper not found"
    );
}

#[test]
fn test_go_visibility() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
package main

func Exported() {}
func unexported() {}
"#;
    let result = analyzer
        .lens_code(code, Language::Go, "vis.go", "sha")
        .unwrap();

    let has_public = result
        .facts
        .iter()
        .any(|f| f.contains("'Exported'") && f.contains("public"));
    let has_private = result
        .facts
        .iter()
        .any(|f| f.contains("'unexported'") && f.contains("private"));

    assert!(has_public, "Exported should be public");
    assert!(has_private, "unexported should be private");
}

#[test]
fn test_go_imports() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
package main

import (
    "fmt"
    "os"
    "github.com/some/pkg"
)

func main() {}
"#;
    let result = analyzer
        .lens_code(code, Language::Go, "main.go", "sha")
        .unwrap();

    assert!(result.summary.dependencies >= 3, "expected 3 imports");
    assert!(
        has_fact(&result.facts, "\"fmt\"") || has_fact(&result.facts, "'fmt'"),
        "fmt import not found"
    );
}

#[test]
fn test_go_method_declaration() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
package main

type Point struct { X, Y float64 }

func (p Point) Distance() float64 {
    return 0.0
}

func (p *Point) Scale(factor float64) {
    p.X *= factor
    p.Y *= factor
}
"#;
    let result = analyzer
        .lens_code(code, Language::Go, "point.go", "sha")
        .unwrap();

    assert!(result.summary.functions >= 2, "Distance and Scale expected");
}

// ─── Cross-language / general ─────────────────────────────────────────────────

#[test]
fn test_checksum_deterministic() {
    let analyzer = Analyzer::new().unwrap();
    let code = "def f(): pass";
    let c1 = analyzer.compute_checksum(code);
    let c2 = analyzer.compute_checksum(code);
    assert_eq!(c1, c2);
    assert_eq!(c1.len(), 64, "SHA-256 hex should be 64 chars");
}

#[test]
fn test_checksum_differs_for_different_code() {
    let analyzer = Analyzer::new().unwrap();
    let c1 = analyzer.compute_checksum("def f(): pass");
    let c2 = analyzer.compute_checksum("def g(): pass");
    assert_ne!(c1, c2);
}

#[test]
fn test_all_facts_end_with_period() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
def hello():
    pass
import os
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "f.py", "sha")
        .unwrap();

    for fact in &result.facts {
        assert!(fact.ends_with('.'), "Fact must end with '.': {}", fact);
    }
}

#[test]
fn test_facts_contain_commit_sha() {
    let mut analyzer = Analyzer::new().unwrap();
    let sha = "deadbeef1234";
    let result = analyzer
        .lens_code("def f(): pass", Language::Python, "f.py", sha)
        .unwrap();

    let has_sha = result.facts.iter().any(|f| f.contains(sha));
    assert!(has_sha, "At least one fact should contain the commit SHA");
}

#[test]
fn test_language_from_extension() {
    assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
    assert_eq!(Language::from_extension("py"), Some(Language::Python));
    assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension("tsx"), Some(Language::Tsx));
    assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
    assert_eq!(Language::from_extension("jsx"), Some(Language::JavaScript));
    assert_eq!(Language::from_extension("go"), Some(Language::Go));
    assert_eq!(Language::from_extension("ex"), Some(Language::Elixir));
    assert_eq!(Language::from_extension("exs"), Some(Language::Elixir));
    assert_eq!(Language::from_extension("unknown"), None);
}

#[test]
fn test_empty_code_produces_module_fact() {
    let mut analyzer = Analyzer::new().unwrap();
    let result = analyzer
        .lens_code("", Language::Python, "empty.py", "sha")
        .unwrap();

    assert!(
        !result.facts.is_empty(),
        "empty code should still produce module fact"
    );
    assert!(result.summary.functions == 0);
}

#[test]
fn test_nested_functions_python() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
def outer(x):
    def inner(y):
        return x + y
    return inner(x)
"#;
    let result = analyzer
        .lens_code(code, Language::Python, "nested.py", "sha")
        .unwrap();

    assert!(result.summary.functions >= 1, "outer should be found");
    assert!(
        has_fact(&result.facts, "'outer'"),
        "outer function fact expected"
    );
}

// ─── Elixir ──────────────────────────────────────────────────────────────────

#[test]
fn test_elixir_basic_module() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
defmodule MyApp.Greeter do
  def hello(name) do
    "Hello, #{name}!"
  end

  defp validate(name) do
    String.length(name) > 0
  end
end
"#;
    let result = analyzer
        .lens_code(code, Language::Elixir, "lib/greeter.ex", "abc123")
        .unwrap();

    assert!(result.summary.functions >= 2, "hello and validate expected");
    assert!(has_fact(&result.facts, "'hello'"), "hello not found");
    assert!(has_fact(&result.facts, "'validate'"), "validate not found");

    let hello_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'hello'") && f.starts_with("function("));
    assert!(hello_fact.is_some(), "hello function fact expected");
    assert!(
        hello_fact.unwrap().contains("public"),
        "def hello should be public"
    );

    let validate_fact = result
        .facts
        .iter()
        .find(|f| f.contains("'validate'") && f.starts_with("function("));
    assert!(validate_fact.is_some(), "validate function fact expected");
    assert!(
        validate_fact.unwrap().contains("private"),
        "defp validate should be private"
    );
}

#[test]
fn test_elixir_imports() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
defmodule MyApp.Worker do
  import Enum
  use GenServer
  alias MyApp.Repo

  def start_link(opts) do
    GenServer.start_link(__MODULE__, opts)
  end
end
"#;
    let result = analyzer
        .lens_code(code, Language::Elixir, "lib/worker.ex", "sha")
        .unwrap();

    assert!(
        has_fact(&result.facts, "depends_on"),
        "dependency facts expected"
    );
    assert!(has_fact(&result.facts, "Enum"), "import Enum expected");
    assert!(
        has_fact(&result.facts, "GenServer"),
        "use GenServer expected"
    );
}

#[test]
fn test_tsx_parsing() {
    let mut analyzer = Analyzer::new().unwrap();
    let code = r#"
import React from 'react';

function App(): JSX.Element {
    return <div>Hello World</div>;
}
"#;
    let result = analyzer
        .lens_code(code, Language::Tsx, "App.tsx", "sha")
        .unwrap();

    assert!(result.summary.functions >= 1);
    assert!(has_fact(&result.facts, "'App'"), "App component not found");
}

// ─── Calibration fixes (testbed, 2026-11-09) ─────────────────────────────────

fn lens(code: &str, lang: Language, path: &str) -> Vec<String> {
    let mut analyzer = Analyzer::new().unwrap();
    analyzer.lens_code(code, lang, path, "sha").unwrap().facts
}

fn count_prefix(facts: &[String], prefix: &str) -> usize {
    facts.iter().filter(|f| f.starts_with(prefix)).count()
}

#[test]
fn tests_are_marked_as_tests() {
    let rust = lens(
        "fn helper() {}\n#[test]\nfn checks_helper() { helper(); }\n",
        Language::Rust,
        "src/lib.rs",
    );
    assert!(has_fact(
        &rust,
        "test('src__lib__checks_helper_0', '', unit)."
    ));
    assert!(!has_fact(&rust, "test('src__lib__helper_0'"));

    let py = lens(
        "def test_add():\n    pass\n\ndef make():\n    pass\n",
        Language::Python,
        "tests/test_math.py",
    );
    assert!(has_fact(
        &py,
        "test('tests_test_math__test_add_0', '', unit)."
    ));
    assert!(has_fact(
        &py,
        "test('tests_test_math__make_0', '', test_helper)."
    ));

    let prod = lens(
        "def test_mode():\n    pass\n",
        Language::Python,
        "app/config.py",
    );
    assert_eq!(
        count_prefix(&prod, "test("),
        0,
        "a test-like name outside test files is not a test"
    );
}

#[test]
fn rust_inline_modules_and_trait_defaults_are_read() {
    let facts = lens(
        r#"
use regex_syntax::hir::{self, Hir};
trait Greet { fn name(&self) -> String; fn hello(&self) -> String { self.name() } }
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() { assert!(true); }
}
"#,
        Language::Rust,
        "src/greet.rs",
    );
    assert!(has_fact(&facts, "'hello', 1"), "trait default method");
    assert!(
        !has_fact(&facts, "'name', 1, public"),
        "a required method has no body"
    );
    assert!(
        has_fact(&facts, "'src__greet__tests__it_works_0'"),
        "inline mod item"
    );
    assert!(has_fact(
        &facts,
        "depends_on('src__greet', 'regex_syntax.hir', use"
    ));
    assert!(
        !facts.iter().any(|f| f.contains("hir.{")),
        "grouped use is not split on spaces"
    );
    assert!(
        !has_fact(&facts, "'tests', mod"),
        "an inline mod is not a dependency"
    );
    assert!(has_fact(
        &facts,
        "depends_on('src__greet', 'regex_syntax.hir.Hir', use"
    ));
    assert!(has_fact(
        &facts,
        "test('src__greet__tests__it_works_0', '', unit)."
    ));
}

#[test]
fn go_methods_are_scoped_by_receiver_and_types_are_types() {
    let facts = lens(
        r#"
package render
type JSON struct{}
type IndentedJSON struct{}
func (r JSON) Render(w int) error { return nil }
func (r *IndentedJSON) Render(w int) error { return nil }
"#,
        Language::Go,
        "render/json.go",
    );
    assert!(has_fact(&facts, "'render_json__json_render_1'"));
    assert!(has_fact(&facts, "'render_json__indentedjson_render_1'"));
    assert!(!facts
        .iter()
        .any(|f| f.starts_with("depends_on") && f.contains("JSON")));
}

#[test]
fn elixir_guards_macros_and_clauses() {
    let facts = lens(
        r#"
defmodule Outer do
  defmodule Inner do
    def f(x) when is_integer(x), do: x
    def f(x), do: g(x)
    defmacro m(ast), do: ast
  end
end
"#,
        Language::Elixir,
        "lib/outer.ex",
    );
    assert_eq!(
        count_prefix(&facts, "function('outer_inner__f_1'"),
        1,
        "one function for two clauses"
    );
    assert!(
        has_fact(&facts, "'outer_inner__m_1'"),
        "defmacro is a function"
    );
    assert!(!facts.iter().any(|f| f.contains("when is_integer")));
    assert!(has_fact(
        &facts,
        "calls_external('outer_inner__f_1', 'unknown', 'g'"
    ));
}

#[test]
fn javascript_functions_take_their_binding_names() {
    let facts = lens(
        r#"
const add = (a, b) => a + b;
View.prototype.lookup = function lookup(name) { return name; };
const api = { fetchUser: async (id) => load(id) };
function outer() {
  function inner() { return helper(); }
  return [1, 2].map(x => compute(x)).concat(new Thing());
}
const lodash = require("lodash");
import "./styles.css";
"#,
        Language::JavaScript,
        "src/app.js",
    );
    assert!(has_fact(&facts, "'src_app__add_2'"));
    assert!(has_fact(&facts, "'src_app__lookup_1'"));
    assert!(has_fact(&facts, "'src_app__fetchuser_1'"));
    assert!(
        has_fact(&facts, "'src_app__outer_0__inner_0'"),
        "nested named function"
    );
    assert!(
        has_fact(
            &facts,
            "calls_external('src_app__outer_0', 'unknown', 'compute'"
        ),
        "an inline callback's calls are the outer function's"
    );
    assert!(
        has_fact(
            &facts,
            "calls_external('src_app__outer_0', 'unknown', 'Thing'"
        ),
        "new X calls X"
    );
    assert!(has_fact(
        &facts,
        "calls_external('src_app__outer_0__inner_0', 'unknown', 'helper'"
    ));
    assert!(!has_fact(
        &facts,
        "calls_external('src_app__outer_0', 'unknown', 'helper'"
    ));
    assert!(has_fact(&facts, "'lodash', require"));
    assert!(has_fact(&facts, "'./styles.css', import"));
    assert!(!facts.iter().any(|f| f.contains("'anonymous'")));
}

#[test]
fn typescript_namespaces_scope_their_functions() {
    let facts = lens(
        r#"
export namespace Range { export function from(x: number) { return x; } }
export namespace Position { export function from(x: number) { return x; } }
"#,
        Language::TypeScript,
        "src/converters.ts",
    );
    assert!(has_fact(&facts, "'src_converters__range__from_1'"));
    assert!(has_fact(&facts, "'src_converters__position__from_1'"));
}

#[test]
fn python_nested_functions_are_their_own() {
    let facts = lens(
        "def keep_lazy(f):\n    def decorator(g):\n        return wrap(g)\n    return decorator\n\ndef keep_lazy(f):\n    return f\n",
        Language::Python,
        "utils/functional.py",
    );
    assert!(has_fact(
        &facts,
        "'utils_functional__keep_lazy_1__decorator_1'"
    ));
    assert!(has_fact(
        &facts,
        "calls_external('utils_functional__keep_lazy_1__decorator_1', 'unknown', 'wrap'"
    ));
    assert!(!has_fact(
        &facts,
        "calls_external('utils_functional__keep_lazy_1', 'unknown', 'wrap'"
    ));
    assert!(
        has_fact(&facts, "'utils_functional__keep_lazy_1_l6'"),
        "a redefinition is a second function"
    );
}

#[test]
fn ruby_include_reads_constants_only_and_atoms_are_quoted() {
    let facts = lens(
        "class Cookies\n  include Comparable\n  include(Enumerable)\n  def check\n    expect(x).to include(\"foo=;\", \"bar\")\n  end\nend\n",
        Language::Ruby,
        "spec/cookies_spec.rb",
    );
    assert!(has_fact(&facts, "'Comparable', include"));
    assert!(has_fact(&facts, "'Enumerable', include"));
    assert!(
        !facts.iter().any(|f| f.contains("foo=;")),
        "an RSpec matcher is not a mixin"
    );
    for f in &facts {
        assert!(!f.contains(",,"), "malformed fact: {f}");
    }
}

#[test]
fn ruby_bare_calls_and_file_scoped_modules() {
    let facts = lens(
        "module Concern\n  module ClassMethods\n    def build(opts)\n      total = 1\n      [1].each { |x| x }\n      app\n      configure(total, opts)\n    end\n  end\nend\n",
        Language::Ruby,
        "lib/concern.rb",
    );
    let calls: Vec<&String> = facts
        .iter()
        .filter(|f| f.starts_with("calls_external("))
        .collect();
    let called = |n: &str| {
        calls
            .iter()
            .any(|f| f.contains(&format!("'unknown', '{n}'")))
    };
    assert!(called("app"), "a bare method name is a call: {calls:?}");
    assert!(called("configure"));
    assert!(!called("total"), "a local is not a call");
    assert!(!called("opts"), "a parameter is not a call");
    assert!(!called("x"), "a block parameter is not a call");
    assert!(
        has_fact(&facts, "module('lib_concern_rb__classmethods'"),
        "modules are scoped by file"
    );
}

// ─── C# ──────────────────────────────────────────────────────────────────────

const CSHARP_CONTROLLER: &str = r#"
using System;
using static System.Math;
using Json = Newtonsoft.Json;

namespace Acme.Orders;

[ApiController]
public partial class OrdersController : ControllerBase, IDisposable
{
    private readonly IRepo _repo;
    public OrdersController(IRepo repo) : base(repo) { _repo = repo; }

    [HttpGet("orders/{id}")]
    public async Task<Order> Get(int id) => await _repo.Find(id);
    public Order Get(string code) => _repo.ByCode(code);
    internal static int Count(List<int> xs) { var t = new Tally(); return xs.Sum(x => Helper.Twice(x)); }

    public string Name { get; set; }
    public int Total => Compute(1, 2);
    public int this[int i] { get { return Items[i]; } }
    public event EventHandler Changed;
    ~OrdersController() { Dispose(); }
    public static Money operator +(Money a, Money b) => Money.Add(a, b);
    void Local() { int Inner(int y) => y * 2; Inner(3); }

    class Nested { protected virtual void Run<T>(T x) { Console.WriteLine(x); } }
}

public interface IRepo { Task<Order> Find(int id); }
"#;

fn lens_cs(code: &str, path: &str) -> Vec<String> {
    Analyzer::new()
        .unwrap()
        .lens_code(code, Language::CSharp, path, "sha")
        .unwrap()
        .facts
}

#[test]
fn csharp_types_are_namespace_qualified_modules() {
    let facts = lens_cs(CSHARP_CONTROLLER, "src/Orders/OrdersController.cs");
    assert!(has_fact(&facts, "module('acme_orders_orderscontroller', 'Acme.Orders.OrdersController', 'class:Acme.Orders.OrdersController'"));
    assert!(has_fact(
        &facts,
        "module('acme_orders_orderscontroller_nested', 'Acme.Orders.OrdersController.Nested'"
    ));
    assert!(has_fact(
        &facts,
        "module('acme_orders_irepo', 'Acme.Orders.IRepo', 'interface:"
    ));
    assert!(has_fact(
        &facts,
        "depends_on('acme_orders_orderscontroller', 'ControllerBase', inherits"
    ));
    assert!(has_fact(
        &facts,
        "depends_on('src_orders_orderscontroller', 'System.Math', using"
    ));
    assert!(has_fact(
        &facts,
        "depends_on('src_orders_orderscontroller', 'Newtonsoft.Json', using"
    ));
}

#[test]
fn csharp_generic_and_plain_types_are_different_modules() {
    let plain = lens_cs(
        "namespace N { public class Result { public Result(bool ok) { } } }",
        "Result.cs",
    );
    let generic = lens_cs("namespace N { public class Result<T> : Result { public Result(T value) : base(true) { } } }", "ResultOfT.cs");
    assert!(has_fact(
        &plain,
        "function('n_result__result_1', 'n_result', 'Result'"
    ));
    assert!(has_fact(
        &generic,
        "module('n_result_t_', 'N.Result<T>', 'class:N.Result<T>'"
    ));
    assert!(has_fact(
        &generic,
        "function('n_result_t___result_1', 'n_result_t_', 'Result'"
    ));
    assert!(has_fact(
        &generic,
        "calls_external('n_result_t___result_1', 'unknown', 'Result'"
    ));
}

#[test]
fn csharp_partial_class_parts_share_one_module() {
    let a = lens_cs(
        "namespace N { public partial class P { public void A() { B(); } } }",
        "a/P.cs",
    );
    let b = lens_cs(
        "namespace N { public partial class P { public void B() { } } }",
        "b/P.Part.cs",
    );
    assert!(has_fact(&a, "function('n_p__a_0', 'n_p', 'A'"));
    assert!(has_fact(&b, "function('n_p__b_0', 'n_p', 'B'"));
}

#[test]
fn csharp_members_and_visibility() {
    let facts = lens_cs(CSHARP_CONTROLLER, "src/Orders/OrdersController.cs");
    let id = "acme_orders_orderscontroller__";
    assert!(has_fact(&facts, &format!("function('{id}orderscontroller_1', 'acme_orders_orderscontroller', 'OrdersController', 1, public")));
    assert!(has_fact(
        &facts,
        &format!("function('{id}count_1', 'acme_orders_orderscontroller', 'Count', 1, internal")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{id}total_0', 'acme_orders_orderscontroller', 'Total', 0, public")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{id}item_1', 'acme_orders_orderscontroller', 'Item', 1, public")
    ));
    assert!(has_fact(&facts, &format!("function('{id}finalize_0'")));
    assert!(has_fact(&facts, &format!("function('{id}op_addition_2'")));
    assert!(has_fact(
        &facts,
        &format!("function('{id}local_0', 'acme_orders_orderscontroller', 'Local', 0, private")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{id}local_0__inner_1'")
    ));
    assert!(has_fact(&facts, "function('acme_orders_orderscontroller_nested__run_1', 'acme_orders_orderscontroller_nested', 'Run', 1, protected"));
    assert!(has_fact(
        &facts,
        "function('acme_orders_irepo__find_1', 'acme_orders_irepo', 'Find', 1, public"
    ));
    // two Get/1 overloads are two functions
    let gets = facts
        .iter()
        .filter(|f| f.starts_with("function(") && f.contains("'Get', 1"))
        .count();
    assert_eq!(gets, 2, "{facts:#?}");
    // auto-properties and field-like events carry no code
    assert!(!has_fact(&facts, "'Name', 0"));
    assert!(!has_fact(&facts, "'Changed'"));
}

#[test]
fn csharp_calls_attributes_and_constructor_chains() {
    let facts = lens_cs(CSHARP_CONTROLLER, "src/Orders/OrdersController.cs");
    let id = "acme_orders_orderscontroller__";
    for (caller, callee) in [
        ("get_1", "Find"),
        ("count_1", "Tally"),
        ("count_1", "Sum"),
        ("count_1", "Helper.Twice"),
        ("total_0", "Compute"),
        ("finalize_0", "Dispose"),
        ("op_addition_2", "Money.Add"),
        ("local_0", "Inner"),
        ("orderscontroller_1", "ControllerBase"),
    ] {
        assert!(
            has_fact(
                &facts,
                &format!("calls_external('{id}{caller}', 'unknown', '{callee}'")
            ),
            "{caller} -> {callee}"
        );
    }
    assert!(has_fact(
        &facts,
        "annotation('acme_orders_orderscontroller', 'ApiController', '')"
    ));
    assert!(has_fact(
        &facts,
        &format!("annotation('{id}get_1', 'HttpGet', '\"orders/{{id}}\"')")
    ));
}

#[test]
fn csharp_top_level_statements_and_tests() {
    let program = lens_cs(
        "var builder = WebApplication.CreateBuilder(args);\nbuilder.Services.AddControllers();\nvar app = builder.Build();\napp.Run();\n",
        "src/Api/Program.cs",
    );
    assert!(has_fact(
        &program,
        "function('src_api_program__main_0', 'src_api_program', 'Main', 0"
    ));
    assert!(has_fact(
        &program,
        "calls_external('src_api_program__main_0', 'unknown', 'WebApplication.CreateBuilder'"
    ));
    assert!(has_fact(
        &program,
        "calls_external('src_api_program__main_0', 'unknown', 'AddControllers'"
    ));

    let tests = lens_cs(
        "public class OrderTests { [Fact] public void Totals() { } [Theory, InlineData(1)] public void Each(int n) { } void Helper() { } }",
        "test/Acme.Orders.Tests/OrderTests.cs",
    );
    assert!(has_fact(&tests, "test('ordertests__totals_0', '', unit)"));
    assert!(has_fact(&tests, "test('ordertests__each_1', '', unit)"));
    assert!(has_fact(
        &tests,
        "test('ordertests__helper_0', '', test_helper)"
    ));
}

#[test]
fn msbuild_project_files_are_dependencies() {
    let csproj = r#"<Project Sdk="Microsoft.NET.Sdk.Web">
  <PropertyGroup>
    <TargetFrameworks>net8.0;net9.0</TargetFrameworks>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="Polly" />
    <PackageReference Include="NUnit" Version="$(NunitPackageVersion)" />
  </ItemGroup>
  <ItemGroup Condition="'$(TargetFramework)'=='net46'">
    <PackageReference Include="NUnit" Version="$(NunitPackageVersion)" />
    <ProjectReference Include="..\Acme.Core\Acme.Core.csproj" />
  </ItemGroup>
</Project>"#;
    let facts = Analyzer::new()
        .unwrap()
        .lens_code(
            csproj,
            Language::MsBuild,
            "src/Acme.Api/Acme.Api.csproj",
            "sha",
        )
        .unwrap()
        .facts;
    let m = "src_acme_api_acme_api_csproj";
    assert!(has_fact(
        &facts,
        &format!("module('{m}', 'Acme.Api.csproj'")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{m}', 'Newtonsoft.Json@13.0.3', package, 6)")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{m}', 'Polly', package")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{m}', '../Acme.Core/Acme.Core.csproj', project")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{m}', 'net8.0', target_framework")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{m}', 'net9.0', target_framework")
    ));
    let nunit = facts.iter().filter(|f| f.contains("'NUnit'")).count();
    assert_eq!(nunit, 1, "one NUnit, no unexpanded version: {facts:#?}");

    let sln = "Microsoft Visual Studio Solution File, Format Version 12.00\nProject(\"{FAE04EC0}\") = \"Acme.Api\", \"src\\Acme.Api\\Acme.Api.csproj\", \"{1}\"\nEndProject\n";
    let facts = Analyzer::new()
        .unwrap()
        .lens_code(sln, Language::MsBuild, "Acme.sln", "sha")
        .unwrap()
        .facts;
    assert!(has_fact(
        &facts,
        "depends_on('acme_sln', 'src/Acme.Api/Acme.Api.csproj', project, 2)"
    ));
}

// ─── T-SQL ───────────────────────────────────────────────────────────────────

fn lens_sql(code: &str, path: &str) -> Vec<String> {
    Analyzer::new()
        .unwrap()
        .lens_code(code, Language::TSql, path, "sha")
        .unwrap()
        .facts
}

const TSQL_PROC: &str = "\u{feff}SET ANSI_NULLS ON;
GO
CREATE OR ALTER PROCEDURE [Sales].[CloseOrder] @OrderID int, @Note AS nvarchar(100) = NULL OUTPUT
WITH EXECUTE AS OWNER
AS
BEGIN
    SET NOCOUNT ON;
    DECLARE @n int = dbo.CountLines(@OrderID);
    IF @n = 0
        RETURN;
    ELSE
        UPDATE Sales.Orders SET ClosedWhen = SYSDATETIME() WHERE OrderID = @OrderID;
    INSERT INTO #log (id) SELECT o.OrderID FROM Sales.Orders AS o JOIN Sales.OrderLines ol ON ol.OrderID = o.OrderID;
    SELECT * INTO #copy FROM Warehouse.Items, Warehouse.Groups g WHERE g.Id IN (1, 2);
    DELETE FROM Sales.Holds WHERE OrderID = @OrderID;
    BEGIN TRY
        EXEC Audit.Record @OrderID;
    END TRY
    BEGIN CATCH
        THROW;
    END CATCH
END;
GO
CREATE FUNCTION dbo.Lines(@id int) RETURNS TABLE AS RETURN SELECT * FROM Sales.OrderLines WHERE OrderID = @id;
GO
CREATE TABLE dbo.Audit (Id int PRIMARY KEY, Note nvarchar(max));
GO
";

#[test]
fn tsql_routines_are_functions_of_their_schema() {
    let facts = lens_sql(TSQL_PROC, "db/Sales/CloseOrder.sql");
    assert!(has_fact(&facts, "module('sales', 'Sales', 'schema:Sales'"));
    assert!(has_fact(
        &facts,
        "function('sales__closeorder_2', 'sales', 'CloseOrder', 2, public"
    ));
    assert!(has_fact(
        &facts,
        "function_param('sales__closeorder_2', 1, '@Note'"
    ));
    assert!(has_fact(
        &facts,
        "annotation('sales__closeorder_2', 'procedure', '')"
    ));
    assert!(has_fact(
        &facts,
        "function('dbo__lines_1', 'dbo', 'Lines', 1, public"
    ));
    assert!(has_fact(
        &facts,
        "annotation('dbo__lines_1', 'function', '')"
    ));
    assert!(has_fact(
        &facts,
        "module('dbo__audit', 'dbo.Audit', 'table:dbo.Audit'"
    ));
    // SET ANSI_NULLS and the CREATE TABLE are the file's script
    assert!(has_fact(&facts, "function('db_sales_closeorder__script_0'"));
}

#[test]
fn tsql_calls_and_table_access() {
    let facts = lens_sql(TSQL_PROC, "db/Sales/CloseOrder.sql");
    let id = "sales__closeorder_2";
    for callee in ["dbo.CountLines", "SYSDATETIME", "Audit.Record"] {
        assert!(
            has_fact(
                &facts,
                &format!("calls_external('{id}', 'unknown', '{callee}'")
            ),
            "{callee}"
        );
    }
    for (pred, table) in [
        ("writes_table", "Sales.Orders"),
        ("writes_table", "#log"),
        ("reads_table", "Sales.Orders"),
        ("reads_table", "Sales.OrderLines"),
        ("writes_table", "#copy"),
        ("reads_table", "Warehouse.Items"),
        ("reads_table", "Warehouse.Groups"),
        ("writes_table", "Sales.Holds"),
    ] {
        assert!(
            has_fact(&facts, &format!("{pred}('{id}', '{table}'")),
            "{pred} {table}"
        );
    }
    assert!(has_fact(
        &facts,
        "reads_table('dbo__lines_1', 'Sales.OrderLines'"
    ));
    // keywords, types and IN (…) are not calls; neither is the INSERT's column list
    for not_call in ["'IN'", "'nvarchar'", "'#log'", "'max'"] {
        assert!(
            !facts
                .iter()
                .any(|f| f.starts_with("calls_external") && f.contains(not_call)),
            "{not_call}"
        );
    }
}

// ─── PowerShell ──────────────────────────────────────────────────────────────

const PS_SCRIPT: &str = r#"#Requires -Modules dbatools, @{ ModuleName="Pester"; ModuleVersion="5.0" }
using module ./Fleet.psm1
Import-Module -Name SqlServer -MinimumVersion 22.0
. "$PSScriptRoot\private\Helpers.ps1"

function Get-DbaThing {
    [CmdletBinding()]
    param (
        [Parameter(Mandatory)]
        [string]$SqlInstance,
        [int]$Timeout = 30
    )
    $server = Connect-DbaInstance -SqlInstance $SqlInstance
    $rows = Invoke-DbaQuery -SqlInstance $server -Query "SELECT 1" | Where-Object { Test-Row $_ }
    $server.Databases.Refresh()
    [System.IO.File]::ReadAllText("x")
    function Inner { Write-Host "x" }
    Inner
}

class Fleet : System.Object {
    [string]$Name
    Fleet([string]$n) { $this.Name = $n }
    hidden [void] Start() { Start-Job -ScriptBlock { Get-Date } }
    static [Fleet] Create() { return [Fleet]::new("x") }
}

Get-DbaThing -SqlInstance localhost
"#;

fn lens_ps(code: &str, path: &str) -> Vec<String> {
    Analyzer::new()
        .unwrap()
        .lens_code(code, Language::PowerShell, path, "sha")
        .unwrap()
        .facts
}

#[test]
fn powershell_functions_parameters_and_classes() {
    let facts = lens_ps(PS_SCRIPT, "tools/Get-DbaThing.ps1");
    let f = "tools_get_dbathing__get_dbathing_2";
    assert!(has_fact(
        &facts,
        &format!("function('{f}', 'tools_get_dbathing', 'Get-DbaThing', 2, public")
    ));
    assert!(has_fact(
        &facts,
        &format!("function_param('{f}', 0, '$SqlInstance', 'string')")
    ));
    assert!(has_fact(
        &facts,
        &format!("function_param('{f}', 1, '$Timeout', 'int')")
    ));
    assert!(has_fact(
        &facts,
        &format!("annotation('{f}', 'CmdletBinding', '')")
    ));
    assert!(has_fact(&facts, &format!("function('{f}__inner_0'")));
    let c = "tools_get_dbathing_ps1__fleet";
    assert!(has_fact(
        &facts,
        &format!("module('{c}', 'Fleet', 'class:Fleet'")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{c}', 'System.Object', inherits")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{c}__fleet_1', '{c}', 'Fleet', 1, public")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{c}__start_0', '{c}', 'Start', 0, private")
    ));
    // top-level statements are the script
    assert!(has_fact(
        &facts,
        "calls_external('tools_get_dbathing__script_0', 'unknown', 'Get-DbaThing'"
    ));
}

#[test]
fn powershell_calls_and_dependencies() {
    let facts = lens_ps(PS_SCRIPT, "tools/Get-DbaThing.ps1");
    let f = "tools_get_dbathing__get_dbathing_2";
    for callee in [
        "Connect-DbaInstance",
        "Invoke-DbaQuery",
        "Where-Object",
        "Test-Row",
        "Refresh",
        "System.IO.File::ReadAllText",
        "Inner",
    ] {
        assert!(
            has_fact(
                &facts,
                &format!("calls_external('{f}', 'unknown', '{callee}'")
            ),
            "{callee}"
        );
    }
    // the nested function's calls are its own
    assert!(!has_fact(
        &facts,
        &format!("calls_external('{f}', 'unknown', 'Write-Host'")
    ));
    assert!(has_fact(
        &facts,
        "calls_external('tools_get_dbathing_ps1__fleet__create_0', 'unknown', 'Fleet'"
    ));
    let m = "tools_get_dbathing";
    for (target, kind) in [
        ("dbatools", "requires"),
        ("Pester", "requires"),
        ("./Fleet.psm1", "using"),
        ("SqlServer", "import"),
        ("$PSScriptRoot/private/Helpers.ps1", "dot_source"),
    ] {
        assert!(
            has_fact(&facts, &format!("depends_on('{m}', '{target}', {kind}")),
            "{target}"
        );
    }
    assert!(!facts
        .iter()
        .any(|f| f.starts_with("calls_external") && f.contains("'using'")));
}

#[test]
fn powershell_pester_files_are_tests() {
    let facts = lens_ps(
        "Describe 'Get-DbaThing' { It 'works' { Get-DbaThing | Should -Be 1 } }\nfunction New-Fixture { }\n",
        "tests/Get-DbaThing.Tests.ps1",
    );
    assert!(has_fact(
        &facts,
        "test('tests_get_dbathing_tests__new_fixture_0', '', test_helper)"
    ));
}

// ─── Java ────────────────────────────────────────────────────────────────────

const JAVA_CONTROLLER: &str = r#"package org.acme.orders;

import java.util.List;
import static org.junit.jupiter.api.Assertions.assertEquals;

@RestController
@RequestMapping("/orders")
public class OrderController extends BaseController implements Auditable {
    private final OrderRepository repo;

    @Autowired
    public OrderController(OrderRepository repo) { super(repo); this.repo = repo; }
    public OrderController() { this(null); }

    @GetMapping("/{id}")
    public Order get(@PathVariable int id) { return repo.findById(id).orElseThrow(() -> new NotFound(id)); }
    public Order get(String code) { return repo.byCode(code); }

    static <T> List<T> wrap(T x) { return List.of(x); }

    void schedule() {
        Runnable r = new Runnable() { public void run() { tick(); } };
    }

    interface Hook { void run(); }
    record Line(int qty) { int total() { return qty * 2; } }
    enum State { OPEN, CLOSED; boolean done() { return this == CLOSED; } }
}
"#;

fn lens_java(code: &str, path: &str) -> Vec<String> {
    Analyzer::new()
        .unwrap()
        .lens_code(code, Language::Java, path, "sha")
        .unwrap()
        .facts
}

#[test]
fn java_types_members_and_annotations() {
    let facts = lens_java(
        JAVA_CONTROLLER,
        "src/main/java/org/acme/orders/OrderController.java",
    );
    let t = "org_acme_orders_ordercontroller";
    assert!(has_fact(&facts, &format!("module('{t}', 'org.acme.orders.OrderController', 'class:org.acme.orders.OrderController'")));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{t}', 'BaseController', inherits")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{t}', 'Auditable', inherits")
    ));
    assert!(has_fact(
        &facts,
        "depends_on('src_main_java_org_acme_orders_ordercontroller', 'java.util.List', import"
    ));
    assert!(has_fact(
        &facts,
        &format!("annotation('{t}', 'RequestMapping', '\"/orders\"')")
    ));
    assert!(has_fact(
        &facts,
        &format!("annotation('{t}__get_1', 'GetMapping', '\"/{{id}}\"')")
    ));
    let gets = facts
        .iter()
        .filter(|f| f.starts_with("function(") && f.contains("'get', 1"))
        .count();
    assert_eq!(gets, 2, "two get/1 overloads");
    assert!(has_fact(
        &facts,
        &format!("function('{t}__wrap_1', '{t}', 'wrap', 1, package")
    ));
    assert!(has_fact(
        &facts,
        &format!("module('{t}_line', 'org.acme.orders.OrderController.Line', 'record:")
    ));
    assert!(has_fact(&facts, &format!("function('{t}_state__done_0'")));
    assert!(has_fact(
        &facts,
        &format!("function('{t}_hook__run_0', '{t}_hook', 'run', 0, public")
    ));
}

#[test]
fn java_calls() {
    let facts = lens_java(
        JAVA_CONTROLLER,
        "src/main/java/org/acme/orders/OrderController.java",
    );
    let t = "org_acme_orders_ordercontroller";
    for (caller, callee) in [
        ("ordercontroller_1", "BaseController"),
        ("ordercontroller_0", "OrderController"),
        ("get_1", "findById"),
        ("get_1", "orElseThrow"),
        ("get_1", "NotFound"),
        ("wrap_1", "List.of"),
        ("schedule_0", "Runnable"),
    ] {
        assert!(
            has_fact(
                &facts,
                &format!("calls_external('{t}__{caller}', 'unknown', '{callee}'")
            ),
            "{caller} -> {callee}"
        );
    }
    // the anonymous class's method is its own function
    assert!(facts
        .iter()
        .any(|f| f.starts_with("function(") && f.contains("'run', 0") && f.contains("anon_l")));
    assert!(!has_fact(
        &facts,
        &format!("calls_external('{t}__schedule_0', 'unknown', 'tick'")
    ));
}

#[test]
fn java_tests_by_annotation() {
    let facts = lens_java(
        "class OrderTest { @Test void totals() { } @ParameterizedTest void each(int n) { } void helper() { } }",
        "src/test/java/OrderTest.java",
    );
    assert!(has_fact(&facts, "test('ordertest__totals_0', '', unit)"));
    assert!(has_fact(&facts, "test('ordertest__each_1', '', unit)"));
    assert!(has_fact(
        &facts,
        "test('ordertest__helper_0', '', test_helper)"
    ));
}

// ─── Kotlin ──────────────────────────────────────────────────────────────────

const KOTLIN_SERVICE: &str = r#"package org.acme.orders

import kotlinx.coroutines.launch
import org.acme.db.*

@Service
class OrderService(private val repo: OrderRepository) : BaseService(), Auditable {
    fun get(id: Int): Order = repo.findById(id) ?: throw NotFound(id)
    internal suspend fun close(id: Int, note: String = "") { repo.update(id) { closed = true }; Audit.record(id) }
    val total: Int get() = compute(1)
    constructor(name: String) : this(Repo(name))
    companion object { fun create(): OrderService = OrderService(Repo()) }
}

fun String.shout(): String = uppercase()
private fun helper(x: Int) = listOf(x).map { it * 2 }

interface Auditable { fun audit() }
"#;

fn lens_kt(code: &str, path: &str) -> Vec<String> {
    Analyzer::new()
        .unwrap()
        .lens_code(code, Language::Kotlin, path, "sha")
        .unwrap()
        .facts
}

#[test]
fn kotlin_classes_objects_and_functions() {
    let facts = lens_kt(KOTLIN_SERVICE, "src/main/kotlin/OrderService.kt");
    let t = "org_acme_orders_orderservice";
    assert!(has_fact(
        &facts,
        &format!("module('{t}', 'org.acme.orders.OrderService', 'class:")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{t}', 'BaseService', inherits")
    ));
    assert!(has_fact(
        &facts,
        &format!("annotation('{t}', 'Service', '')")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{t}__orderservice_1', '{t}', 'OrderService', 1, public")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{t}__close_2', '{t}', 'close', 2, internal")
    ));
    assert!(has_fact(
        &facts,
        &format!("function_param('{t}__close_2', 1, 'note', 'String')")
    ));
    assert!(has_fact(&facts, &format!("function('{t}__total_0'")));
    assert!(has_fact(
        &facts,
        &format!("module('{t}_companion', 'org.acme.orders.OrderService.Companion', 'object:")
    ));
    assert!(has_fact(
        &facts,
        "module('org_acme_orders_auditable', 'org.acme.orders.Auditable', 'interface:"
    ));
    let f = "src_main_kotlin_orderservice";
    assert!(has_fact(
        &facts,
        &format!("function('{f}__shout_0', '{f}', 'shout', 0, public")
    ));
    assert!(has_fact(
        &facts,
        &format!("annotation('{f}__shout_0', 'receiver', 'String')")
    ));
    assert!(has_fact(
        &facts,
        &format!("function('{f}__helper_1', '{f}', 'helper', 1, private")
    ));
    assert!(has_fact(
        &facts,
        &format!("depends_on('{f}', 'org.acme.db.*', import")
    ));
}

#[test]
fn kotlin_calls() {
    let facts = lens_kt(KOTLIN_SERVICE, "src/main/kotlin/OrderService.kt");
    let t = "org_acme_orders_orderservice";
    for (caller, callee) in [
        ("get_1", "findById"),
        ("get_1", "NotFound"),
        ("close_2", "update"),
        ("close_2", "Audit.record"),
        ("total_0", "compute"),
    ] {
        assert!(
            has_fact(
                &facts,
                &format!("calls_external('{t}__{caller}', 'unknown', '{callee}'")
            ),
            "{caller} -> {callee}"
        );
    }
    assert!(has_fact(
        &facts,
        &format!("calls_external('{t}_companion__create_0', 'unknown', 'OrderService'")
    ));
    assert!(has_fact(
        &facts,
        "calls_external('src_main_kotlin_orderservice__helper_1', 'unknown', 'listOf'"
    ));
}
