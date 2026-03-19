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
