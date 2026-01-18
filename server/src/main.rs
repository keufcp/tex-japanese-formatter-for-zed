use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormatterConfig {
    enabled: bool,
    format_on_save: bool,
    target_languages: Vec<String>,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            format_on_save: true,
            target_languages: vec!["latex".to_string()],
        }
    }
}

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
    fn format_text(&self, text: &str) -> String {
        text.replace('、', "，").replace('。', "．")
    }

    fn document_end_position(text: &str) -> Position {
        let mut line_index: u32 = 0;
        let mut last_line = "";

        for line in text.split('\n') {
            last_line = line;
            line_index = line_index.saturating_add(1);
        }

        if line_index == 0 {
            return Position::new(0, 0);
        }

        let last_line_index = line_index - 1;
        let character = last_line.encode_utf16().count() as u32;
        Position::new(last_line_index, character)
    }

    fn update_config_from_value(config: &mut FormatterConfig, value: serde_json::Value) {
        if let Ok(parsed) = serde_json::from_value::<FormatterConfig>(value.clone()) {
            *config = parsed;
            return;
        }

        if let Some(settings_value) = value.get("settings") {
            if let Ok(parsed) = serde_json::from_value::<FormatterConfig>(settings_value.clone()) {
                *config = parsed;
            }
        }
    }

    async fn is_target_language(&self, uri: &Url) -> bool {
        let documents = self.documents.read().await;
        let Some(doc) = documents.get(uri) else {
            return false;
        };
        let Some(language_id) = doc.language_id.as_deref() else {
            return false;
        };

        let config = self.config.read().await;
        if !config.enabled {
            return false;
        }
        config
            .target_languages
            .iter()
            .any(|lang| lang == language_id)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(init_options) = params.initialization_options {
            let mut config = self.config.write().await;
            Self::update_config_from_value(&mut config, init_options);
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
        let mut config = self.config.write().await;
        Self::update_config_from_value(&mut config, params.settings);
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
        {
            let config = self.config.read().await;
            if !config.format_on_save {
                return Ok(Some(Vec::new()));
            }
        }

        if !self.is_target_language(&params.text_document.uri).await {
            return Ok(Some(Vec::new()));
        }

        let documents = self.documents.read().await;
        let Some(doc) = documents.get(&params.text_document.uri) else {
            return Ok(Some(Vec::new()));
        };

        let formatted = self.format_text(&doc.text);
        if formatted == doc.text {
            return Ok(Some(Vec::new()));
        }

        let end = Self::document_end_position(&doc.text);
        let edit = TextEdit {
            range: Range::new(Position::new(0, 0), end),
            new_text: formatted,
        };

        Ok(Some(vec![edit]))
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
