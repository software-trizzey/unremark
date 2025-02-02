use crate::types::{CommentInfo, Language};
use crate::utils::find_context;

use log::debug;
use tree_sitter::{Node, Parser};

pub fn detect_comments(source_code: &str, language: Language) -> Result<Vec<CommentInfo>, String> {
    let mut parser = Parser::new();
    if parser.set_language(&language.get_tree_sitter_language()).is_err() {
        return Ok(vec![]);
    }

    let tree = match parser.parse(source_code, None) {
        Some(tree) => tree,
        None => return Ok(vec![]),
    };

    if tree.root_node().has_error() {
        return Ok(vec![]);
    }

    Ok(collect_comments(tree.root_node(), source_code))
}

fn collect_comments(node: Node, code: &str) -> Vec<CommentInfo> {
    let mut comments = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        debug!("Node kind: {} at line {}", child.kind(), child.start_position().row + 1);
        if child.kind().contains("comment") {
            let comment_text = code[child.byte_range()].trim().to_string();
            
            // Skip documentation comments for all supported languages
            if comment_text.starts_with("///") ||    // Rust doc comments
               comment_text.starts_with("//!") ||    // Rust module doc comments
               comment_text.starts_with("/**") ||    // JSDoc/TSDoc/Rust block doc comments
               comment_text.starts_with("/*!")  ||   // Rust module block doc comments
               comment_text.starts_with("\"\"\"") || // Python docstrings
               comment_text.starts_with("'''") {     // Python docstrings (alternative)
                debug!("Skipping doc comment: {}", comment_text);
                continue;
            }

            let line_number = child.start_position().row + 1;
            let context = find_context(child, code);

            debug!("Found comment: '{}' of type '{}' on line {}", 
                comment_text, child.kind(), line_number
            );

            comments.push(CommentInfo {
                text: comment_text,
                line_number,
                context,
                explanation: Some("This comment may be redundant".to_string())
            });
        }
        comments.extend(collect_comments(child, code));
    }
    comments
} 