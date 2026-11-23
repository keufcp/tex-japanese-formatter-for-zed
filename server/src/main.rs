mod config;
mod format;

use std::collections::HashMap;

use config::FormatterConfig;
use format::FormatOptions;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug, Clone, Default)]
struct DocumentState {
    text: String,
    language_id: Option<String>,
}

#[derive(Debug)]
struct Backend {
    client: Client,
    config: RwLock<FormatterConfig>,
    documents: RwLock<HashMap<Url, DocumentState>>,
}

impl Backend {
    async fn update_config(&self, value: serde_json::Value) {
        if let Some(parsed) = FormatterConfig::from_value(&value) {
            *self.config.write().await = parsed;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(init_options) = params.initialization_options {
            self.update_config(init_options).await;
        }

        let capabilities = ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            document_formatting_provider: Some(OneOf::Left(true)),

            ..ServerCapabilities::default()
        };

        Ok(InitializeResult {
            capabilities,
            server_info: Some(ServerInfo {
                name: "tex-japanese-formatter-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "tex-japanese-formatter-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.update_config(params.settings).await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let document = params.text_document;
        let mut documents = self.documents.write().await;
        documents.insert(
            document.uri,
            DocumentState {
                text: document.text,
                language_id: Some(document.language_id),
            },
        );
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut documents = self.documents.write().await;
        let entry = documents.entry(params.text_document.uri).or_default();
        if let Some(change) = params.content_changes.last() {
            entry.text = change.text.clone();
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut documents = self.documents.write().await;
        documents.remove(&params.text_document.uri);
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let config = self.config.read().await.clone();
        if !config.enabled || !config.format_on_save {
            return Ok(Some(Vec::new()));
        }

        let documents = self.documents.read().await;
        let Some(doc) = documents.get(&params.text_document.uri) else {
            return Ok(Some(Vec::new()));
        };
        let Some(language_id) = doc.language_id.as_deref() else {
            return Ok(Some(Vec::new()));
        };
        if !config
            .target_languages
            .iter()
            .any(|lang| lang == language_id)
        {
            return Ok(Some(Vec::new()));
        }

        let options = FormatOptions {
            frontmatter_exclude_keys: &config.frontmatter_exclude_keys,
        };
        match format::convertible_ranges(language_id, &doc.text, &options) {
            Ok(ranges) => Ok(Some(format::text_edits(&doc.text, &ranges))),
            Err(message) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!(
                            "{}: skipped formatting, {message}",
                            params.text_document.uri
                        ),
                    )
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        config: RwLock::new(FormatterConfig::default()),
        documents: RwLock::new(HashMap::new()),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
