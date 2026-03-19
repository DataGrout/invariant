use tree_sitter::Parser;

fn main() {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap();
    
    let code = r#"
def hello():
    print('hello')

def world(x, y):
    return x + y
"#;
    
    let tree = parser.parse(code, None).unwrap();
    let root = tree.root_node();
    
    println!("Root kind: {}", root.kind());
    println!("Root children:");
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        println!("  - {} (line {})", child.kind(), child.start_position().row + 1);
        if child.kind() == "function_definition" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = name_node.utf8_text(code.as_bytes()).unwrap();
                println!("    Function name: {}", name);
            }
        }
        let mut child_cursor = child.walk();
        for grandchild in child.children(&mut child_cursor) {
            println!("    - {} (line {})", grandchild.kind(), grandchild.start_position().row + 1);
        }
    }
}
