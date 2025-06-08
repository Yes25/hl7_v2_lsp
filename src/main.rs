use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::lsp_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use tree_sitter::{Language, Node, Parser, Point, Tree};

use tokio::sync::RwLock;

unsafe extern "C" {
    fn tree_sitter_hl7v2() -> Language;
}

struct Backend {
    client: Client,
    language: Language,
    ast: RwLock<Option<Tree>>,
    message_text: RwLock<Option<String>>,
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
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
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params.position;

        let hover_point = Point {
            row: position.line as usize,
            column: position.character as usize,
        };

        let parse_tree = self.ast.read().await;
        if let Some(parse_tree) = parse_tree.as_ref() {
            if let Some(node) = parse_tree
                .root_node()
                .descendant_for_point_range(hover_point, hover_point)
            {
                let rwl_message_text = self.message_text.read().await;
                if let Some(msg_text) = rwl_message_text.as_ref() {
                    let node_info = get_node_info(node, msg_text);
                    return Ok(Some(Hover {
                        contents: HoverContents::Scalar(MarkedString::String(node_info)),
                        range: None,
                    }));
                }
            }
        }
        Ok(None)
    }

    async fn inlay_hint(&self, _params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
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
        {
            let mut msg = self.message_text.write().await;
            *msg = Some(text_doc);
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let language = unsafe { tree_sitter_hl7v2() };

    let (service, socket) = LspService::new(|client| Backend {
        client,
        language,
        ast: RwLock::new(None),
        message_text: RwLock::new(None),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
// TODO: There is some kind of bug with chars like ` and ´ (zero width?)
fn get_inlay_hints(parse_tree: &Tree) -> Vec<InlayHint> {
    let mut inlay_hints: Vec<InlayHint> = Vec::new();
    let root_node = parse_tree.root_node();

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

fn get_node_info(node: Node, msg_text: &String) -> String {
    let segment = get_node_segment(node, msg_text);
    let node_numbers = get_node_numbers(node, &segment);

    format!("{segment} - {node_numbers}")
}

fn get_node_segment(node: Node, msg_text: &String) -> String {
    let mut tmp_node = node;
    while let Some(parent) = tmp_node.parent() {
        tmp_node = parent;
        if parent.kind() == "segment" {
            break;
        }
    }

    let whole_segment = tmp_node.byte_range();
    let reduced_range = whole_segment.start..(whole_segment.start + 3);
    let segment = msg_text.get(reduced_range).unwrap();
    segment.to_owned()
}

fn get_node_numbers(node: Node, segment: &str) -> String {
    let mut sub_component_idx = 0;
    let mut component_idx = 0;
    let mut field_idx = 0;

    let mut tmp_node = node;
    while let Some(parent) = tmp_node.parent() {
        if parent.kind() == "subcomponent" {
            sub_component_idx = count_prev_subcomponents(parent);
        }
        if parent.kind() == "component" {
            component_idx = count_prev_components(parent);
        }
        if parent.kind() == "field" {
            field_idx = count_prev_fields(parent);
        }
        tmp_node = parent
    }
    if segment == "MSH" {
        field_idx += 1;
    }
    format!("{field_idx}.{component_idx}.{sub_component_idx}")
}

fn count_prev_fields(node: Node) -> u32 {
    assert_eq!(node.kind(), "field");

    let mut tmp_node = node;
    let mut sibling_count = 0;
    while let Some(sibling) = tmp_node.prev_sibling() {
        tmp_node = sibling;
        if tmp_node.kind() == "field_separator" {
            sibling_count += 1
        }
    }
    sibling_count
}

fn count_prev_components(node: Node) -> u32 {
    assert_eq!(node.kind(), "component");

    let mut tmp_node = node;
    let mut sibling_count = 1;
    while let Some(sibling) = tmp_node.prev_sibling() {
        tmp_node = sibling;
        if tmp_node.kind() == "component_separator" {
            sibling_count += 1
        }
    }
    sibling_count
}

fn count_prev_subcomponents(node: Node) -> u32 {
    assert_eq!(node.kind(), "subcomponent");

    let mut tmp_node = node;
    let mut sibling_count = 1;
    while let Some(sibling) = tmp_node.prev_sibling() {
        tmp_node = sibling;
        if tmp_node.kind() == "subcomponent_separator" {
            sibling_count += 1
        }
    }
    sibling_count
}
