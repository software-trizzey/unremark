/* This file is a WIP and is not yet ready for use. */

use tower_lsp::Client;
use tower_lsp::lsp_types::*;
use std::collections::HashMap;

use unremark::{Language, detect_comments, CommentInfo, analyze_comments, create_analysis_service};

#[derive(Debug, Clone)]
pub struct Document {
    text: String,
    language: Language,
    comments: Vec<CommentInfo>,
    analysis: HashMap<usize, Analysis>,
    version: i32,
}

#[derive(Debug, Clone)]
struct Analysis {
    is_redundant: bool,
    explanation: Option<String>,
    version: i32,  // Track when this analysis was performed
}

impl Document {
    pub fn new(text: String, language: Language, version: i32) -> Self {
        let mut doc = Self {
            text,
            language,
            comments: Vec::new(),
            analysis: HashMap::new(),
            version,
        };
        doc.detect_comments();
        doc
    }

    pub fn apply_change(&mut self, change: &TextDocumentContentChangeEvent, new_version: i32) {
        if let Some(range) = change.range {
            let lines: Vec<&str> = self.text.lines().collect();
            let mut new_lines: Vec<String> = lines.iter().map(|&s| s.to_string()).collect();
            
            if range.start.line == range.end.line {
                if range.start.character == 0 && range.end.character == 0 {
                    // Line insertion
                    new_lines.insert(range.start.line as usize, change.text.trim_end().to_string());
                } else {
                    // Line replacement
                    new_lines[range.start.line as usize] = change.text.clone();
                }
            }
            
            self.text = new_lines.join("\n");
        } else {
            self.text = change.text.clone();
        }
        self.version = new_version;
        self.detect_comments();
        self.invalidate_affected_analysis(change.range);
    }

    fn detect_comments(&mut self) {
        self.comments = detect_comments(&self.text, self.language)
            .unwrap_or_default();
    }

    pub async fn get_diagnostics(&mut self, client: &Client) -> Vec<Diagnostic> {
        client.log_message(MessageType::INFO, format!("Getting diagnostics for document:")).await;
        let mut diagnostics = Vec::new();
        
        let unanalyzed = self.comments.iter()
            .filter(|c| !self.analysis.contains_key(&c.line_number));

        if let Some(comments) = self.analyze_comments(unanalyzed, client).await {
            for (comment, analysis) in comments {
                self.analysis.insert(comment.line_number, analysis);
            }
        }

        client.log_message(MessageType::INFO, format!("Analyzed {} comments", self.analysis.len())).await;

        for comment in &self.comments {
            if let Some(analysis) = self.analysis.get(&comment.line_number) {
                if analysis.is_redundant {
                    diagnostics.push(Diagnostic {
                        range: Range {
                            start: Position {
                                line: (comment.line_number - 1) as u32,
                                character: 0,
                            },
                            end: Position {
                                line: (comment.line_number - 1) as u32,
                                character: comment.text.len() as u32,
                            },
                        },
                        severity: Some(DiagnosticSeverity::WARNING),
                        code: Some(NumberOrString::String("redundant-comment".to_string())),
                        source: Some("unremark".to_string()),
                        message: analysis.explanation.clone()
                            .unwrap_or_else(|| "This comment may be redundant".to_string()),
                        data: Some(serde_json::to_value(comment).unwrap()),
                        ..Default::default()
                    });
                }
            }
        }

        client.log_message(MessageType::INFO, format!("Generated {} diagnostics", diagnostics.len())).await;

        diagnostics
    }

    fn invalidate_affected_analysis(&mut self, range: Option<Range>) {
        if let Some(range) = range {
            self.analysis.retain(|line, _| {
                let line = *line as u32;
                line < range.start.line || line > range.end.line
            });
        } else {
            self.analysis.clear();
        }
    }

    async fn analyze_comments<'a, I>(&self, comments: I, client: &Client) -> Option<Vec<(CommentInfo, Analysis)>>
    where
        I: Iterator<Item = &'a CommentInfo>,
    {
        let comments: Vec<_> = comments.cloned().collect();
        if comments.is_empty() {
            return None;
        }

        let analyzed = if std::env::var("OPENAI_API_KEY").is_ok() {
            client.log_message(MessageType::INFO, "Analyzing comments with OpenAI").await;
            analyze_comments(comments.clone()).await.unwrap_or_default()
        } else {
            client.log_message(MessageType::INFO, "Analyzing comments with proxy").await;
            match create_analysis_service().analyze_comments_with_proxy(comments.clone()).await {
                Ok(results) => {
                    client.log_message(MessageType::INFO, 
                        format!("Successfully received {} analyzed comments from proxy", results.len())).await;
                    results
                }
                Err(e) => {
                    client.log_message(MessageType::ERROR, 
                        format!("Failed to analyze comments with proxy: {}", e)).await;
                    return None;
                }
            }
        };

        if analyzed.is_empty() {
            client.log_message(MessageType::WARNING, "No comments were analyzed").await;
            return None;
        }

        Some(analyzed.into_iter()
            .map(|c| (c.clone(), Analysis {
                is_redundant: true,
                explanation: c.explanation.clone(),
                version: self.version,
            }))
            .collect())
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::{Client, LanguageServer, jsonrpc, LspService};
    use crate::SERVER_ID;

    #[derive(Debug)]
    struct MockClient(Client);

    #[tower_lsp::async_trait]
    impl LanguageServer for MockClient {
        async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
            unimplemented!()
        }

        async fn shutdown(&self) -> jsonrpc::Result<()> {
            unimplemented!()
        }
    }

    fn create_test_client() -> Client {
        let (service, _) = LspService::new(|client| {
            MockClient(client.clone())
        });
        service.inner().0.clone()
    }

    fn create_test_document() -> Document {
        let text = "fn main() {\n    // Test comment\n    println!(\"Hello\");\n}".to_string();
        Document::new(text, Language::Rust, 1)
    }

    #[test]
    fn test_document_creation() {
        let doc = create_test_document();
        assert!(!doc.comments.is_empty(), "Should detect comments");
        assert_eq!(doc.version, 1);
        
        let comment = &doc.comments[0];
        assert_eq!(comment.text.trim(), "// Test comment");
        assert_eq!(comment.line_number, 2);
    }

    #[test]
    fn test_incremental_change() {
        let mut doc = create_test_document();
        
        let comment_text = "    // Updated comment";  // Include indentation
        doc.apply_change(&TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position { line: 1, character: 0 },  // Start from beginning of line
                end: Position { line: 1, character: 17 }
            }),
            range_length: None,
            text: comment_text.to_string(),
        }, 2);

        assert_eq!(doc.version, 2);
        assert_eq!(doc.comments[0].text.trim(), "// Updated comment");  // Still trim for comparison
    }

    #[test]
    fn test_line_addition() {
        let mut doc = create_test_document();
        let initial_comment_line = doc.comments[0].line_number;
        
        // Add a line before the comment
        doc.apply_change(&TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 }
            }),
            range_length: None,
            text: "// Another comment\n".to_string(),
        }, 2);

        // Verify comment position is updated
        assert_eq!(doc.comments[1].line_number, initial_comment_line + 1);
    }

    #[test]
    fn test_full_document_update() {
        let mut doc = create_test_document();
        
        let comment_text = "// Single comment";
        doc.apply_change(&TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: comment_text.to_string(),
        }, 2);

        assert_eq!(doc.version, 2);
        assert_eq!(doc.comments.len(), 1);
        assert_eq!(doc.comments[0].text.trim(), comment_text);
    }

    #[tokio::test]
    async fn test_diagnostics_generation() {
        let mut doc = create_test_document();
        let client = create_test_client();
        let diagnostics = doc.get_diagnostics(&client).await;
        
        if !diagnostics.is_empty() {
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.source, Some(SERVER_ID.to_string()));
            assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(diagnostic.code, Some(NumberOrString::String("redundant-comment".to_string())));
        }
    }
}