use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use unremark::{Language, Cache, analyze_source};
use std::sync::Arc;
use parking_lot::RwLock;
use dashmap::DashMap;
use std::path::PathBuf;
use tokio::runtime::Runtime;

#[derive(Debug, Clone)]
struct UnremarkLanguageServer {
    client: Client,
    document_map: DashMap<String, String>,
    cache: Arc<RwLock<Cache>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for UnremarkLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL
                )),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: Some("unremark".to_string()),
                        inter_file_dependencies: false,
                        workspace_diagnostics: false,
                        ..Default::default()
                    }
                )),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "unremark server initialized").await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.document_map.insert(
            params.text_document.uri.to_string(),
            params.text_document.text,
        );
        self.analyze_document(&params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.first() {
            self.document_map.insert(
                params.text_document.uri.to_string(),
                change.text.clone(),
            );
            self.analyze_document(&params.text_document.uri).await;
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<Vec<CodeActionOrCommand>>> {
        let mut actions = Vec::new();
        
        for diagnostic in params.context.diagnostics {
            if diagnostic.source == Some("unremark".to_string()) {
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: "Remove redundant comment".to_string(),
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
        Ok(())
    }
}

impl UnremarkLanguageServer {
    async fn analyze_document(&self, uri: &Url) {
        if let Some(text) = self.document_map.get(uri.as_str()) {
            let ext = uri.path()
                .rsplit('.')
                .next()
                .and_then(Language::from_extension);

            if let Some(language) = ext {
                let analysis = analyze_source(text.as_str(), &PathBuf::from(uri.path())).await;
                let diagnostics: Vec<Diagnostic> = analysis.redundant_comments
                    .into_iter()
                    .map(|comment| Diagnostic {
                        range: Range {
                            start: Position {
                                line: comment.line_number as u32,
                                character: 0, // Start at beginning of line for now
                            },
                            end: Position {
                                line: comment.line_number as u32,
                                character: comment.text.len() as u32,
                            },
                        },
                        severity: Some(DiagnosticSeverity::HINT),
                        code: None,
                        source: Some("unremark".to_string()),
                        message: "This comment may be redundant".to_string(),
                        ..Default::default()
                    })
                    .collect();

                self.client
                    .publish_diagnostics(uri.clone(), diagnostics, None)
                    .await;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| UnremarkLanguageServer {
        client,
        document_map: DashMap::new(),
        cache: Arc::new(RwLock::new(Cache::load())),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::LanguageServer;
    use std::sync::Arc;
    use futures::executor::block_on;

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
            assert_eq!(opts.identifier, Some("unremark".to_string()));
            assert!(!opts.inter_file_dependencies);
            assert!(!opts.workspace_diagnostics);
        }

        // Check code action provider
        assert!(matches!(
            capabilities.code_action_provider,
            Some(CodeActionProviderCapability::Simple(true))
        ));
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
}