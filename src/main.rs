use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use tree_sitter::{Language, Node, Parser, Point, Tree};

use tokio::sync::RwLock;

mod hl7_docs;
use hl7_docs::{lookup_doc, lookup_segment_doc};

unsafe extern "C" {
    fn tree_sitter_hl7v2() -> Language;
}

struct Backend {
    client: Client,
    parser: RwLock<Parser>,
    ast: RwLock<Option<Tree>>,
    message_text: RwLock<Option<String>>,
}

impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
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
        if let Some(parse_tree) = parse_tree.as_ref()
            && let Some(node) = parse_tree
                .root_node()
                .descendant_for_point_range(hover_point, hover_point)
        {
            // Don't show hover for structural separator characters
            if matches!(
                node.kind(),
                "field_separator"
                    | "component_separator"
                    | "repetition_separator"
                    | "subcomponent_separator"
                    | "segment_separator"
                    | "escape_character"
            ) {
                return Ok(None);
            }
            let rwl_message_text = self.message_text.read().await;
            if let Some(msg_text) = rwl_message_text.as_ref() {
                if let Some(node_info) = get_node_info(node, msg_text) {
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: node_info,
                        }),
                        range: None,
                    }));
                }
            }
        }
        Ok(None)
    }

    async fn inlay_hint(&self, _params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let message_text = self.message_text.read().await;
        if let Some(text) = message_text.as_ref() {
            return Ok(Some(get_inlay_hints(text)));
        }
        Ok(None)
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) -> () {
        self.parse(params.text_document.text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) -> () {
        // With FULL sync the client sends the entire document in each change event.
        if let Some(change) = params.content_changes.into_iter().last() {
            self.parse(change.text).await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

impl Backend {
    async fn parse(&self, text_doc: String) {
        let tree = {
            let mut parser = self.parser.write().await;
            parser.parse(&text_doc, None)
        };
        match tree {
            Some(tree) => {
                *self.ast.write().await = Some(tree);
                *self.message_text.write().await = Some(text_doc);
            }
            None => {
                self.client
                    .log_message(MessageType::WARNING, "Failed to parse document")
                    .await;
            }
        }
    }
}

fn get_node_info<'a>(node: Node<'a>, msg_text: &str) -> Option<String> {
    // Hovering over a segment name (e.g., "PID", "OBX")
    if node.kind() == "segment_name" {
        let seg = &msg_text[node.byte_range()];
        if let Some(doc) = lookup_segment_doc(seg) {
            return Some(doc);
        }
        return Some(seg.to_owned());
    }

    let segment = get_node_segment(node, msg_text)?;
    let node_numbers = get_node_numbers(node, &segment);
    // get_node_numbers already applies the MSH +1 offset so we use it directly
    // Strip optional repeat bracket e.g. "11[2]" → "11" before parsing
    let field_idx = node_numbers
        .split('.')
        .next()
        .map(|s| s.split('[').next().unwrap_or(s))
        .and_then(|s| s.parse::<u32>().ok());
    if let Some(idx) = field_idx
        && let Some(doc) = lookup_doc(&segment, idx)
    {
        return Some(format!("{segment} - {node_numbers}\n\n{doc}"));
    }
    Some(format!("{segment}.{node_numbers}"))
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let language = unsafe { tree_sitter_hl7v2() };
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Error loading hl7v2 grammar");

    let (service, socket) = LspService::new(|client| Backend {
        client,
        parser: RwLock::new(parser),
        ast: RwLock::new(None),
        message_text: RwLock::new(None),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}

// Inlay hints are computed from the raw message text rather than the tree-sitter AST.
// This avoids two problems with the AST-based approach:
//   1. ERROR nodes: when tree-sitter can't parse a field cleanly, it wraps surrounding
//      content in an ERROR node, hiding field_separator children from direct iteration.
//   2. Byte vs UTF-16 offsets: tree-sitter's Node::start_position().column is a byte
//      offset, but LSP requires UTF-16 code unit offsets; for non-ASCII characters
//      (e.g. curly quotes) these diverge.
// Since '|' is always the field separator in HL7 v2 (defined in MSH.1), we can count
// pipe characters directly from the text with correct UTF-16 positioning.
fn get_inlay_hints(message_text: &str) -> Vec<InlayHint> {
    let mut hints = Vec::new();
    let mut row = 0u32;
    let bytes = message_text.as_bytes();
    let len = bytes.len();
    let mut line_start = 0;

    while line_start < len {
        // Find end of line, handling \r, \n, and \r\n (all valid HL7 segment terminators)
        let mut line_end = line_start;
        while line_end < len && bytes[line_end] != b'\r' && bytes[line_end] != b'\n' {
            line_end += 1;
        }

        let line = &message_text[line_start..line_end];

        if line.contains('|') {
            // MSH.1 is the pipe itself, so the first pipe in MSH is field 2
            let is_msh = line.starts_with("MSH");
            let mut field_count: u32 = if is_msh { 1 } else { 0 };
            let mut utf16_col: u32 = 0;

            for (byte_offset, ch) in line.char_indices() {
                if ch == '|' {
                    field_count += 1;
                    // Add a colon suffix when a non-empty field follows
                    let after = &line[byte_offset + 1..];
                    let has_content = !after.is_empty() && !after.starts_with('|');
                    let label = if has_content {
                        format!("{field_count}:")
                    } else {
                        format!("{field_count}")
                    };
                    hints.push(InlayHint {
                        position: Position {
                            line: row,
                            character: utf16_col + 1, // position after the '|'
                        },
                        label: InlayHintLabel::String(label),
                        data: None,
                        kind: Some(InlayHintKind::TYPE),
                        padding_left: Some(false),
                        padding_right: Some(false),
                        text_edits: None,
                        tooltip: None,
                    });
                } else if ch == '~' && field_count > 0 && !(is_msh && field_count == 2) {
                    // Repetition separator: show the field number again so the reader
                    // knows this is another repetition of the same field
                    hints.push(InlayHint {
                        position: Position {
                            line: row,
                            character: utf16_col + 1, // position after the '~'
                        },
                        label: InlayHintLabel::String(format!("{field_count}:")),
                        data: None,
                        kind: Some(InlayHintKind::TYPE),
                        padding_left: Some(false),
                        padding_right: Some(false),
                        text_edits: None,
                        tooltip: None,
                    });
                }
                utf16_col += ch.len_utf16() as u32;
            }
        }

        // Advance past line ending (\r\n counts as one)
        if line_end < len {
            if bytes[line_end] == b'\r' && line_end + 1 < len && bytes[line_end + 1] == b'\n' {
                line_start = line_end + 2;
            } else {
                line_start = line_end + 1;
            }
        } else {
            break;
        }
        row += 1;
    }

    hints
}

fn get_node_segment(node: Node, msg_text: &str) -> Option<String> {
    let mut tmp_node = node;
    while let Some(parent) = tmp_node.parent() {
        tmp_node = parent;
        if parent.kind() == "segment" {
            break;
        }
    }

    let whole_segment = tmp_node.byte_range();
    let reduced_range = whole_segment.start..(whole_segment.start + 3);
    msg_text.get(reduced_range).map(|s| s.to_owned())
}

fn get_node_numbers(node: Node, segment: &str) -> String {
    let mut sub_component_idx = 0;
    let mut component_idx = 0;
    let mut repeat_idx = 0;
    let mut field_idx = 0;

    let mut tmp_node = node;
    while let Some(parent) = tmp_node.parent() {
        if parent.kind() == "subcomponent" {
            sub_component_idx = count_prev_subcomponents(parent);
        }
        if parent.kind() == "component" {
            component_idx = count_prev_components(parent);
        }
        if parent.kind() == "repeat" {
            repeat_idx = count_prev_repeats(parent);
        }
        if parent.kind() == "field" {
            field_idx = count_prev_fields(parent);
        }
        tmp_node = parent
    }
    if segment == "MSH" {
        field_idx += 1;
    }
    if repeat_idx > 1 {
        format!("{field_idx}[{repeat_idx}].{component_idx}.{sub_component_idx}")
    } else {
        format!("{field_idx}.{component_idx}.{sub_component_idx}")
    }
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

fn count_prev_repeats(node: Node) -> u32 {
    assert_eq!(node.kind(), "repeat");

    let mut tmp_node = node;
    let mut sibling_count = 1;
    while let Some(sibling) = tmp_node.prev_sibling() {
        tmp_node = sibling;
        if tmp_node.kind() == "repetition_separator" {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_parser() -> Parser {
        let language = unsafe { tree_sitter_hl7v2() };
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .expect("Error loading hl7v2 grammar");
        parser
    }

    fn parse_and_check(msg: &str) -> bool {
        let mut parser = make_parser();
        let tree = parser.parse(msg, None).unwrap();
        let root = tree.root_node();
        if root.has_error() {
            eprintln!("Parse tree has errors:\n{}", root.to_sexp());
        }
        !root.has_error()
    }

    #[test]
    fn test_parse_without_errors() {
        let msg = "MSH|^~\\&|MegaReg|XYZHospC|SuperOE|XYZImgCtr|20060529090131-0500||ADT^A01^ADT_A01|01052901|P|2.5\rEVN||200605290901||||\rPID|||56782445^^^UAReg^PI||KLEINSAMPLE^BARRY^Q^JR||19620910|M||2028-9^^HL70005^RA99113^^XYZ|260 GOODWIN CREST DRIVE^^BIRMINGHAM^AL^35209^^M~NICKELL'S PICKLES^10000 W 100TH AVE^BIRMINGHAM^AL^35200^^O|||||||0105I30001^^^99DEF^AN\rPV1||I|W^389^1^UABH^^^^3||||12345^MORGAN^REX^J^^^MD^0010^UAMC^L||67890^GRAINGER^LUCY^X^^^MD^0010^UAMC^L|MED|||||A0||13579^POTTER^SHERMAN^T^^^MD^0010^UAMC^L|||||||||||||||||||||||||||200605290900\rOBX|1|NM|^Body Height||1.80|m^Meter^ISO+|||||F\rOBX|2|NM|^Body Weight||79|kg^Kilogram^ISO+|||||F\rAL1|1||^ASPIRIN\rDG1|1||786.50^CHEST PAIN, UNSPECIFIED^I9|||A\r";
        assert!(
            parse_and_check(msg),
            "Full sample HL7 message should parse without errors"
        );
    }

    #[test]
    fn test_repeat_index_in_node_numbers() {
        // PID.11 has two repetitions separated by ~:
        //   rep 1: "260 GOODWIN CREST DRIVE^^BIRMINGHAM^AL^35209^^M"
        //   rep 2: "NICKELL'S PICKLES^10000 W 100TH AVE^BIRMINGHAM^AL^35200^^O"
        // BIRMINGHAM in rep 1 is component 3 → "11.3.1"
        // NICKELL'S PICKLES in rep 2 is component 1 → "11[2].1.1"
        let msg = "MSH|^~\\&|A|B|C|D|20060101||\rPID|||||||||||260 GOODWIN CREST DRIVE^^BIRMINGHAM^AL^35209^^M~NICKELL'S PICKLES^10000 W 100TH AVE^BIRMINGHAM^AL^35200^^O\r";
        let mut parser = make_parser();
        let tree = parser.parse(msg, None).unwrap();
        assert!(!tree.root_node().has_error(), "Should parse without errors");

        // Find the string node for BIRMINGHAM (rep 1, component 3)
        let birmingham_node = tree
            .root_node()
            .descendant_for_byte_range(
                msg.find("BIRMINGHAM").unwrap(),
                msg.find("BIRMINGHAM").unwrap() + 1,
            )
            .unwrap();
        let numbers = get_node_numbers(birmingham_node, "PID");
        assert_eq!(
            numbers, "11.3.1",
            "BIRMINGHAM should be PID.11, component 3 (no repeat bracket for rep 1)"
        );

        // Find the string node for NICKELL (rep 2, component 1)
        let nickell_offset = msg.find("NICKELL").unwrap();
        let nickell_node = tree
            .root_node()
            .descendant_for_byte_range(nickell_offset, nickell_offset + 1)
            .unwrap();
        let numbers = get_node_numbers(nickell_node, "PID");
        assert_eq!(
            numbers, "11[2].1.1",
            "NICKELL'S PICKLES should be PID.11 repeat 2, component 1"
        );
    }
}
