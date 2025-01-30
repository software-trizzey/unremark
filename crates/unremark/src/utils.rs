use crate::types::CommentInfo;
use tree_sitter::Node;
use log::debug;
use std::path::PathBuf;
use std::fs;
use crate::constants::CACHE_FILE_NAME;

pub fn get_cache_path() -> PathBuf {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("unremark");
    
    debug!("Cache directory: {}", cache_dir.display());
    fs::create_dir_all(&cache_dir).unwrap_or_default();
    
    cache_dir.join(CACHE_FILE_NAME)
}


pub fn find_context(node: Node, code: &str) -> String {
    let mut parent = node;

    while let Some(p) = parent.parent() {
        let kind = p.kind();
        if kind.contains("function") || kind.contains("class") || kind.contains("method") {
            return p.utf8_text(code.as_bytes()).unwrap_or("").to_string();
        }
        parent = p;
    }

    String::new()
}

pub fn remove_redundant_comments(source: &str, redundant_comments: &[CommentInfo]) -> String {
    let mut updated_source = source.to_string();

    // First, find all docstring positions to avoid modifying them
    let docstring_pattern = match std::path::Path::new(&source)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        Some("py") => r#"(?m)[\t ]*'''[^']*'''|[\t ]*"""[^"]*""""#,
        _ => r#"(?m)$^"#  // Match nothing for non-Python files
    };
    let docstring_regex = regex::Regex::new(docstring_pattern).unwrap();
    let docstring_positions: Vec<_> = docstring_regex.find_iter(&updated_source)
        .map(|m| (m.start(), m.end()))
        .collect();

    debug!("Source content:\n{}", updated_source);
    debug!("Found {} docstrings", docstring_positions.len());
    for (i, (start, end)) in docstring_positions.iter().enumerate() {
        let docstring = &updated_source[*start..*end];
        debug!("Docstring {} at positions {}..{}:\n{}", 
            i,
            start,
            end,
            docstring
        );
    }

    for comment in redundant_comments {
        let comment_text = &comment.text;
        
        // Get the position of this comment in the source
        if let Some(comment_pos) = updated_source.find(comment_text) {
            debug!("Found comment '{}' at position {}", 
                comment_text.replace('\n', "\\n"), 
                comment_pos
            );
            
            // Check if this comment is part of a docstring
            let is_in_docstring = docstring_positions.iter()
                .any(|&(start, end)| {
                    let in_range = comment_pos >= start && comment_pos < end;
                    if in_range {
                        debug!("Comment is inside docstring range {}..{}", start, end);
                    }
                    in_range
                });
            
            if is_in_docstring {
                debug!("Skipping comment in docstring: {}", comment_text);
                continue;
            }

            // For single-line comments, ensure we match the exact comment
            let pattern = if comment_text.starts_with('#') || comment_text.starts_with("//") {
                if comment_pos > 0 && updated_source[..comment_pos].trim_end().chars().last() != Some('{') {
                    // Inline comment
                    format!("[ \t]*{}[ \t]*(?:\r?\n|$)", regex::escape(comment_text))
                } else {
                    // Line-start comment
                    format!("(?m)^[ \t]*{}[ \t]*(?:\r?\n|$)", regex::escape(comment_text))
                }
            } else {
                format!("[ \t]*{}[ \t]*", regex::escape(comment_text))
            };

            // Use regex to ensure we only replace exact matches
            if let Ok(regex) = regex::Regex::new(&pattern) {
                debug!("Removing comment at line {}: {} with pattern {}", 
                    comment.line_number, 
                    comment_text,
                    pattern
                );
                updated_source = regex.replace_all(&updated_source, "").to_string();
            }
        }
    }

    // Clean up any empty lines created by comment removal
    let cleaned = updated_source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<&str>>()
        .join("\n");

    debug!("Final content:\n{}", cleaned);

    // Ensure we end with a newline
    cleaned + "\n"
}