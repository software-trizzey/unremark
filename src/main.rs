use dotenv::dotenv;
use openai_api_rust::*;
use openai_api_rust::chat::*;
use tree_sitter::{Node, Parser};
use serde::Deserialize;

#[derive(Debug, Clone)]
struct CommentInfo {
    text: String,
    line_number: usize,
    context: String,
}

#[derive(Debug, Deserialize)]
struct CommentAnalysis {
    is_redundant: bool,
    comment_line_number: usize,
    explanation: String,
}

fn main() {
    dotenv().ok();

    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser.set_language(&language.into()).expect("Error loading Python grammar");

    let file_path: &str = "examples/python/main.py";
    println!("Parsing Python file {}", file_path);

    let source_code = std::fs::read_to_string(file_path).unwrap();
    let tree = parser.parse(&source_code, None).unwrap();
    assert!(!tree.root_node().has_error());

    let root_node = tree.root_node();

    let comments = detect_comments(root_node, &source_code);

    let redundant_comments = analyze_comments(comments);
    
    if !redundant_comments.is_empty() {
        let updated_source = remove_redundant_comments(&source_code, &redundant_comments);
        std::fs::write(file_path, updated_source).expect("Failed to write updated source file");
        println!("Updated file written successfully!");
    } else {
        println!("No redundant comments found");
    }
}

fn detect_comments(node: Node, code: &str) -> Vec<CommentInfo> {
    let mut comments = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "comment" {
            let comment_text = code[child.byte_range()].to_string();
            let line_number = child.start_position().row + 1;
            let context = find_context(child, code);

            // found a comment
            println!("Found comment on line {}: {}", line_number, comment_text);

            comments.push(CommentInfo {
                text: comment_text,
                line_number,
                context,
            });
        }

        // Recursively check child nodes
        comments.extend(detect_comments(child, code));
    }

    comments
}

fn analyze_comments(comments: Vec<CommentInfo>) -> Vec<CommentInfo> {
    let openai_api_key = std::env::var("OPENAI_API_KEY").expect("OpenAI API key not set");
    let auth = Auth::new(&openai_api_key);
    let openai = OpenAI::new(auth, "https://api.openai.com/v1/");
    
    comments.into_iter()
        .filter_map(|comment| {
            println!("Analyzing comment on line {}: {}", comment.line_number, comment.text);
            
            let message = Message {
                role: Role::User,
                content: format!(
                    "Comment: '{}'\nContext: '{}'\nLine Number: {}\nIs this comment redundant or useful? Please respond with a JSON object containing the following fields: is_redundant, comment_line_number, comment_text, explanation",
                    comment.text,
                    comment.context,
                    comment.line_number
                ),
            };

            let body = ChatBody {
                model: std::env::var("OPENAI_API_MODEL").expect("OpenAI API model not set"),
                max_tokens: Some(500),
                temperature: Some(0_f32),
                top_p: Some(1_f32),
                n: Some(1),
                stream: Some(false),
                stop: None,
                presence_penalty: None,
                frequency_penalty: None,
                logit_bias: None,
                user: None,
                messages: vec![message],
            };

            let response = openai.chat_completion_create(&body);
            
            match response {
                Ok(result) => {
                    if let Some(choice) = result.choices.first() {
                        if let Some(content) = &choice.message {                            
                            if let Ok(analysis) = serde_json::from_str::<CommentAnalysis>(&content.content) {
                                if analysis.comment_line_number == comment.line_number && analysis.is_redundant {
                                    println!("Found redundant comment: {}", analysis.explanation);
                                    return Some(comment);
                                }
                            } else {
                                println!("Failed to parse OpenAI response as JSON: {}", content.content);
                            }
                        }
                    }
                },
                Err(err) => println!("Error communicating with OpenAI: {:?}", err),
            }
            None
        })
        .collect()
}


fn remove_redundant_comments(source: &str, redundant_comments: &[CommentInfo]) -> String {
    let mut updated_source = source.to_string();

    for comment in redundant_comments {
        println!("Removing comment at line {}: {}", comment.line_number, comment.text);
        updated_source = updated_source.replacen(&comment.text, "", 1);
    }

    updated_source
}

fn find_context(node: Node, code: &str) -> String {
    let mut context = String::new();
    
    let start_line = node.start_position().row;
    let lines: Vec<&str> = code.lines().collect();
    
    // Get up to 2 lines before and after the comment
    let context_start = start_line.saturating_sub(2);
    let context_end = (start_line + 2).min(lines.len());
    
    // Add the surrounding lines to context
    for i in context_start..context_end {
        if let Some(line) = lines.get(i) {
            let line = line.trim();
            if !line.is_empty() && !line.contains(&code[node.byte_range()]) {
                context.push_str(line);
                context.push('\n');
            }
        }
    }

    if context.is_empty() {
        "No surrounding code found".to_string()
    } else {
        context.trim().to_string()
    }
}