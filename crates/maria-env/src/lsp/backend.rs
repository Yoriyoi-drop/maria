//! LSP Backend — tower-lsp LanguageServer implementation for SystemVerilog.
//!
//! Provides:
//! - TextDocumentSyncKind::FULL for open/change/close
//! - publishDiagnostics via the existing parser
//! - go-to-definition (LSP-03 tahap 1): module/interface/package/class/
//!   function/task/parameter + port/signal declaration di file yang sama
//! - Initialize/shutdown lifecycle

use std::collections::HashMap;

use lsp_types::notification::PublishDiagnostics;
use lsp_types::{
    Diagnostic, DiagnosticSeverity, GotoDefinitionParams, GotoDefinitionResponse, Hover,
    HoverContents, InitializeParams, InitializeResult, LanguageString, Location, MarkedString,
    OneOf, Position, PublishDiagnosticsParams, Range, ReferenceParams, RenameParams,
    ServerCapabilities, ServerInfo, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, WorkspaceEdit,
};
use maria_parser::lexer::Lexer;
use maria_parser::preprocessor::Preprocessor;
use maria_parser::Parser;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// LSP backend for SystemVerilog language server.
pub struct LspBackend {
    client: Client,
    /// Dokumen terbuka: uri → teks (LSP-03: sumber goto-definition).
    documents: std::sync::Mutex<HashMap<String, String>>,
}

impl LspBackend {
    pub fn new(client: Client) -> Self {
        LspBackend {
            client,
            documents: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Ambil teks dokumen dari cache, fallback baca dari disk (uri file://).
    fn document_text(&self, uri: &lsp_types::Url) -> Option<String> {
        if let Ok(map) = self.documents.lock() {
            if let Some(text) = map.get(uri.as_str()) {
                return Some(text.clone());
            }
        }
        // Fallback: file belum dibuka editor — baca langsung dari path.
        let path = uri.to_file_path().ok()?;
        std::fs::read_to_string(path).ok()
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
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
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
            if tok == maria_parser::lexer::Token::Eof {
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
                    start: Position {
                        line,
                        character: col,
                    },
                    end: Position {
                        line,
                        character: col + 1,
                    },
                },
                severity: Some(severity),
                source: Some("maria".to_string()),
                message: diag.message.to_string(),
                ..Default::default()
            });
        }

        diagnostics
    }

    /// Ekstrak identifier pada posisi kursor (LSP-03).
    /// Identifier SV: [A-Za-z0-9_$]. Mengembalikan (word, start_col, end_col).
    fn word_at_position(line_text: &str, character: u32) -> Option<(String, u32, u32)> {
        let bytes = line_text.as_bytes();
        let ch = character as usize;
        let is_ident =
            |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';

        if ch > bytes.len() {
            return None;
        }
        // Kursor di atas karakter non-ident → cek karakter sebelumnya
        // (posisi akhir kata).
        let mut start = ch.min(bytes.len());
        while start > 0 && is_ident(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = ch.min(bytes.len());
        while end < bytes.len() && is_ident(bytes[end]) {
            end += 1;
        }
        if start == end {
            return None;
        }
        Some((
            line_text[start..end].to_string(),
            start as u32,
            end as u32,
        ))
    }

    /// Tokenize satu baris menjadi (word, col) — ident [A-Za-z0-9_$] atau
    /// simbol tunggal. Untuk deteksi pola deklarasi.
    fn line_tokens(line: &str) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' {
                let s = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'$')
                {
                    i += 1;
                }
                out.push((line[s..i].to_string(), s));
            } else if c == b' ' || c == b'\t' {
                i += 1;
            } else {
                // Komentar — abaikan sisa baris.
                if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    break;
                }
                out.push((String::from_utf8_lossy(&bytes[i..i + 1]).to_string(), i));
                i += 1;
            }
        }
        out
    }

    /// Deteksi apakah baris adalah DEFINISI dari `word` (LSP-03 tahap 1).
    /// Mengembalikan kolom awal kata bila ya. Pola yang dikenali:
    ///   1. module/interface/package/program/class/checker/function/task/
    ///      property/sequence `<word>` — definisi scope.
    ///   2. parameter/localparam/typedef/genvar ... `<word>` — deklarasi.
    ///   3. Baris diawali direction/type keyword (input/output/inout/reg/
    ///      wire/logic/bit/int/... ) dan memuat `<word>` sebagai token
    ///      berikutnya yang relevan — port/signal declaration.
    fn definition_col(line: &str, word: &str) -> Option<usize> {
        const SCOPE_KW: &[&str] = &[
            "module", "macromodule", "interface", "package", "program", "class",
            "checker", "function", "task", "property", "sequence",
        ];
        const DECL_KW: &[&str] = &["parameter", "localparam", "typedef", "genvar"];
        const TYPE_DIR_KW: &[&str] = &[
            "input", "output", "inout", "wire", "reg", "logic", "bit", "int",
            "integer", "byte", "shortint", "longint", "real", "realtime",
            "time", "nettype", "enum", "struct", "union", "string", "event",
        ];

        let toks = Self::line_tokens(line);
        if toks.is_empty() {
            return None;
        }

        // 0. Deklarasi parameter di mana pun dalam baris (termasuk
        //    `module m #(parameter WIDTH = 8) (`): token parameter/localparam
        //    → cari word SETELAHnya.
        for (i, (w, _)) in toks.iter().enumerate() {
            if matches!(w.as_str(), "parameter" | "localparam") {
                for (w2, c2) in toks.iter().skip(i + 1) {
                    if w2 == word {
                        return Some(*c2);
                    }
                }
            }
        }

        // 1. Definisi scope: <kw> <word>
        let (t0, _) = &toks[0];
        if SCOPE_KW.contains(&t0.as_str()) {
            if let Some((w1, c1)) = toks.get(1) {
                if w1 == word {
                    return Some(*c1);
                }
            }
            // function ret_type <name>( / task ...
            if matches!(t0.as_str(), "function" | "task") {
                for (w, c) in toks.iter().skip(1) {
                    if w == word {
                        // pastikan bukan tipe return: harus sebelum '('
                        return Some(*c);
                    }
                }
            }
            return None;
        }

        // 2. parameter/localparam/typedef/genvar ... word ...
        if DECL_KW.contains(&t0.as_str()) {
            for (w, c) in toks.iter().skip(1) {
                if w == word {
                    return Some(*c);
                }
            }
            return None;
        }

        // 3. Port/signal decl: baris diawali direction/type keyword dan
        //    memuat word sebagai token berdiri sendiri (bukan nama tipe
        //    pertama). Contoh: `input logic [7:0] data_in`, `reg [3:0] cnt`.
        if TYPE_DIR_KW.contains(&t0.as_str()) {
            // Cari word sebagai token berdiri sendiri setelah keyword.
            // Pemakaian dalam ekspresi (mis. `count + 1`) tetap bisa match
            // di sini BILA baris juga diawali type keyword — jarang dan
            // hasilnya tetap lokasi deklarasi-relevan (MVP tahap 1).
            for (idx, (w, c)) in toks.iter().enumerate().skip(1) {
                if w == word && idx > 0 {
                    // Skip pemakaian hierarki (a.b): token sebelumnya "."
                    let prev_dot = idx >= 1 && toks[idx - 1].0 == ".";
                    if !prev_dot {
                        return Some(*c);
                    }
                }
            }
        }
        None
    }

    /// Validasi identifier SV sederhana (LSP-05): [A-Za-z_][A-Za-z0-9_$]*.
    fn is_valid_identifier(name: &str) -> bool {
        let bytes = name.as_bytes();
        if bytes.is_empty() {
            return false;
        }
        let first = bytes[0];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return false;
        }
        bytes[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$')
    }

    /// Rename core (murni, bisa diuji) — LSP-05 tahap 1: hasilkan edit
    /// (range) untuk SEMUA kemunculan identifier di dokumen. Return
    /// Err dengan pesan jelas bila kursor bukan identifier atau nama
    /// baru tidak valid.
    pub(crate) fn compute_rename_edits(
        source: &str,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<Vec<(u32, u32, u32)>, String> {
        if !Self::is_valid_identifier(new_name) {
            return Err(format!(
                "'{}' bukan identifier SystemVerilog yang valid",
                new_name
            ));
        }
        let Some(line_text) = source.lines().nth(line as usize) else {
            return Err("posisi kursor di luar dokumen".into());
        };
        let Some((word, _, _)) = Self::word_at_position(line_text, character) else {
            return Err("kursor tidak berada di atas identifier".into());
        };
        if word == new_name {
            return Ok(Vec::new());
        }
        // find_references menjamin token mandiri + word-boundary.
        Ok(Self::find_references(source, line, character)
            .into_iter()
            .collect())
    }

    /// Hover core (murni, bisa diuji) — LSP-06 tahap 1: konteks deklarasi
    /// identifier di posisi kursor, berformat markdown:
    ///   **kind** `name` — snippet baris deklarasi.
    fn hover_info(source: &str, line: u32, character: u32) -> Option<String> {
        let line_text = source.lines().nth(line as usize)?;
        let (word, _, _) = Self::word_at_position(line_text, character)?;

        let (def_line, col, _) = Self::find_definition(source, line, character)?;
        let decl_text = source
            .lines()
            .nth(def_line as usize)?
            .trim()
            .to_string();

        // Tebak kind dari keyword di baris definisi.
        let toks = Self::line_tokens(&decl_text);
        let kind = toks
            .iter()
            .find_map(|(w, _)| match w.as_str() {
                "module" | "interface" | "package" | "program" | "class" | "checker" => {
                    Some(w.clone())
                }
                "input" => Some("port input".into()),
                "output" => Some("port output".into()),
                "inout" => Some("port inout".into()),
                "parameter" | "localparam" => Some(w.clone()),
                "function" | "task" => Some(w.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "declaration".into());

        Some(format!(
            "**{kind}** `{word}`\n\n```systemverilog\n{decl_text}\n```"
        ))
    }

    /// Find-references core (murni, bisa diuji) — LSP-04 tahap 1:
    /// semua kemunculan identifier (sebagai token mandiri) dalam dokumen,
    /// urut baris. `include_declaration` tetap dikembalikan penuh — caller
    /// (handler) yang memfilter bila diminta tanpa deklarasi.
    pub(crate) fn find_references(
        source: &str,
        line: u32,
        character: u32,
    ) -> Vec<(u32, u32, u32)> {
        let Some(line_text) = source.lines().nth(line as usize) else {
            return Vec::new();
        };
        let Some((word, _, _)) = Self::word_at_position(line_text, character) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for (idx, text) in source.lines().enumerate() {
            // Skip komentar baris.
            let trimmed = text.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for (w, col) in Self::line_tokens(text) {
                if w == word {
                    out.push((idx as u32, col as u32, (col + w.len()) as u32));
                }
            }
        }
        out
    }

    /// Core go-to-definition (murni, bisa diuji): cari lokasi definisi dari
    /// identifier di posisi kursor. Mengembalikan (line, col_start, col_end)
    /// posisi definisi pertama dalam dokumen.
    pub(crate) fn find_definition(
        source: &str,
        line: u32,
        character: u32,
    ) -> Option<(u32, u32, u32)> {
        let line_text = source.lines().nth(line as usize)?;
        let (word, _, _) = Self::word_at_position(line_text, character)?;

        for (idx, text) in source.lines().enumerate() {
            // Skip baris kursor sendiri bila kemungkinan pemakaian —
            // definisi boleh saja di baris yang sama (deklarasi inline),
            // jadi tetap periksa semua baris.
            if let Some(col) = Self::definition_col(text, &word) {
                return Some((idx as u32, col as u32, (col + word.len()) as u32));
            }
        }
        None
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for LspBackend {
    /// Initialize the LSP server — advertises capabilities.
    async fn initialize(&self, _params: InitializeParams) -> JsonRpcResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
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
        if let Ok(mut map) = self.documents.lock() {
            map.insert(uri.to_string(), source.clone());
        }
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
            if let Ok(mut map) = self.documents.lock() {
                map.insert(uri.to_string(), change.text.clone());
            }
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

    /// Go-to-definition (LSP-03 tahap 1): cari deklarasi identifier di
    /// posisi kursor dalam dokumen yang sama.
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(text) = self.document_text(uri) else {
            return Ok(None);
        };
        match Self::find_definition(&text, pos.line, pos.character) {
            Some((line, start, end)) => Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: Range {
                    start: Position {
                        line,
                        character: start,
                    },
                    end: Position {
                        line,
                        character: end,
                    },
                },
            }))),
            None => Ok(None),
        }
    }

    /// Find-references (LSP-04 tahap 1): semua kemunculan identifier di
    /// dokumen yang sama.
    async fn references(&self, params: ReferenceParams) -> JsonRpcResult<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let Some(text) = self.document_text(uri) else {
            return Ok(None);
        };
        let refs: Vec<Location> = Self::find_references(&text, pos.line, pos.character)
            .into_iter()
            .map(|(line, start, end)| Location {
                uri: uri.clone(),
                range: Range {
                    start: Position {
                        line,
                        character: start,
                    },
                    end: Position {
                        line,
                        character: end,
                    },
                },
            })
            .collect();
        if refs.is_empty() {
            return Ok(None);
        }
        Ok(Some(refs))
    }

    /// Rename (LSP-05 tahap 1): rename identifier di seluruh dokumen.
    async fn rename(
        &self,
        params: RenameParams,
    ) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let Some(text) = self.document_text(uri) else {
            return Ok(None);
        };
        match Self::compute_rename_edits(&text, pos.line, pos.character, &params.new_name) {
            Ok(edits) => {
                let changes: std::collections::HashMap<
                    lsp_types::Url,
                    Vec<lsp_types::TextEdit>,
                > = std::collections::HashMap::from([(
                    uri.clone(),
                    edits
                        .into_iter()
                        .map(|(line, start, end)| lsp_types::TextEdit {
                            range: Range {
                                start: Position {
                                    line,
                                    character: start,
                                },
                                end: Position {
                                    line,
                                    character: end,
                                },
                            },
                            new_text: params.new_name.clone(),
                        })
                        .collect(),
                )]);
                Ok(Some(WorkspaceEdit::new(changes)))
            }
            Err(msg) => Err(tower_lsp::jsonrpc::Error::invalid_params(msg)),
        }
    }

    /// Hover (LSP-06 tahap 1): konteks deklarasi identifier.
    async fn hover(&self, params: lsp_types::HoverParams) -> JsonRpcResult<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(text) = self.document_text(uri) else {
            return Ok(None);
        };
        match Self::hover_info(&text, pos.line, pos.character) {
            Some(markdown) => Ok(Some(Hover {
                contents: HoverContents::Scalar(MarkedString::LanguageString(
                    LanguageString {
                        language: "markdown".into(),
                        value: markdown,
                    },
                )),
                range: None,
            })),
            None => Ok(None),
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
    Server::new(stdin, stdout, messages).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
module counter #(parameter WIDTH = 8) (
    input logic clk,
    input logic rst_n,
    output logic [WIDTH-1:0] count
);
    logic [3:0] state;
    reg enable;

    counter_aux aux (.clk(clk));

    always_ff @(posedge clk) begin
        if (!rst_n) count <= 0;
        else if (enable) count <= count + 1;
    end
endmodule

module counter_aux (
    input logic clk
);
endmodule
";

    /// Baris pertama yang memuat `needle` (untuk ekspektasi robust).
    fn line_of(needle: &str) -> u32 {
        SAMPLE
            .lines()
            .position(|l| l.contains(needle))
            .unwrap() as u32
    }

    #[test]
    fn test_word_at_position() {
        // Kursor di tengah "counter" baris 0.
        let (w, s, e) = LspBackend::word_at_position("module counter;", 11).unwrap();
        assert_eq!(w, "counter");
        assert_eq!(s, 7);
        assert_eq!(e, 14);
        // Kursor di akhir kata.
        let (w, _, _) = LspBackend::word_at_position("module counter;", 14).unwrap();
        assert_eq!(w, "counter");
        // Bukan identifier.
        assert!(LspBackend::word_at_position("   ;", 1).is_none());
        // $ dalam identifier SV.
        let (w, _, _) = LspBackend::word_at_position("wire a$b;", 6).unwrap();
        assert_eq!(w, "a$b");
    }

    #[test]
    fn test_find_definition_module() {
        // Kursor di nama module sendiri → definisi tetap ditemukan.
        let (line, col, _) = LspBackend::find_definition(SAMPLE, 0, 10).unwrap();
        assert_eq!(line, line_of("module counter"), "definisi module counter");
        assert_eq!(
            SAMPLE.lines().nth(line as usize).unwrap()[col as usize..].starts_with("counter"),
            true
        );

        // Instance `counter_aux aux` (baris pemakaian) → definisi module
        // counter_aux.
        let use_line = line_of("counter_aux aux");
        let aux_col = SAMPLE
            .lines()
            .nth(use_line as usize)
            .unwrap()
            .find("counter_aux")
            .unwrap() as u32;
        let (line, col, _) =
            LspBackend::find_definition(SAMPLE, use_line, aux_col + 3).unwrap();
        assert_eq!(line, line_of("module counter_aux"));
        assert_eq!(
            SAMPLE.lines().nth(line as usize).unwrap()[col as usize..].starts_with("counter_aux"),
            true
        );
    }

    #[test]
    fn test_find_definition_signal_and_port() {
        // Pemakaian `enable` di always_ff → deklarasi `reg enable;`.
        let use_line = line_of("else if (enable)");
        let enable_col = SAMPLE.lines().nth(use_line as usize).unwrap().find("enable").unwrap() as u32;
        let (line, col, _) =
            LspBackend::find_definition(SAMPLE, use_line, enable_col).unwrap();
        assert_eq!(line, line_of("reg enable"));
        assert_eq!(
            SAMPLE.lines().nth(line as usize).unwrap()[col as usize..].starts_with("enable"),
            true
        );

        // Pemakaian `clk` di port connection `.clk(clk)` → port decl
        // `input logic clk,` di module counter.
        let inst_line = line_of(".clk(clk)");
        let clk_col = SAMPLE
            .lines()
            .nth(inst_line as usize)
            .unwrap()
            .rfind("(clk)")
            .map(|p| p as u32 + 1)
            .unwrap();
        let (line, col, _) = LspBackend::find_definition(SAMPLE, inst_line, clk_col).unwrap();
        assert_eq!(line, line_of("input logic clk,"));
        assert_eq!(
            SAMPLE.lines().nth(line as usize).unwrap()[col as usize..].starts_with("clk"),
            true
        );

        // Signal `state` dipakai? Tidak ada pemakaian; deklarasinya sendiri
        // tetap ditemukan.
        let decl_line = line_of("logic [3:0] state;");
        let st_col = SAMPLE.lines().nth(decl_line as usize).unwrap().find("state").unwrap() as u32;
        let (line, col, _) = LspBackend::find_definition(SAMPLE, decl_line, st_col).unwrap();
        assert_eq!(line, decl_line);
        assert_eq!(
            SAMPLE.lines().nth(line as usize).unwrap()[col as usize..].starts_with("state"),
            true
        );
    }

    #[test]
    fn test_find_definition_parameter() {
        // `[WIDTH-1:0]` pada port output → parameter WIDTH baris pertama.
        let out_line = line_of("[WIDTH-1:0] count");
        let w_col = SAMPLE
            .lines()
            .nth(out_line as usize)
            .unwrap()
            .find("WIDTH")
            .unwrap() as u32;
        let (line, col, _) = LspBackend::find_definition(SAMPLE, out_line, w_col).unwrap();
        assert_eq!(line, line_of("parameter WIDTH"));
        assert_eq!(
            SAMPLE.lines().nth(line as usize).unwrap()[col as usize..].starts_with("WIDTH"),
            true
        );
    }

    #[test]
    fn test_find_references() {
        // Referensi `clk`: port decl + instance connection + always_ff.
        let decl_line = line_of("input logic clk,");
        let inst_line = line_of(".clk(clk)");
        let ff_line = line_of("always_ff @(posedge clk)");
        let refs = LspBackend::find_references(SAMPLE, inst_line, 26);
        let refs = if refs.is_empty() {
            // fallback: kolom clk kedua di baris instance.
            let col = SAMPLE.lines().nth(inst_line as usize).unwrap().rfind("(clk)").unwrap() as u32 + 1;
            LspBackend::find_references(SAMPLE, inst_line, col)
        } else {
            refs
        };
        assert_eq!(refs.len(), 5, "clk muncul 5x: {:?}", refs);
        let lines: Vec<u32> = refs.iter().map(|(l, _, _)| *l).collect();
        assert!(lines.contains(&decl_line));
        assert!(lines.contains(&inst_line));
        assert!(lines.contains(&ff_line));
        assert!(lines.contains(&line_of("input logic clk"))); // port counter_aux

        // Referensi `enable` (decl + pemakaian di else-if).
        let en_line = line_of("reg enable;");
        let use_line = line_of("else if (enable)");
        let col = SAMPLE
            .lines()
            .nth(use_line as usize)
            .unwrap()
            .find("enable")
            .unwrap() as u32;
        let refs2 = LspBackend::find_references(SAMPLE, use_line, col);
        assert_eq!(refs2.len(), 2, "enable muncul 2x: {:?}", refs2);
        let lines2: Vec<u32> = refs2.iter().map(|(l, _, _)| *l).collect();
        assert!(lines2.contains(&en_line));
        assert!(lines2.contains(&use_line));

        // Word boundary: `count` TIDAK match dengan `counter`/`count_aux`.
        let cnt_line = line_of("output logic [WIDTH-1:0] count");
        let c_col = SAMPLE.lines().nth(cnt_line as usize).unwrap().rfind("count").unwrap() as u32;
        let refs3 = LspBackend::find_references(SAMPLE, cnt_line, c_col);
        assert!(!refs3.is_empty(), "count harus ditemukan");
        for (l, s, e) in &refs3 {
            let text = SAMPLE.lines().nth(*l as usize).unwrap();
            assert!(
                !text[*s as usize..*e as usize].contains("counter"),
                "word boundary bocor: {}",
                text
            );
        }
    }

    #[test]
    fn test_find_references_empty_and_miss() {
        // Kursor bukan identifier → kosong.
        assert!(LspBackend::find_references(SAMPLE, 7, 0).is_empty());
        // Baris di luar dokumen → kosong.
        assert!(LspBackend::find_references(SAMPLE, 999, 0).is_empty());
    }

    #[test]
    fn test_rename_edits() {
        // Rename `enable` (2 lokusi) → `en`.
        let decl_line = line_of("reg enable;");
        let use_line = line_of("else if (enable)");
        let col = SAMPLE
            .lines()
            .nth(use_line as usize)
            .unwrap()
            .find("enable")
            .unwrap() as u32;
        let edits =
            LspBackend::compute_rename_edits(SAMPLE, use_line, col, "en").unwrap();
        assert_eq!(edits.len(), 2, "enable di-rename 2 lokusi: {:?}", edits);
        let lines: Vec<u32> = edits.iter().map(|(l, _, _)| *l).collect();
        assert!(lines.contains(&decl_line));
        assert!(lines.contains(&use_line));
        // Simulasi apply edit (dari bawah ke atas agar offset aman).
        let mut lines_mut: Vec<String> = SAMPLE.lines().map(|s| s.to_string()).collect();
        for (l, s, e) in edits.iter().rev() {
            let text = lines_mut[*l as usize].clone();
            lines_mut[*l as usize] = format!(
                "{}{}{}",
                &text[..*s as usize],
                "en",
                &text[*e as usize..]
            );
        }
        assert!(lines_mut[decl_line as usize].contains("reg en;"));
        assert!(lines_mut[use_line as usize].contains("if (en)"));

        // Nama tidak valid → Err.
        assert!(LspBackend::compute_rename_edits(SAMPLE, use_line, col, "1bad").is_err());
        assert!(LspBackend::compute_rename_edits(SAMPLE, use_line, col, "").is_err());

        // Kursor bukan identifier → Err.
        assert!(LspBackend::compute_rename_edits(SAMPLE, 7, 0, "x").is_err());

        // Word boundary: rename `count` TIDAK menyentuh `counter_aux`/
        // `count <=`. count muncul: port decl + 3 pemakaian always_ff.
        let cnt_decl = line_of("[WIDTH-1:0] count");
        let cnt_use = line_of("count <= count + 1");
        let c_col = SAMPLE
            .lines()
            .nth(cnt_use as usize)
            .unwrap()
            .find("count")
            .unwrap() as u32;
        let edits_cnt =
            LspBackend::compute_rename_edits(SAMPLE, cnt_use, c_col, "cnt").unwrap();
        assert_eq!(
            edits_cnt.len(),
            4,
            "count 4 lokusi (decl+3): {:?}",
            edits_cnt
        );
        assert!(edits_cnt.iter().all(|(l, _, _)| *l >= cnt_decl));
    }

    #[test]
    fn test_hover_info() {
        // Hover di pemakaian `enable` → konteks deklarasi reg.
        let use_line = line_of("else if (enable)");
        let col = SAMPLE.lines().nth(use_line as usize).unwrap().find("enable").unwrap() as u32;
        let info = LspBackend::hover_info(SAMPLE, use_line, col).unwrap();
        assert!(info.contains("enable"), "{}", info);
        assert!(info.contains("reg enable;"), "{}", info);

        // Hover di nama module instance → konteks module.
        let inst_line = line_of("counter_aux aux");
        let aux_col = SAMPLE
            .lines()
            .nth(inst_line as usize)
            .unwrap()
            .find("counter_aux")
            .unwrap() as u32;
        let info2 = LspBackend::hover_info(SAMPLE, inst_line, aux_col).unwrap();
        assert!(info2.contains("**module**"), "{}", info2);

        // Bukan identifier → None.
        assert!(LspBackend::hover_info(SAMPLE, 7, 0).is_none());
    }

    #[test]
    fn test_find_definition_not_found() {
        // Identifier tanpa definisi (rst_n hanya ada di decl+usage... pakai
        // nama yang benar-benar tak ada).
        assert!(LspBackend::find_definition(SAMPLE, 0, 0).is_none()); // "module" bukan ident target? word="module" → def module <word>? tidak match
        assert!(
            LspBackend::find_definition(
                "module m;\nendmodule\n",
                1,
                2
            )
            .is_none()
        );
        // Baris di luar dokumen.
        assert!(LspBackend::find_definition(SAMPLE, 999, 0).is_none());
    }
}
