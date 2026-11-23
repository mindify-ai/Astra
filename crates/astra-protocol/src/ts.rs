//! TypeScript definitions of every public protocol type.
//!
//! These are maintained next to the serde types (tagged unions with `kind`/`type`/`t` tags and explicit
//! `null`s are expressed exactly as serde emits them) and exported to
//! `packages/client/src/generated/protocol.ts` by `cargo run -p astra-protocol --bin export-ts`.
//! The test suite cross-checks the interfaces against serde output.

/// The TypeScript source.
pub const PROTOCOL_TS: &str = include_str!("protocol.ts");

/// Path of the generated file relative to the repository root.
pub const OUTPUT_PATH: &str = "packages/client/src/generated/protocol.ts";

/// Field names declared for a TS `interface` (top-level members only).
pub fn interface_fields(name: &str) -> Option<Vec<String>> {
    let start = PROTOCOL_TS.find(&format!("export interface {name} {{"))?;
    let body = &PROTOCOL_TS[start..];
    let open = body.find('{')?;
    let close = body.find("\n}")?;
    let mut fields = Vec::new();
    for line in body[open + 1..close].lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("/*")
            || line.starts_with("//")
            || line.starts_with('*')
        {
            continue;
        }
        if let Some((field, _)) = line.split_once(':') {
            fields.push(field.trim().to_string());
        }
    }
    Some(fields)
}
