use std::fs::File;
use std::io::Write;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::lsp_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use tree_sitter::{Language, Parser};

unsafe extern "C" {
    fn tree_sitter_hl7v2() -> Language;
}

struct Backend {
    client: Client,
    log_path: String,
    language: Language,
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        let mut f = File::options().append(true).open(&self.log_path).unwrap();
        writeln!(&mut f, "Inititialize called").unwrap();

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let mut f = File::options().append(true).open(&self.log_path).unwrap();
        writeln!(&mut f, "Initialized called").unwrap();

        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn hover(&self, _: HoverParams) -> Result<Option<Hover>> {
        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String("You're hovering!".to_string())),
            range: None,
        }))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) -> () {
        let text_doc = params.text_document.text;
        let length = text_doc.len();
        let message = format!("Doc length = {length}");

        self.client.show_message(MessageType::INFO, message).await;

        let mut parser = Parser::new();
        parser
            .set_language(&self.language)
            .expect("Error loading MyLang grammar");

        let _tree = parser.parse(&text_doc, None).unwrap();
        // let ast = tree.root_node().to_sexp();

        self.client
            .show_message(MessageType::INFO, "successfully parsed message")
            .await;

        // let mut f = File::options().append(true).open(&self.log_path).unwrap();
        // writeln!(&mut f, &ast).unwrap();
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let log_path = "/Users/jessekruse/Desktop/lsp_dbg_log.txt";
    let mut output = File::create(&log_path).unwrap();
    let line = "Created File";
    writeln!(output, "{}", line).unwrap();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let language = unsafe { tree_sitter_hl7v2() };

    let (service, socket) = LspService::new(|client| Backend {
        client,
        log_path: String::from(log_path),
        language,
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
