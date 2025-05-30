use std::fs::File;
use std::io::Write;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::lsp_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use tree_sitter::{Language, Parser, Tree};

use tokio::sync::RwLock;

unsafe extern "C" {
    fn tree_sitter_hl7v2() -> Language;
}

struct Backend {
    client: Client,
    log_path: String,
    language: Language,
    ast: RwLock<Option<Tree>>,
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

        self.client
            .show_message(MessageType::INFO, "successfully parsed message2")
            .await;

        let tree = parser.parse(&text_doc, None).unwrap();
        // This needs to its own scope so that the RWLock is dropped and ast can be read again
        // afterwards.
        {
            let mut ast = self.ast.write().await;
            *ast = Some(tree);
        }

        let ast = self.ast.read().await;
        let ast_string = ast.as_ref().unwrap().root_node().to_sexp();

        let mut f = File::options().append(true).open(&self.log_path).unwrap();
        writeln!(&mut f, "{}", ast_string).unwrap();
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
        ast: RwLock::new(None),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
