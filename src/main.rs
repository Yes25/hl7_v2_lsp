use std::fs::File;
use std::io::Write;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::lsp_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use tree_sitter::{Language, Node, Parser, Tree};

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
                inlay_hint_provider: Some(OneOf::Left(true)),
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

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let mut f = File::options().append(true).open(&self.log_path).unwrap();
        writeln!(&mut f, "inlay_hint got called",).unwrap();

        let parse_tree = self.ast.read().await;
        if let Some(parse_tree) = parse_tree.as_ref() {
            let inlay_hints = get_inlay_hints(parse_tree);
            return Ok(Some(inlay_hints));
        }

        Ok(None)
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
            .show_message(MessageType::INFO, "successfully parsed message")
            .await;

        let tree = parser.parse(&text_doc, None).unwrap();
        // This needs to its own scope so that the RWLock is dropped and ast can be read again
        // afterwards.
        {
            let mut ast = self.ast.write().await;
            *ast = Some(tree);
        }

        // let ast = self.ast.read().await;
        // let ast_string = ast.as_ref().unwrap().root_node().to_sexp();

        // let mut f = File::options().append(true).open(&self.log_path).unwrap();
        // writeln!(&mut f, "{}", ast_string).unwrap();
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let log_path = "lsp_dbg_log.txt";
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
// TODO: There is some kind of bug with chars like ` and ´ (zero width?)
fn get_inlay_hints(parse_tree: &Tree) -> Vec<InlayHint> {
    let mut inlay_hints: Vec<InlayHint> = Vec::new();
    let root_node = parse_tree.root_node();

    // TODO: Just for Dbg
    let log_path = "lsp_dbg_log.txt";
    let mut f = File::options().append(true).open(&log_path).unwrap();

    for i in 0..root_node.child_count() {
        if let Some(segment) = root_node.child(i) {
            let mut field_count = 0;
            for j in 1..segment.child_count() {
                if segment.kind() == "msh" && field_count == 0 {
                    field_count += 1; // one additional for msh, because first field is actually the field spearator ("|")
                }
                if let Some(field) = segment.child(j) {
                    if field.kind() == "field_separator" {
                        field_count += 1;

                        let mut hint_label = format!("{field_count}");
                        if let Some(next_field) = segment.child(j + 1) {
                            if next_field.kind() == "field" {
                                hint_label = format!("{field_count}:");
                            }
                        }
                        inlay_hints.push(build_inlay_hint(hint_label, &field))
                    }

                    // if field.kind() == "field" {
                    //     if let Some(repeat) = field.child(0) {
                    //         let mut component_count = 1;
                    //         for k in 0..repeat.child_count() {
                    //             if let Some(component) = repeat.child(k) {
                    //                 component_count += 1;
                    //                 // if component.kind() == "component_separator" {
                    //                 //     component_count += 1;
                    //                 //     let hint_label =
                    //                 //         format!("{field_count}.{component_count}:");
                    //                 //     inlay_hints.push(build_inlay_hint(hint_label, &component))
                    //                 // }

                    //                 if component.kind() == "component" {
                    //                     writeln!(&mut f, "in component path",).unwrap();

                    //                     let mut sub_component_count = 1;
                    //                     for l in 0..component.child_count() {
                    //                         if let Some(sub_component) = component.child(l) {
                    //                             sub_component_count += 1;
                    //                             let hint_label = format!(
                    //                                 "{field_count}.{component_count}.{sub_component_count}:"
                    //                             );
                    //                             inlay_hints.push(build_inlay_hint(
                    //                                 hint_label,
                    //                                 &sub_component,
                    //                             ))
                    //                         }
                    //                     }
                    //                 }
                    //             }
                    //         }
                    //     }
                    // }
                }
            }
        }
    }

    inlay_hints
}

fn build_inlay_hint(hint_label: String, child: &Node) -> InlayHint {
    let position = child.start_position();
    let row = position.row;
    let start = position.column;
    let mut start_idx = start as u32;

    if child.kind() == "field_separator" {
        start_idx += 1
    }

    InlayHint {
        position: Position {
            line: row as u32,
            character: start_idx,
        },
        label: InlayHintLabel::String(hint_label),
        data: None,
        kind: Some(InlayHintKind::TYPE),
        padding_left: Some(false),
        padding_right: Some(false),
        text_edits: None,
        tooltip: None,
    }
}
