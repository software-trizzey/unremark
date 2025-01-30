use crate::types::{ApiError, CommentInfo, CommentAnalysis, AnalysisResult, Language, Cache, CacheEntry};
use crate::api::make_api_request;
use crate::comment_detection::detect_comments;
use crate::utils::remove_redundant_comments;
use std::path::PathBuf;
use std::fs;
use std::sync::Arc;
use futures::future::join_all;
use std::time::{Duration, Instant};
use tree_sitter::Parser;
use log::{debug, error, info};
use std::time::SystemTime;
use parking_lot;


pub async fn analyze_file(path: &PathBuf, fix: bool, cache: &parking_lot::RwLock<Cache>) -> AnalysisResult {
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());
    let path_str = canonical_path.to_string_lossy().to_string();

    // Get file's last modified time
    let last_modified = fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let source_code = match std::fs::read_to_string(path) {
        Ok(code) => code,
        Err(_) => return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![],
        },
    };

    // Check cache first
    let redundant_comments = {
        let cache_read = cache.read();
        if let Some(entry) = cache_read.entries.get(&path_str) {
            if entry.last_modified == last_modified {
                entry.redundant_comments.clone()
            } else {
                drop(cache_read);
                let analysis = analyze_source(&source_code, path).await;
                // Update cache
                let mut cache_write = cache.write();
                cache_write.entries.insert(
                    path_str,
                    CacheEntry {
                        last_modified,
                        redundant_comments: analysis.redundant_comments.clone(),
                    },
                );
                analysis.redundant_comments
            }
        } else {
            drop(cache_read);
            let analysis = analyze_source(&source_code, path).await;
            // Update cache
            let mut cache_write = cache.write();
            cache_write.entries.insert(
                path_str,
                CacheEntry {
                    last_modified,
                    redundant_comments: analysis.redundant_comments.clone(),
                },
            );
            analysis.redundant_comments
        }
    };

    // Apply fixes if requested
    if fix && !redundant_comments.is_empty() {
        let updated_source = remove_redundant_comments(&source_code, &redundant_comments);
        if let Err(e) = std::fs::write(path, updated_source) {
            error!("Failed to write changes to {}: {}", path.display(), e);
        }
    }

    AnalysisResult {
        path: path.clone(),
        redundant_comments,
        errors: vec![],
    }
}

pub async fn analyze_source(source_code: &str, path: &PathBuf) -> AnalysisResult {
    let language = match path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(Language::from_extension) {
            Some(lang) => lang,
            None => return AnalysisResult {
                path: path.clone(),
                redundant_comments: vec![],
                errors: vec![],
            },
    };

    let mut parser = Parser::new();
    if parser.set_language(&language.get_tree_sitter_language()).is_err() {
        return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![],
        };
    }

    let tree = match parser.parse(source_code, None) {
        Some(tree) => tree,
        None => return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![],
        },
    };

    if tree.root_node().has_error() {
        return AnalysisResult {
            path: path.clone(),
            redundant_comments: vec![],
            errors: vec![],
        };
    }

    let comments = detect_comments(source_code, language).unwrap_or_default();
    let redundant_comments = analyze_comments(comments).await.unwrap_or_default();

    AnalysisResult {
        path: path.clone(),
        redundant_comments,
        errors: vec![],
    }
}

pub async fn analyze_comments(comments: Vec<CommentInfo>) -> Result<Vec<CommentInfo>, String> {
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(None)
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let openai_api_key = std::env::var("OPENAI_API_KEY").expect("OpenAI API key not set");
    let openai = Arc::new(client);
    
    let start_time = Instant::now();
    debug!("Starting concurrent analysis of {} comments", comments.len());

    // Create all API request futures at once
    let futures: Vec<_> = comments.into_iter()
        .map(|comment| {
            let openai = Arc::clone(&openai);
            let api_key = openai_api_key.clone();
            async move {
                let result = make_api_request(&openai, &api_key, &comment).await;
                (comment, result)
            }
        })
        .collect();

    // Execute all API requests concurrently
    let results = join_all(futures).await;
    
    let duration = start_time.elapsed();
    debug!("Completed analysis of {} comments in {:.2} seconds", 
        results.len(),
        duration.as_secs_f64()
    );

    // Process results and filter redundant comments
    let futures: Vec<_> = results.into_iter()
        .map(|(comment, api_result)| async move {
            match api_result {
                Ok(json) => {
                    if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
                        if let Ok(analysis) = serde_json::from_str::<CommentAnalysis>(content) {
                            if analysis.comment_line_number == comment.line_number && analysis.is_redundant {
                                info!("Found redundant comment: {}", analysis.explanation);
                                return Some(comment);
                            }
                        }
                    }
                },
                Err(err) => {
                    error!("Error analyzing comment '{}': {}", comment.text, err);
                    match err {
                        ApiError::RateLimit(msg) => {
                            error!("Rate limit exceeded. Consider reducing concurrent requests. Details: {}", msg);
                        },
                        ApiError::Timeout(msg) => {
                            error!("Request timed out. The API may be experiencing high latency. Details: {}", msg);
                        },
                        ApiError::Network(msg) => {
                            error!("Network error. Please check your internet connection. Details: {}", msg);
                        },
                        ApiError::Other(msg) => {
                            error!("Unexpected error occurred. Details: {}", msg);
                        },
                    }
                }
            }
            None
        })
        .collect();

    Ok(join_all(futures).await.into_iter().filter_map(|x| x).collect())
}

// Note: this is used by the LSP server to analyze the current file
pub async fn analyze_current_file(source_code: &str, language: Language) -> AnalysisResult {
    let mut parser = Parser::new();
    if parser.set_language(&language.get_tree_sitter_language()).is_err() {
        return AnalysisResult {
            path: PathBuf::new(),
            redundant_comments: vec![],
            errors: vec![],
        };
    }

    let tree = match parser.parse(source_code, None) {
        Some(tree) => tree,
        None => return AnalysisResult {
            path: PathBuf::new(),
            redundant_comments: vec![],
            errors: vec![],
        },
    };

    if tree.root_node().has_error() {
        return AnalysisResult {
            path: PathBuf::new(),
            redundant_comments: vec![],
            errors: vec![],
        };
    }

    let comments = detect_comments(source_code, language).unwrap_or_default();
    let redundant_comments = analyze_comments(comments).await.unwrap_or_default();

    AnalysisResult {
        path: PathBuf::new(),
        redundant_comments,
        errors: vec![],
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ApiError;
    use crate::constants::CACHE_FILE_NAME;
    use crate::utils::get_cache_path;
    
    use std::collections::HashMap;
    use std::fs;
    use reqwest::StatusCode;
    use tempfile::TempDir;
    use tokio::time::sleep;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path};
    use serde_json::json;

    fn setup_test_cache() -> (TempDir, PathBuf) {
        let temporary_directory = TempDir::new().unwrap();
        let cache_path = temporary_directory.path().join(CACHE_FILE_NAME);
        (temporary_directory, cache_path)
    }

    fn clear_cache() {
        if let Ok(cache_path) = get_cache_path().canonicalize() {
            debug!("Clearing cache at: {}", cache_path.display());
            let _ = fs::remove_file(cache_path);
        }
    }

    #[tokio::test]
    async fn test_cache_storage_and_retrieval() {
        clear_cache(); // Add this at the start of each test
        let (temporary_directory, cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.py");
        fs::write(&test_file, "# Test comment\ndef test():\n    pass").unwrap();

        let result1 = analyze_file(&test_file, false, &cache).await;
        cache.write().save_to_path(&cache_path);
        assert!(!result1.redundant_comments.is_empty(), "Should find redundant comments");

        let cache_contents = fs::read_to_string(&cache_path).unwrap_or_default();
        assert!(!cache_contents.is_empty(), "Cache file should not be empty");

        let cache2 = Arc::new(parking_lot::RwLock::new(Cache::load_from_path(&cache_path)));
        let result2 = analyze_file(&test_file, false, &cache2).await;

        assert_eq!(
            result1.redundant_comments.len(),
            result2.redundant_comments.len(),
            "Cached results should match original analysis"
        );
    }

    #[tokio::test]
    async fn test_cache_invalidation() {
        let (temporary_directory, cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.py");
        fs::write(&test_file, "# This is a test file\ndef calculate_sum(a, b):\n    return a + b").unwrap();

        let result1 = analyze_file(&test_file, false, &cache).await;
        cache.write().save_to_path(&cache_path);

        // Modify the file with a useful comment
        std::thread::sleep(std::time::Duration::from_secs(1));
        fs::write(&test_file, "# This function uses integer arithmetic for precise calculations\ndef calculate_sum(a, b):\n    return a + b").unwrap();

        let cache2 = Arc::new(parking_lot::RwLock::new(Cache::load_from_path(&cache_path)));
        let result2 = analyze_file(&test_file, false, &cache2).await;

        assert_ne!(
            result1.redundant_comments.len(),
            result2.redundant_comments.len(),
            "Results should differ after file modification"
        );
    }

    #[tokio::test]
    async fn test_fix_command_uncached() {
        let (temporary_directory, _cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.py");
        let initial_content = "# This is a test file\ndef calculate_sum(a, b):\n    # Adds two numbers together\n    return a + b";
        fs::write(&test_file, initial_content).unwrap();

        let result = analyze_file(&test_file, true, &cache).await;
        
        let updated_content = fs::read_to_string(&test_file).unwrap();
        assert_ne!(initial_content, updated_content, "Fix command should modify the file");
        assert!(!updated_content.contains("# This is a test file"), "Redundant comment should be removed");
        assert!(!updated_content.contains("# Adds two numbers together"), "Redundant comment should be removed");
        assert!(!result.redundant_comments.is_empty(), "Should identify redundant comments");
    }

    #[tokio::test]
    async fn test_fix_command_cached() {
        let (temporary_directory, cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.py");
        let initial_content = "# Another test comment\ndef calculate_sum(a, b):\n    # Performs addition\n    return a + b";
        fs::write(&test_file, initial_content).unwrap();

        let result1 = analyze_file(&test_file, false, &cache).await;
        cache.write().save_to_path(&cache_path);
        assert!(!result1.redundant_comments.is_empty(), "Should find redundant comments");

        let cache2 = Arc::new(parking_lot::RwLock::new(Cache::load_from_path(&cache_path)));
        let result2 = analyze_file(&test_file, true, &cache2).await;

        let final_content = fs::read_to_string(&test_file).unwrap();
        assert_ne!(initial_content, final_content, "Fix command should work with cached results");
        assert!(!final_content.contains("# Another test comment"), "Redundant comment should be removed");
        assert!(!final_content.contains("# Performs addition"), "Redundant comment should be removed");
        assert!(!result2.redundant_comments.is_empty(), "Should find redundant comments from cache");
    }

    #[tokio::test]
    async fn test_rust_comment_analysis() {
        let (temporary_directory, _cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.rs");
        let initial_content = r#"
// This is a test file
fn calculate_sum(a: i32, b: i32) -> i32 {
    // Adds two numbers together
    a + b  // Returns the sum
}

// Another redundant comment
struct Point {
    // The x coordinate
    x: i32,
    // The y coordinate
    y: i32,
}
"#;
        fs::write(&test_file, initial_content).unwrap();

        let analysis_result = analyze_file(&test_file, false, &cache).await;
        assert!(!analysis_result.redundant_comments.is_empty(), "Should identify redundant comments in Rust code");
        
        let comment_texts: Vec<&str> = analysis_result.redundant_comments
            .iter()
            .map(|c| c.text.trim())
            .collect();
        
        assert!(comment_texts.contains(&"// This is a test file"), "Should detect file-level redundant comment");
        assert!(comment_texts.contains(&"// Adds two numbers together"), "Should detect redundant function comment");
        assert!(comment_texts.contains(&"// Returns the sum"), "Should detect redundant inline comment");

        let fix_result = analyze_file(&test_file, true, &cache).await;
        assert!(!fix_result.redundant_comments.is_empty(), "Should still report the redundant comments");

        let final_content = fs::read_to_string(&test_file).unwrap();
        assert!(!final_content.contains("// This is a test file"), "Should remove redundant file comment");
        assert!(!final_content.contains("// Adds two numbers together"), "Should remove redundant function comment");
        assert!(!final_content.contains("// Returns the sum"), "Should remove redundant inline comment");
        
        assert!(final_content.contains("fn calculate_sum(a: i32, b: i32) -> i32 {"), "Should preserve function signature");
        assert!(final_content.contains("struct Point {"), "Should preserve struct definition");
        assert!(final_content.contains("x: i32,"), "Should preserve struct fields");
        assert!(final_content.contains("y: i32,"), "Should preserve struct fields");
    }

    #[tokio::test]
    async fn test_rust_doc_comments_ignored() {
        let (temporary_directory, _cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.rs");
        let initial_content = r#"
//! Module-level documentation
//! that should be preserved

/// Documentation for the function
/// that spans multiple lines
fn documented_function(x: i32) -> i32 {
    // This is a redundant comment
    x + 1
}

/** 
 * Alternative doc comment style
 * that should also be preserved
 */
struct DocumentedStruct {
    /// Documentation for x field
    x: i32,
    /** Documentation for y field */
    y: i32,
}
"#;
        fs::write(&test_file, initial_content).unwrap();

        let analysis_result = analyze_file(&test_file, false, &cache).await;
        
        assert_eq!(analysis_result.redundant_comments.len(), 1, "Should only detect one redundant comment");
        assert_eq!(
            analysis_result.redundant_comments[0].text.trim(),
            "// This is a redundant comment",
            "Should only detect the non-doc comment as redundant"
        );

        let fix_result = analyze_file(&test_file, true, &cache).await;
        assert!(!fix_result.redundant_comments.is_empty(), "Should still report the redundant comments");
        let final_content = fs::read_to_string(&test_file).unwrap();
        assert!(final_content.contains("//! Module-level documentation"), "Should preserve module doc comments");
        assert!(final_content.contains("/// Documentation for the function"), "Should preserve function doc comments");
        assert!(final_content.contains("* Alternative doc comment style"), "Should preserve alternative doc style");
        assert!(final_content.contains("/// Documentation for x field"), "Should preserve field doc comments");
        assert!(final_content.contains("/** Documentation for y field */"), "Should preserve inline doc comments");
        assert!(!final_content.contains("// This is a redundant comment"), "Should remove redundant comment");
        assert!(final_content.contains("fn documented_function(x: i32) -> i32 {"), "Should preserve function signature");
        assert!(final_content.contains("struct DocumentedStruct {"), "Should preserve struct definition");
    }

    #[tokio::test]
    async fn test_python_comment_analysis() {
        let (temporary_directory, _cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.py");
        let initial_content = r#"
#!/usr/bin/env python3
"""
Module level docstring
that should be preserved
"""

# This is a redundant file comment
def calculate_sum(a: int, b: int) -> int:
    '''Function level docstring that should be preserved'''
    # Adds two numbers together
    return a + b  # Returns the sum

class Point:
    """
    Class level docstring
    that should be preserved
    """
    def __init__(self, x: int, y: int):
        # Initialize coordinates
        self.x = x  # x coordinate
        self.y = y  # y coordinate
"#;
        fs::write(&test_file, initial_content).unwrap();

        let analysis_result = analyze_file(&test_file, false, &cache).await;
        
        let comment_texts: Vec<&str> = analysis_result.redundant_comments
            .iter()
            .map(|c| c.text.trim())
            .collect();

        assert!(!analysis_result.redundant_comments.is_empty(), "Should identify redundant comments");
        assert!(comment_texts.contains(&"# This is a redundant file comment"), "Should detect file-level comment");
        assert!(comment_texts.contains(&"# Adds two numbers together"), "Should detect function comment");
        assert!(comment_texts.contains(&"# Returns the sum"), "Should detect inline comment");
        assert!(!comment_texts.iter().any(|&c| c.contains("Module level docstring")), "Should not detect module docstring");
        assert!(!comment_texts.iter().any(|&c| c.contains("Function level docstring")), "Should not detect function docstring");
        assert!(!comment_texts.iter().any(|&c| c.contains("Class level docstring")), "Should not detect class docstring");

        let fix_result = analyze_file(&test_file, true, &cache).await;
        assert!(!fix_result.redundant_comments.is_empty(), "Should still report the redundant comments");
        
        let final_content = fs::read_to_string(&test_file).unwrap();
        assert!(final_content.contains("'''Function level docstring"), "Should preserve function docstring");
        assert!(!final_content.contains("# This is a redundant file comment"), "Should remove redundant comment");
    }

    #[tokio::test]
    async fn test_javascript_comment_analysis() {
        let (temporary_directory, _cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.js");
        let initial_content = r#"
/**
 * @fileoverview Module documentation
 * that should be preserved
 */

// This is a redundant file comment
function calculateSum(a, b) {
    /** 
     * Function documentation
     * that should be preserved
     */
    // Adds two numbers together
    return a + b; // Returns the sum
}
"#;
        fs::write(&test_file, initial_content).unwrap();

        let analysis_result = analyze_file(&test_file, false, &cache).await;
        
        let comment_texts: Vec<&str> = analysis_result.redundant_comments
            .iter()
            .map(|c| c.text.trim())
            .collect();

        assert!(!analysis_result.redundant_comments.is_empty(), "Should identify redundant comments");
        assert!(comment_texts.contains(&"// This is a redundant file comment"), "Should detect file-level comment");
        assert!(comment_texts.contains(&"// Adds two numbers together"), "Should detect function comment");
        assert!(comment_texts.contains(&"// Returns the sum"), "Should detect inline comment");
        assert!(!comment_texts.iter().any(|&c| c.contains("@fileoverview")), "Should not detect JSDoc module comment");
        assert!(!comment_texts.iter().any(|&c| c.contains("Function documentation")), "Should not detect JSDoc function comment");

        let fix_result = analyze_file(&test_file, true, &cache).await;
        assert!(!fix_result.redundant_comments.is_empty(), "Should still report the redundant comments");
        
        let final_content = fs::read_to_string(&test_file).unwrap();
        assert!(final_content.contains("@fileoverview Module documentation"), "Should preserve JSDoc module comment");
        assert!(final_content.contains("Function documentation"), "Should preserve JSDoc function comment");
        assert!(!final_content.contains("// This is a redundant file comment"), "Should remove redundant comment");
    }

    #[tokio::test]
    async fn test_typescript_comment_analysis() {
        let (temporary_directory, _cache_path) = setup_test_cache();
        let cache = Arc::new(parking_lot::RwLock::new(Cache {
            entries: HashMap::new(),
        }));

        let test_file = temporary_directory.path().join("test.ts");
        let initial_content = r#"
/**
 * @fileoverview Module documentation
 * that should be preserved
 */

// This is a redundant file comment
function calculateSum(a: number, b: number): number {
    /** 
     * Function documentation
     * that should be preserved
     */
    // Adds two numbers together
    return a + b; // Returns the sum
}

interface Shape {
    /** Interface documentation that should be preserved */
    getArea(): number;
}
"#;
        fs::write(&test_file, initial_content).unwrap();

        let analysis_result = analyze_file(&test_file, false, &cache).await;
        
        let comment_texts: Vec<&str> = analysis_result.redundant_comments
            .iter()
            .map(|c| c.text.trim())
            .collect();

        assert!(!analysis_result.redundant_comments.is_empty(), "Should identify redundant comments");
        assert!(comment_texts.contains(&"// This is a redundant file comment"), "Should detect file-level comment");
        assert!(comment_texts.contains(&"// Adds two numbers together"), "Should detect function comment");
        assert!(comment_texts.contains(&"// Returns the sum"), "Should detect inline comment");
        assert!(!comment_texts.iter().any(|&c| c.contains("@fileoverview")), "Should not detect TSDoc module comment");
        assert!(!comment_texts.iter().any(|&c| c.contains("Function documentation")), "Should not detect TSDoc function comment");
        assert!(!comment_texts.iter().any(|&c| c.contains("Interface documentation")), "Should not detect TSDoc interface comment");

        let fix_result = analyze_file(&test_file, true, &cache).await;
        assert!(!fix_result.redundant_comments.is_empty(), "Should still report the redundant comments");
        
        let final_content = fs::read_to_string(&test_file).unwrap();
        assert!(final_content.contains("@fileoverview Module documentation"), "Should preserve TSDoc module comment");
        assert!(final_content.contains("Function documentation"), "Should preserve TSDoc function comment");
        assert!(final_content.contains("Interface documentation"), "Should preserve TSDoc interface comment");
        assert!(!final_content.contains("// This is a redundant file comment"), "Should remove redundant comment");
    }

    #[tokio::test]
    async fn test_rate_limit_handling() {
        let mock_server = MockServer::start().await;
        
        // First request - rate limit with retry-after
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429)
                .insert_header("retry-after", "1")
                .set_body_json(json!({
                    "error": {
                        "message": "Rate limit exceeded",
                        "type": "rate_limit_error"
                    }
                })))
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // Second request - success
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(json!({
                    "id": "test-id",
                    "object": "chat.completion",
                    "created": 1234567890,
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "{\"is_redundant\": true, \"comment_line_number\": 1, \"comment_text\": \"Test comment\", \"explanation\": \"Test explanation\"}"
                        },
                        "finish_reason": "stop"
                    }]
                })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let comment = CommentInfo {
            text: "// Test comment".to_string(),
            line_number: 1,
            context: "Test context".to_string(),
        };

        let result = make_test_api_request(
            &client,
            "test_key",
            &comment,
            &mock_server.uri()
        ).await;

        assert!(result.is_ok(), "Request should succeed after retries: {:?}", result);
    }

    async fn make_test_api_request(
        client: &reqwest::Client,
        api_key: &str,
        comment: &CommentInfo,
        base_url: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let max_retries = 3;
        let mut retry_delay = Duration::from_millis(100); // Reduced for tests

        for attempt in 0..max_retries {
            if attempt > 0 {
                debug!("Retrying request (attempt {}/{})", attempt + 1, max_retries);
                sleep(retry_delay).await;
            }

            let message = serde_json::json!({
                "model": "ft:gpt-4o-mini-2024-07-18:personal:unremark:Aq45wBQq",
                "messages": [{
                    "role": "user",
                    "content": format!(
                        "Comment: '{}'\nContext: '{}'\nLine Number: {}\nIs this comment redundant or useful? Please respond with a JSON object containing the following fields: is_redundant, comment_line_number, comment_text, explanation",
                        comment.text,
                        comment.context,
                        comment.line_number
                    )
                }],
                "max_tokens": 500,
                "temperature": 0.0,
                "top_p": 1.0,
                "n": 1,
                "stream": false
            });

            let request_url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

            match client
                .post(&request_url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&message)
                .send()
                .await
            {
                Ok(response) => {
                    match response.status() {
                        StatusCode::OK => {
                            return response.json().await.map_err(|e| {
                                ApiError::Other(format!("Failed to parse response: {}", e))
                            });
                        }
                        StatusCode::TOO_MANY_REQUESTS => {
                            if attempt == max_retries - 1 {
                                return Err(ApiError::RateLimit(
                                    "Rate limit exceeded after all retries".to_string(),
                                ));
                            }
                            
                            // Get retry delay from header or use exponential backoff
                            retry_delay = match response.headers()
                                .get("retry-after")
                                .and_then(|h| h.to_str().ok())
                                .and_then(|s| s.parse::<u64>().ok())
                            {
                                Some(secs) => Duration::from_secs(secs),
                                None => Duration::from_millis(100 * 2u64.pow(attempt as u32))
                            };
                            
                            debug!("Rate limited. Retrying in {:?}", retry_delay);
                            continue;
                        }
                        status => {
                            if attempt < max_retries - 1 {
                                retry_delay = Duration::from_millis(100 * 2u64.pow(attempt as u32));
                                continue;
                            }
                            return Err(ApiError::Other(
                                format!("Request failed with status: {}", status),
                            ));
                        }
                    }
                }
                Err(e) => {
                    if attempt < max_retries - 1 {
                        retry_delay = Duration::from_millis(100 * 2u64.pow(attempt as u32));
                        continue;
                    }
                    return Err(if e.is_timeout() {
                        ApiError::Timeout("Request timed out after all retries".to_string())
                    } else if e.is_connect() {
                        ApiError::Network("Failed to connect after all retries".to_string())
                    } else {
                        ApiError::Other(format!("Request failed: {}", e))
                    });
                }
            }
        }

        Err(ApiError::Other("Maximum retries exceeded".to_string()))
    }
}