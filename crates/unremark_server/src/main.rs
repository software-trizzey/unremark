use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use unremark::{
    analyze_comments, 
    detect_comments, 
    Cache, 
    Language,
    create_analysis_service,
};
use std::sync::Arc;
use parking_lot::RwLock;
use dashmap::DashMap;
use serde_json::Value;

const VERSION_COMMAND: &str = "unremark.version";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const SERVER_ID: &str = "unremark";


#[derive(Debug, Default, serde::Deserialize)]
struct UnremarkInitializeParams {
    openai_api_key: Option<String>,
}

#[derive(Debug, Clone)]
struct UnremarkLanguageServer {
    client: Client,
    document_map: DashMap<String, String>,
    #[allow(dead_code)]
    cache: Arc<RwLock<Cache>>, // TODO: implement cache logic after we've prototyped the server
}

#[tower_lsp::async_trait]
impl LanguageServer for UnremarkLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Extract our custom initialization options
        if let Some(options) = params.initialization_options {
            if let Ok(unremark_options) = serde_json::from_value::<UnremarkInitializeParams>(options) {
                if let Some(api_key) = unremark_options.openai_api_key {
                    std::env::set_var("OPENAI_API_KEY", api_key);
                }
            }
        }

        self.client.log_message(MessageType::INFO, "Initializing server").await;
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL
                )),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some(SERVER_ID.to_string()),
                        inter_file_dependencies: false,
                        workspace_diagnostics: false,
                        work_done_progress_options: Default::default(),
                    }
                )),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![VERSION_COMMAND.to_string()],
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "Server initialized").await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.client.log_message(MessageType::INFO, format!("Document {} opened", params.text_document.uri)).await;
        self.document_map.insert(
            params.text_document.uri.to_string(),
            params.text_document.text,
        );
        let diagnostics = self.analyze_document(&params.text_document.uri).await;
        self.client.publish_diagnostics(params.text_document.uri, diagnostics, None).await;
    }

    async fn diagnostic(&self, params: DocumentDiagnosticParams) -> Result<DocumentDiagnosticReportResult> {
        self.client.log_message(MessageType::INFO, format!("Requesting diagnostics for file: {}", params.text_document.uri)).await;
        let diagnostics = self.analyze_document(&params.text_document.uri).await;
        self.client.log_message(MessageType::INFO, format!("Collected {} diagnostics", diagnostics.len())).await;
        Ok(DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(
            RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    items: diagnostics,
                    ..Default::default()
                },
            }
        )))
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.client.log_message(MessageType::INFO, 
            format!("Document change detected - version: {}", params.text_document.version)).await;
        
        if let Some(change) = params.content_changes.first() {
            let uri_str = params.text_document.uri.to_string();
            let mut current_text = if let Some(text) = self.document_map.get(&uri_str) {
                text.clone()
            } else {
                return;
            };

            // Apply the change to the document
            if let Some(range) = change.range {
                let start_pos = range.start;
                let end_pos = range.end;
                
                // Convert the positions to string indices
                let lines: Vec<&str> = current_text.lines().collect();
                let start_idx = lines[..start_pos.line as usize]
                    .iter()
                    .map(|l| l.len() + 1)
                    .sum::<usize>()
                    + start_pos.character as usize;
                let end_idx = lines[..end_pos.line as usize]
                    .iter()
                    .map(|l| l.len() + 1)
                    .sum::<usize>()
                    + end_pos.character as usize;

                // Replace the text in the range
                current_text.replace_range(start_idx..end_idx, &change.text);
            } else {
                // If no range is provided, replace the entire content
                current_text = change.text.clone();
            }

            self.document_map.insert(uri_str, current_text);
            let diagnostics = self.analyze_document(&params.text_document.uri).await;
            self.client.publish_diagnostics(params.text_document.uri, diagnostics, None).await;
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<Vec<CodeActionOrCommand>>> {
        let mut actions = Vec::new();
        
        for diagnostic in params.context.diagnostics {
            let title_text = match &diagnostic.data {
                Some(data) => data.get("text").unwrap().to_string(),
                None => diagnostic.message.clone(),
            };
            if diagnostic.source == Some(SERVER_ID.to_string()) {
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Remove redundant comment: {}", title_text),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some([(
                            params.text_document.uri.clone(),
                            vec![TextEdit {
                                range: diagnostic.range,
                                new_text: String::new(),
                            }]
                        )].into_iter().collect()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }

        Ok(Some(actions))
    }

    async fn shutdown(&self) -> Result<()> {
        self.client.log_message(MessageType::INFO, "Shutting down server").await;
        Ok(())
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        match params.command.as_str() {
            VERSION_COMMAND => {
                Ok(Some(serde_json::to_value(VERSION).unwrap()))
            }
            _ => Ok(None)
        }
    }
}

impl UnremarkLanguageServer {
    async fn analyze_document(&self, uri: &Url) -> Vec<Diagnostic> {
        if let Some(text) = self.document_map.get(uri.as_str()) {
            if let Some(language) = uri.path().rsplit('.').next().and_then(Language::from_extension) {
                let comments = detect_comments(text.as_str(), language).unwrap_or_default();
                if comments.is_empty() {
                    self.client.log_message(MessageType::LOG, "No comments found to analyze").await;
                    return vec![];
                }

                let redundant_comments = if std::env::var("OPENAI_API_KEY").is_ok() {
                    self.client.log_message(MessageType::INFO, "Local OpenAI API key found, analyzing comments locally").await;
                    analyze_comments(comments).await.unwrap_or_default()
                } else {
                    self.client.log_message(MessageType::INFO, "No OpenAI API key found, using proxy to analyze comments").await;

                    let proxy_result = create_analysis_service().analyze_comments_with_proxy(comments).await;
                    match proxy_result {
                        Ok(comments) => {
                            self.client.log_message(MessageType::INFO, 
                                format!("Proxy returned {} redundant comments", comments.len())).await;
                            comments
                        }
                        Err(e) => {
                            self.client.log_message(MessageType::ERROR, 
                                format!("Proxy analysis failed: {}", e)).await;
                            vec![]
                        }
                    }
                };

                self.client.log_message(MessageType::LOG, format!("Found {} redundant comments", redundant_comments.len())).await;

                let diagnostics: Vec<Diagnostic> = redundant_comments
                    .into_iter()
                    .map(|comment| Diagnostic {
                        range: Range {
                            start: Position {
                                line: comment.line_number as u32 - 1,
                                character: 0,
                            },
                            end: Position {
                                line: comment.line_number as u32 - 1,
                                character: comment.text.len() as u32,
                            },
                        },
                        severity: Some(DiagnosticSeverity::WARNING),
                        code: Some(NumberOrString::String("redundant-comment".to_string())),
                        source: Some(SERVER_ID.to_string()),
                        message: comment.explanation.clone().unwrap_or("This comment may be redundant".to_string()),
                        data: Some(serde_json::to_value(comment).unwrap()),
                        ..Default::default()
                    })
                    .collect();
                
                return diagnostics;
            }
        }
        vec![]
    }
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info")
    )
    .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| UnremarkLanguageServer {
        client,
        document_map: DashMap::new(),
        cache: Arc::new(RwLock::new(Cache::load())),
    });

    Server::new(stdin, stdout, socket)
        .serve(service)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::LanguageServer;
    use std::sync::Arc;
    use futures::executor::block_on;
    use tokio::runtime::Runtime;
    
    fn create_test_server() -> UnremarkLanguageServer {
        let (service, _socket) = LspService::build(|client| UnremarkLanguageServer {
            client,
            document_map: DashMap::new(),
            cache: Arc::new(RwLock::new(Cache::load())),
        })
        .finish();
        service.inner().clone()
    }

    #[test]
    fn test_server_initialization() {
        let server = create_test_server();
        let init_params = InitializeParams::default();

        let init_result = block_on(server.initialize(init_params)).unwrap();

        // Verify server capabilities
        let capabilities = init_result.capabilities;
        
        // Check text document sync
        assert!(matches!(
            capabilities.text_document_sync,
            Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL))
        ));

        // Check diagnostic provider
        assert!(capabilities.diagnostic_provider.is_some());
        if let Some(DiagnosticServerCapabilities::Options(opts)) = capabilities.diagnostic_provider {
            assert_eq!(opts.identifier, Some(SERVER_ID.to_string()));
            assert!(!opts.inter_file_dependencies);
            assert!(!opts.workspace_diagnostics);
        }

        // Check code action provider
        assert!(matches!(
            capabilities.code_action_provider,
            Some(CodeActionProviderCapability::Simple(true))
        ));

        // Check execute command provider
        assert!(capabilities.execute_command_provider.is_some());
        if let Some(ExecuteCommandOptions { commands, .. }) = capabilities.execute_command_provider {
            assert_eq!(commands, vec![VERSION_COMMAND.to_string()]);
        }
    }

    #[test]
    fn test_document_management() {
        let runtime = Runtime::new().unwrap();
        let server = create_test_server();
        let uri = Url::parse("file:///test.rs").unwrap();
        let text = "fn main() {\n    // Test comment\n}".to_string();

        // Test document opening
        runtime.block_on(server.did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "rust".to_string(),
                version: 1,
                text: text.clone(),
            },
        }));

        assert_eq!(
            server.document_map.get(uri.as_str()).unwrap().as_str(),
            text
        );

        // Test document changes
        let new_text = "fn main() {\n    // Updated comment\n}".to_string();
        runtime.block_on(server.did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 2,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: new_text.clone(),
            }],
        }));

        assert_eq!(
            server.document_map.get(uri.as_str()).unwrap().as_str(),
            new_text
        );
    }

    #[test]
    fn test_diagnostic() {
        let runtime = Runtime::new().unwrap();
        let server = create_test_server();
        let uri = Url::parse("file:///test.rs").unwrap();

        let params = DocumentDiagnosticParams {
            text_document: TextDocumentIdentifier { uri },
            identifier: None,
            previous_result_id: None,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = runtime.block_on(server.diagnostic(params)).unwrap();
        
        match result {
            DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(report)) => {
                assert!(report.related_documents.is_none());
                assert!(report.full_document_diagnostic_report.items.is_empty());
            },
            _ => panic!("Expected full diagnostic report"),
        }
    }
}