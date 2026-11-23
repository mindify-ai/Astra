//! The Astra protocol: the contract shared by the Rust core and the TypeScript client.
//!
//! See `docs/PROTOCOL.md`. JSON is camelCase, enum values are camelCase strings, tagged unions use `type`
//! (entities use `kind`, frames use `t`), and optional values serialize as `null`.

pub mod entities;
pub mod frames;
pub mod hlc;
pub mod ops;
pub mod reducer;
pub mod state;
pub mod ts;

pub use entities::*;
pub use frames::{ClientFrame, ServerFrame};
pub use hlc::{Hlc, HlcClock, HlcParseError};
pub use ops::{Command, Op, OpBody, new_op_id};
pub use reducer::{Clocks, Effect, EntityClock, Reducer, apply, apply_body};
pub use state::{Seed, Snapshot, State};

/// Current time as an RFC 3339 UTC string (second precision, `Z` suffix).
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
