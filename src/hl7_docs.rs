use std::collections::HashMap;
use once_cell::sync::Lazy;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Documentation {
    hl7_v2_fields: HashMap<String, SegmentDoc>,
}

#[derive(Debug, Deserialize)]
struct SegmentDoc {
    description: String,
    fields: HashMap<String, FieldDoc>,
}

#[derive(Debug, Deserialize)]
struct FieldDoc {
    name: String,
    description: String,
}

static DOCS: Lazy<Option<Documentation>> = Lazy::new(|| {
    serde_json::from_str(include_str!("documentation.json")).ok()
});

/// Looks up documentation for a given segment and field index (e.g., ("PID", 5)).
pub fn lookup_doc(segment: &str, field_idx: u32) -> Option<String> {
    let docs = DOCS.as_ref()?;
    let seg = docs.hl7_v2_fields.get(segment)?;
    let field_key = format!("{segment}.{}", field_idx);
    let field = seg.fields.get(&field_key)?;
    Some(format!("**{}**\n\n{}", field.name, field.description))
}

/// Looks up documentation for a segment (e.g., "PID").
pub fn lookup_segment_doc(segment: &str) -> Option<String> {
    let docs = DOCS.as_ref()?;
    let seg = docs.hl7_v2_fields.get(segment)?;
    Some(format!("**{}** — {}", segment, seg.description))
} 