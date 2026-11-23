//! Writes `packages/client/src/generated/protocol.ts`.
//!
//! Usage: `cargo run -p astra-protocol --bin export-ts [-- <output path>]`

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(astra_protocol::ts::OUTPUT_PATH)
        });
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, astra_protocol::ts::PROTOCOL_TS)?;
    println!("wrote {}", out.display());
    Ok(())
}
