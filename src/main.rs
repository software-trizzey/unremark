/* Note: This project is still a prototype and I find it easier to keep the code in a single file. */

use clap::Parser as ClapParser;
use dotenv::dotenv;
use openai_api_rust::*;
use openai_api_rust::chat::*;
use rayon::prelude::*;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;
use log::{debug, error, info};
use env_logger;

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Remove redundant comments
    #[arg(long, default_value_t = false)]
    fix: bool,

    /// Ignore specific directories (comma-separated)
    #[arg(long, default_value = "venv,node_modules,.git,__pycache__")]
    ignore: String,

    /// Output results in JSON format
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Debug)]
struct AnalysisResult {
    path: PathBuf,
    redundant_comments: Vec<CommentInfo>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum Language {
    Python,
    JavaScript,
    TypeScript,
}

impl Language {
    fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(Language::Python),
            "js" => Some(Language::JavaScript),
            "ts" => Some(Language::TypeScript),
            _ => None,
        }
    }

    fn get_tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        }
    }
}

fn main() {
    dotenv().ok();
    env_logger::init();
    let args = Args::parse();
    let ignore_dirs: Vec<&str> = args.ignore.split(',').collect();

    info!("Analyzing files in: {}", args.path.display());

    let source_files: Vec<PathBuf> = WalkDir::new(&args.path)
        .into_iter()
        .filter_entry(|e| !is_ignored(e, &ignore_dirs))
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension()
                .and_then(|ext| ext.to_str())
                .and_then(Language::from_extension)
                .is_some()
        })
        .map(|e| e.path().to_owned())
        .collect();

    let total_files = source_files.len();
    debug!("Found {} files to analyze", total_files);

    let processed_files = Arc::new(AtomicUsize::new(0));

    // Process files in parallel
    let results: Vec<AnalysisResult> = source_files.par_iter()
        .map(|file| {
            let result = analyze_file(file, args.fix);
            let current = processed_files.fetch_add(1, Ordering::SeqCst) + 1;
            info!("Progress: [{}/{}] {}", current, total_files, file.display());
            result
        })
        .collect();

    print_summary(&results, args.json);
}

fn analyze_file(path: &PathBuf, fix: bool) -> AnalysisResult {
    let language = match path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(Language::from_extension) {
            Some(lang) => lang,
            None => return AnalysisResult {
                path: path.clone(),
                redundant_comments: vec![],
                errors: vec!["Unsupported file extension".to_string()],
            },
    };

    let mut parser = Parser::new();
    match parser.set_language(&language.get_tree_sitter_language()) {
        Ok(_) => (),
        Err(e) => return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![format!("Error loading language grammar: {}", e)],
        },
    }

    let source_code = match std::fs::read_to_string(path) {
        Ok(code) => code,
        Err(e) => return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![format!("Error reading file: {}", e)],
        },
    };

    let tree = match parser.parse(&source_code, None) {
        Some(tree) => tree,
        None => return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec!["Error parsing file".to_string()],
        },
    };

    if tree.root_node().has_error() {
        return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec!["Syntax errors found in file".to_string()],
        };
    }

    let comments = detect_comments(tree.root_node(), &source_code);
    
    if comments.is_empty() {
        return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![],
        };
    }

    let redundant_comments = match analyze_comments(comments) {
        Ok(comments) => comments,
        Err(e) => return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![format!("Error analyzing comments: {}", e)],
        },
    };

    if fix && !redundant_comments.is_empty() {
        let updated_source = remove_redundant_comments(&source_code, &redundant_comments);
        if let Err(e) = std::fs::write(path, updated_source) {
            return AnalysisResult {
                path: path.clone(),
                redundant_comments,
                errors: vec![format!("Error writing updated file: {}", e)],
            };
        }
    }

    AnalysisResult {
        path: path.clone(),
        redundant_comments,
        errors: vec![],
    }
}

fn print_summary(results: &[AnalysisResult], json_output: bool) {
    if json_output {
        let json_results: Vec<JsonFileResult> = results.iter().map(|r| JsonFileResult {
            path: r.path.display().to_string(),
            redundant_comments: r.redundant_comments.iter().map(|c| JsonCommentInfo {
                text: c.text.clone(),
                line_number: c.line_number,
                context: c.context.clone(),
            }).collect(),
            errors: r.errors.clone(),
        }).collect();

        let output = JsonOutput {
            total_files: results.len(),
            files_with_comments: results.iter()
                .filter(|r| !r.redundant_comments.is_empty())
                .count(),
            files_with_errors: results.iter()
                .filter(|r| !r.errors.is_empty())
                .count(),
            total_redundant_comments: results.iter()
                .map(|r| r.redundant_comments.len())
                .sum(),
            results: json_results,
        };

        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return;
    }

    let total_redundant = results.iter()
        .map(|r| r.redundant_comments.len())
        .sum::<usize>();

    let files_with_errors = results.iter()
        .filter(|r| !r.errors.is_empty())
        .count();

    let files_with_comments = results.iter()
        .filter(|r| !r.redundant_comments.is_empty())
        .count();

    println!("\nAnalysis Summary:");
    println!("----------------");
    println!("Total files processed: {}", results.len());
    println!("Files with redundant comments: {}", files_with_comments);
    println!("Files with errors: {}", files_with_errors);
    println!("Total redundant comments found: {}", total_redundant);

    if files_with_errors > 0 {
        println!("\nErrors encountered:");
        for result in results.iter().filter(|r| !r.errors.is_empty()) {
            eprintln!("  {}: ", result.path.display());
            for error in &result.errors {
                eprintln!("    - {}", error);
            }
        }
    }

    if total_redundant > 0 {
        println!("\nResults by file:");
        for result in results.iter().filter(|r| !r.redundant_comments.is_empty()) {
            println!("  {}: ", result.path.display());
            for comment in &result.redundant_comments {
                println!("    Line {}: {}", comment.line_number, comment.text);
            }
        }
    }
}

fn is_ignored(entry: &walkdir::DirEntry, ignore_dirs: &[&str]) -> bool {
    entry.file_type().is_dir() && 
    ignore_dirs.iter().any(|dir| entry.file_name().to_str().map_or(false, |s| s == *dir))
}

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

fn detect_comments(node: Node, code: &str) -> Vec<CommentInfo> {
    let mut comments = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "comment" {
            let comment_text = code[child.byte_range()].to_string();
            let line_number = child.start_position().row + 1;
            let context = find_context(child, code);

            debug!("Found comment on line {}: {}", line_number, comment_text);

            comments.push(CommentInfo {
                text: comment_text,
                line_number,
                context,
            });
        }
        comments.extend(detect_comments(child, code));
    }
    comments
}

fn analyze_comments(comments: Vec<CommentInfo>) -> Result<Vec<CommentInfo>, String> {
    let openai_api_key = std::env::var("OPENAI_API_KEY").expect("OpenAI API key not set");
    let auth = Auth::new(&openai_api_key);
    let openai = OpenAI::new(auth, "https://api.openai.com/v1/");
    
    Ok(comments.into_iter()
        .filter_map(|comment| {
            debug!("Analyzing comment on line {}: {}", comment.line_number, comment.text);
            
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
                                    info!("Found redundant comment: {}", analysis.explanation);
                                    return Some(comment);
                                }
                            } else {
                                error!("Failed to parse OpenAI response as JSON: {}", content.content);
                            }
                        }
                    }
                },
                Err(err) => error!("Error communicating with OpenAI: {:?}", err),
            }
            None
        })
        .collect())
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

#[derive(Debug, serde::Serialize)]
struct JsonOutput {
    total_files: usize,
    files_with_comments: usize,
    files_with_errors: usize,
    total_redundant_comments: usize,
    results: Vec<JsonFileResult>,
}

#[derive(Debug, serde::Serialize)]
struct JsonFileResult {
    path: String,
    redundant_comments: Vec<JsonCommentInfo>,
    errors: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
struct JsonCommentInfo {
    text: String,
    line_number: usize,
    context: String,
}