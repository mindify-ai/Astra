//! Transport-agnostic Astra sync.
//!
//! - [`codec`]: MessagePack (default) / JSON frame encoding.
//! - [`client`]: the client half: a sans-IO [`client::ClientCore`] state machine and the async
//!   [`client::SyncClient`] driver (hello with cursor → replay → push pending → acks; reconnect with backoff)
//!   over any `Sink`/`Stream` of frames.
//! - [`hub`]: the server half: per-workspace sequence numbers, dedup by op id, replay after cursor, fan-out
//!   (which routes `command` ops to their node), presence relay; storage behind the [`hub::OpLog`] trait with an
//!   in-memory implementation.

pub mod client;
pub mod codec;
pub mod hub;

pub use client::{
    ClientCore, Connector, StoreHost, SyncClient, SyncConfig, SyncHandle, SyncHost, Transport,
};
pub use codec::Format;
pub use hub::{AllowAll, Authorizer, ConnId, Hub, HubConnector, MemoryOpLog, OpLog, Principal};

/// Sync errors.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("codec: {0}")]
    Codec(String),
    #[error("server: {0}")]
    Server(String),
    /// Authentication failed. Displays exactly `unauthorized` (clients treat it as "sign in again");
    /// the reason is for server logs only.
    #[error("unauthorized")]
    Unauthorized(String),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("closed")]
    Closed,
}

impl From<astra_store::StoreError> for SyncError {
    fn from(e: astra_store::StoreError) -> Self {
        SyncError::Storage(e.to_string())
    }
}

/// Result alias.
pub type Result<T> = std::result::Result<T, SyncError>;
