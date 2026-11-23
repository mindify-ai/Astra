//! `astrad`: the headless Astra runner node ("Your server") and LAN host.
//!
//! LAN sync is the normal sync protocol (MessagePack frames) inside a Noise_XX-encrypted WebSocket at
//! `/v1/lan`. Pairing: `astrad pair` prints a one-time code and a QR for
//! `astra://pair?host=..&port=..&code=..&fp=..`; the phone connects, checks the node's key fingerprint `fp`,
//! and sends `hello.token = "pair:<code>"`. The node then remembers the phone's Noise static key, which
//! authenticates every later connection (no bearer token travels on the LAN).

pub mod lan;
pub mod pairing;
