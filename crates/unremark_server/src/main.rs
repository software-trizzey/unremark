use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use unremark::{Language, analyze_file, Cache};
use std::sync::Arc;
use parking_lot::RwLock;
use dashmap::DashMap;

#[derive(Debug)]
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

            if let Some(_language) = ext {
                // TODO: Implement document analysis using unremark core
                // For now, just publish a sample diagnostic
                self.client
                    .publish_diagnostics(
                        uri.clone(),
                        vec![Diagnostic {
                            range: Range {
                                start: Position { line: 0, character: 0 },
                                end: Position { line: 0, character: 10 },
                            },
                            severity: Some(DiagnosticSeverity::HINT),
                            code: None,
                            source: Some("unremark".to_string()),
                            message: "Sample diagnostic".to_string(),
                            ..Default::default()
                        }],
                        None,
                    )
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