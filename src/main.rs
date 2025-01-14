use tree_sitter::{Node, Parser};

fn main() {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser.set_language(&language.into()).expect("Error loading Python grammar");

    let file_path: &str = "examples/python/main.py";
    println!("Parsing Python file {}", file_path);

    let source_code = std::fs::read_to_string(file_path).unwrap();
    let tree = parser.parse(&source_code, None).unwrap();
    assert!(!tree.root_node().has_error());

    let root_node = tree.root_node();

    find_comments_in_node(root_node, &source_code);
}

fn find_comments_in_node(node: Node, code: &str) {
    if node.kind() == "comment" {
        let comment_text = &code[node.byte_range()];
        let line_number = node.start_position().row + 1;
        let context = find_context(node, code);

        println!(
            "Found comment: \"{}\" at line {} with context: \"{}\"",
            comment_text, line_number, context
        );
    }

    // Recursively check child nodes
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_comments_in_node(child, code);
    }
}

fn find_context(node: Node, code: &str) -> String {
    let mut context = String::new();

    // Include the parent node kind and its text (if available)
    if let Some(parent) = node.parent() {
        context.push_str(&format!("Parent: {}", parent.kind()));

        let parent_text = &code[parent.byte_range()];
        context.push_str(&format!(", Associated Code: {}", parent_text.trim()));
    } else {
        context.push_str("Parent: None, Associated Code: None");
    }

    context
}