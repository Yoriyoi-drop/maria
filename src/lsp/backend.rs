//! LSP Backend — tower-lsp LanguageServer implementation for SystemVerilog.
//!
//! Provides:
//! - TextDocumentSyncKind::Incremental for open/change/close
//! - publishDiagnostics via the existing parser
//! - Initialize/shutdown lifecycle

use crate::parser::lexer::Lexer;
use crate::parser::preprocessor::Preprocessor;
use crate::parser::Parser;
use lsp_types::notification::PublishDiagnostics;
use lsp_types::{
    Diagnostic, DiagnosticSeverity, InitializeParams, InitializeResult,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, ServerInfo,
};
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// LSP backend for SystemVerilog language server.
pub struct LspBackend {
    client: Client,
}

impl LspBackend {
    pub fn new(client: Client) -> Self {
        LspBackend { client }
    }

    /// Parse SystemVerilog source text and return diagnostics.
    fn parse_diagnostics(source: &str) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // 1. Preprocess
        let mut pp = Preprocessor::new();
        pp.quiet = true;
        let preprocessed = match pp.preprocess(source, None) {
            Ok(s) => s,
            Err(e) => {
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position { line: 0, character: 0 },
                        end: Position { line: 0, character: 0 },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("maria".to_string()),
                    message: format!("Preprocessor error: {}", e),
                    ..Default::default()
                });
                return diagnostics;
            }
        };

        // 2. Lex
        let mut lexer = Lexer::new(&preprocessed);
        let mut tokens = Vec::new();
        loop {
            let (tok, line, col) = lexer.next_token();
            if tok == crate::parser::lexer::Token::Eof {
                break;
            }
            tokens.push((tok, line, col));
        }

        if tokens.is_empty() {
            return diagnostics;
        }

        // 3. Parse with diagnostics collection
        let file_line_map = lexer.file_line_map.clone();
        let file_path = "<buffer>".to_string();
        let mut parser = Parser::new(tokens, &file_path)
            .with_source_lines(&preprocessed)
            .with_file_line_map(file_line_map);

        let _design = parser.parse_design();

        // 4. Convert parser diagnostics to LSP diagnostics
        for diag in &parser.errors {
            let severity = if diag.is_error() {
                DiagnosticSeverity::ERROR
            } else {
                DiagnosticSeverity::WARNING
            };

            // Get line/col from the diagnostic's spans or source snippet
            let (line, col) = if let Some(snippet) = &diag.source_snippet {
                ((snippet.line - 1) as u32, (snippet.col - 1) as u32)
            } else {
                (0, 0)
            };

            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position { line, character: col },
                    end: Position { line, character: col + 1 },
                },
                severity: Some(severity),
                source: Some("maria".to_string()),
                message: diag.message.to_string(),
                ..Default::default()
            });
        }

        diagnostics
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for LspBackend {
    /// Initialize the LSP server — advertises capabilities.
    async fn initialize(&self, _params: InitializeParams) -> JsonRpcResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(    TextDocumentSyncCapability::Kind(
        TextDocumentSyncKind::FULL,
    )),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "maria-lsp".to_string(),
                version: Some("0.3.0".to_string()),
            }),
        })
    }

    /// Called when the server is shut down.
    async fn shutdown(&self) -> JsonRpcResult<()> {
        Ok(())
    }

    /// Text document didOpen — parse and emit diagnostics.
    async fn did_open(&self, params: lsp_types::DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let source = &params.text_document.text;
        let diagnostics = Self::parse_diagnostics(source);
        self.client
            .send_notification::<PublishDiagnostics>(PublishDiagnosticsParams {
                uri,
                diagnostics,
                version: Some(1),
            })
            .await;
    }

    /// Text document didChange — re-parse and emit diagnostics.
    async fn did_change(&self, params: lsp_types::DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().last() {
            let diagnostics = Self::parse_diagnostics(&change.text);
            self.client
                .send_notification::<PublishDiagnostics>(PublishDiagnosticsParams {
                    uri,
                    diagnostics,
                    version: Some(params.text_document.version),
                })
                .await;
        }
    }

    /// Text document didSave — re-parse and emit diagnostics.
    async fn did_save(&self, params: lsp_types::DidSaveTextDocumentParams) {
        if let Some(source) = &params.text {
            let uri = params.text_document.uri;
            let diagnostics = Self::parse_diagnostics(source);
            self.client
                .send_notification::<PublishDiagnostics>(PublishDiagnosticsParams {
                    uri,
                    diagnostics,
                    version: None,
                })
                .await;
        }
    }

    /// Text document didClose — clear diagnostics.
    async fn did_close(&self, params: lsp_types::DidCloseTextDocumentParams) {
        self.client
            .send_notification::<PublishDiagnostics>(PublishDiagnosticsParams {
                uri: params.text_document.uri,
                diagnostics: vec![],
                version: None,
            })
            .await;
    }
}

/// Run the LSP server with stdio transport.
pub async fn run_lsp_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, messages) = LspService::new(LspBackend::new);
    Server::new(stdin, stdout, messages)
        .serve(service)
        .await;
}
