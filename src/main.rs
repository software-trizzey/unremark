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
use std::collections::HashMap;
use std::time::SystemTime;
use std::fs;
use parking_lot;

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

// Add new structs for caching
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    last_modified: u64,
    redundant_comments: Vec<CommentInfo>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Cache {
    entries: HashMap<String, CacheEntry>,
}

const CACHE_FILE_NAME: &str = "analysis_cache.json";

impl Cache {
    fn load_from_path(cache_path: &PathBuf) -> Self {
        match fs::read_to_string(cache_path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or(Cache {
                entries: HashMap::new(),
            }),
            Err(_) => Cache {
                entries: HashMap::new(),
            },
        }
    }

    fn save_to_path(&self, cache_path: &PathBuf) {
        if let Ok(contents) = serde_json::to_string(self) {
            let _ = fs::write(cache_path, contents);
        }
    }

    fn load() -> Self {
        Self::load_from_path(&get_cache_path())
    }

    fn save(&self) {
        self.save_to_path(&get_cache_path())
    }
}

fn get_cache_path() -> PathBuf {
    let mut cache_dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("."));
    cache_dir.push("unremark");
    fs::create_dir_all(&cache_dir).unwrap_or_default();
    cache_dir.push(CACHE_FILE_NAME);
    cache_dir
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CommentInfo {
    text: String,
    line_number: usize,
    context: String,
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

    // Use Arc to share cache across threads
    let cache = Arc::new(parking_lot::RwLock::new(Cache::load()));
    
    let results: Vec<AnalysisResult> = source_files.par_iter()
        .map(|file| {
            let cache = Arc::clone(&cache);
            let result = analyze_file(file, args.fix, &cache);
            let current = processed_files.fetch_add(1, Ordering::SeqCst) + 1;
            info!("Progress: [{}/{}] {}", current, total_files, file.display());
            result
        })
        .collect();

    // Save the cache after all processing
    cache.write().save();
    print_summary(&results, args.json);
}

fn analyze_file(path: &PathBuf, fix: bool, cache: &parking_lot::RwLock<Cache>) -> AnalysisResult {
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
    let path_str = canonical_path.to_string_lossy().to_string();

    // Get file's last modified time
    let last_modified = fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    {
        let cache_read = cache.read();
        if let Some(entry) = cache_read.entries.get(&path_str) {
            if entry.last_modified == last_modified {
                debug!("Using cached results for {}", path.display());
                return AnalysisResult {
                    path: path.clone(),
                    redundant_comments: entry.redundant_comments.clone(),
                    errors: vec![],
                };
            }
        }
    }

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

    // Update cache with write lock
    let mut cache_write = cache.write();
    cache_write.entries.insert(
        path_str,
        CacheEntry {
            last_modified,
            redundant_comments: redundant_comments.clone(),
        },
    );

    // Add debug logging to track cache operations
    debug!("Cached results for {}: {} comments", path.display(), redundant_comments.len());

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
                model: "ft:gpt-4o-mini-2024-07-18:personal:unremark:Aq45wBQq".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_cache() -> (TempDir, PathBuf) {
        let temporary_directory = TempDir::new().unwrap();
        let cache_path = temporary_directory.path().join(CACHE_FILE_NAME);
        (temporary_directory, cache_path)
    }

    #[test]
    fn test_cache_storage_and_retrieval() {
        let (temporary_directory, cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.py");
        fs::write(&test_file, "# Test comment\ndef test():\n    pass").unwrap();

        let result1 = analyze_file(&test_file, false, &cache);
        cache.write().save_to_path(&cache_path); // Save to test cache path
        assert!(!result1.redundant_comments.is_empty(), "Should find redundant comments");

        let cache_contents = fs::read_to_string(&cache_path).unwrap_or_default();
        assert!(!cache_contents.is_empty(), "Cache file should not be empty");

        let cache2 = Arc::new(parking_lot::RwLock::new(Cache::load_from_path(&cache_path)));
        let result2 = analyze_file(&test_file, false, &cache2);
        assert_eq!(
            result1.redundant_comments.len(),
            result2.redundant_comments.len(),
            "Cached results should match original analysis"
        );
    }

    #[test]
    fn test_cache_invalidation() {
        let (temporary_directory, cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.py");
        fs::write(&test_file, "# This is a test file\ndef calculate_sum(a, b):\n    return a + b").unwrap();

        let result1 = analyze_file(&test_file, false, &cache);
        cache.write().save_to_path(&cache_path); // Save after first analysis

        // Modify the file with a useful comment
        std::thread::sleep(std::time::Duration::from_secs(1));
        fs::write(&test_file, "# This function uses integer arithmetic for precise calculations\ndef calculate_sum(a, b):\n    return a + b").unwrap();

        let cache2 = Arc::new(parking_lot::RwLock::new(Cache::load_from_path(&cache_path)));
        let result2 = analyze_file(&test_file, false, &cache2);

        assert_ne!(
            result1.redundant_comments.len(),
            result2.redundant_comments.len(),
            "Results should differ after file modification"
        );
    }
}