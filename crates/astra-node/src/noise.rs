//! Noise-encrypted WebSocket transport for LAN sync (phone ⇄ desktop/astrad).
//!
//! Handshake: `Noise_XX_25519_ChaChaPoly_BLAKE2s`, one WebSocket binary message per handshake message
//! (initiator → `e`; responder → `e, ee, s, es`; initiator → `s, se`). The initiator checks the responder's
//! static key against the fingerprint from the pairing QR (`fp` = hex SHA-256 of the static public key).
//! The responder learns the initiator's static key, which identifies a paired device.
//!
//! Transport: each sync frame (MessagePack) is split into chunks of at most [`MAX_CHUNK`] bytes; each chunk is
//! `[more: u8][data]`, encrypted as one Noise message and sent as one binary WebSocket message.

use std::path::Path;
use std::sync::{Arc, Mutex};

use astra_protocol::{ClientFrame, ServerFrame};
use astra_sync::codec::{self, Format};
use astra_sync::{Connector, SyncError, Transport};
use futures::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

/// Noise protocol name.
pub const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const NOISE_MAX: usize = 65_535;
const TAG: usize = 16;
/// Largest plaintext chunk per Noise message (excluding the flag byte).
pub const MAX_CHUNK: usize = NOISE_MAX - TAG - 1;

/// Noise errors.
#[derive(Debug, thiserror::Error)]
pub enum NoiseError {
    #[error("noise: {0}")]
    Snow(#[from] snow::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// A static X25519 key pair.
#[derive(Clone)]
pub struct StaticKey {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

impl std::fmt::Debug for StaticKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticKey")
            .field("fingerprint", &fingerprint(&self.public))
            .finish()
    }
}

fn params() -> Result<snow::params::NoiseParams, NoiseError> {
    PATTERN
        .parse()
        .map_err(|e: snow::Error| NoiseError::Snow(e))
}

/// Generate a key pair.
pub fn generate_key() -> Result<StaticKey, NoiseError> {
    let kp = snow::Builder::new(params()?).generate_keypair()?;
    Ok(StaticKey {
        private: kp.private.to_vec(),
        public: kp.public.to_vec(),
    })
}

/// Load a key pair from `path` (hex `private\npublic`), creating it if missing.
pub fn load_or_create_key(path: &Path) -> Result<StaticKey, NoiseError> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let mut lines = text.lines();
        if let (Some(a), Some(b)) = (lines.next(), lines.next())
            && let (Ok(private), Ok(public)) = (hex::decode(a.trim()), hex::decode(b.trim()))
        {
            return Ok(StaticKey { private, public });
        }
    }
    let key = generate_key()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        path,
        format!(
            "{}\n{}\n",
            hex::encode(&key.private),
            hex::encode(&key.public)
        ),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

/// Hex SHA-256 of a public key.
pub fn fingerprint(public: &[u8]) -> String {
    hex::encode(Sha256::digest(public))
}

/// Start a handshake as initiator.
pub fn initiator(key: &StaticKey) -> Result<snow::HandshakeState, NoiseError> {
    Ok(snow::Builder::new(params()?)
        .local_private_key(&key.private)?
        .build_initiator()?)
}

/// Start a handshake as responder.
pub fn responder(key: &StaticKey) -> Result<snow::HandshakeState, NoiseError> {
    Ok(snow::Builder::new(params()?)
        .local_private_key(&key.private)?
        .build_responder()?)
}

/// Write the next handshake message.
pub fn hs_write(hs: &mut snow::HandshakeState) -> Result<Vec<u8>, NoiseError> {
    let mut buf = vec![0u8; NOISE_MAX];
    let n = hs.write_message(&[], &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Read a handshake message.
pub fn hs_read(hs: &mut snow::HandshakeState, msg: &[u8]) -> Result<(), NoiseError> {
    let mut buf = vec![0u8; NOISE_MAX];
    hs.read_message(msg, &mut buf)?;
    Ok(())
}

/// Transport-mode cipher with frame chunking.
pub struct Cipher {
    state: snow::TransportState,
    pending: Vec<u8>,
}

impl Cipher {
    /// Wrap a finished handshake.
    pub fn new(hs: snow::HandshakeState) -> Result<Cipher, NoiseError> {
        Ok(Cipher {
            state: hs.into_transport_mode()?,
            pending: Vec::new(),
        })
    }

    /// Encrypt one frame into one or more messages.
    pub fn seal(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>, NoiseError> {
        let mut out = Vec::new();
        let chunks: Vec<&[u8]> = if frame.is_empty() {
            vec![&[][..]]
        } else {
            frame.chunks(MAX_CHUNK).collect()
        };
        let last = chunks.len() - 1;
        for (i, chunk) in chunks.into_iter().enumerate() {
            let mut plain = Vec::with_capacity(chunk.len() + 1);
            plain.push(u8::from(i != last));
            plain.extend_from_slice(chunk);
            let mut buf = vec![0u8; plain.len() + TAG];
            let n = self.state.write_message(&plain, &mut buf)?;
            buf.truncate(n);
            out.push(buf);
        }
        Ok(out)
    }

    /// Decrypt one message; returns a complete frame when its last chunk arrives.
    pub fn open(&mut self, msg: &[u8]) -> Result<Option<Vec<u8>>, NoiseError> {
        let mut buf = vec![0u8; msg.len()];
        let n = self.state.read_message(msg, &mut buf)?;
        let Some((&more, data)) = buf[..n].split_first() else {
            return Err(NoiseError::Other("empty chunk".into()));
        };
        self.pending.extend_from_slice(data);
        if self.pending.len() > 64 * 1024 * 1024 {
            return Err(NoiseError::Other("frame too large".into()));
        }
        Ok(if more == 0 {
            Some(std::mem::take(&mut self.pending))
        } else {
            None
        })
    }
}

/// Connects to an `astrad --serve-lan` node at `ws://host:port/v1/lan`, pinning its key fingerprint.
#[derive(Debug, Clone)]
pub struct NoiseWsConnector {
    pub url: String,
    pub fingerprint: String,
    pub key: StaticKey,
}

fn terr(e: impl std::fmt::Display) -> SyncError {
    SyncError::Transport(e.to_string())
}

#[async_trait::async_trait]
impl Connector for NoiseWsConnector {
    async fn connect(&self) -> astra_sync::Result<Transport> {
        let (mut ws, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .map_err(terr)?;
        let mut hs = initiator(&self.key).map_err(terr)?;
        ws.send(Message::Binary(hs_write(&mut hs).map_err(terr)?.into()))
            .await
            .map_err(terr)?;
        let msg2 = loop {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => break b,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                other => {
                    return Err(SyncError::Transport(format!(
                        "handshake: unexpected {other:?}"
                    )));
                }
            }
        };
        hs_read(&mut hs, &msg2).map_err(terr)?;
        let remote = hs.get_remote_static().map(fingerprint).unwrap_or_default();
        if !remote.eq_ignore_ascii_case(&self.fingerprint) {
            return Err(SyncError::Unauthorized(
                "server key does not match the paired fingerprint".into(),
            ));
        }
        ws.send(Message::Binary(hs_write(&mut hs).map_err(terr)?.into()))
            .await
            .map_err(terr)?;
        let cipher = Arc::new(Mutex::new(Cipher::new(hs).map_err(terr)?));

        let (sink, stream) = ws.split();
        let seal = cipher.clone();
        let sink = sink
            .sink_map_err(terr)
            .with_flat_map(move |frame: ClientFrame| {
                let sealed: Vec<astra_sync::Result<Message>> =
                    match codec::encode(Format::MsgPack, &frame).and_then(|b| {
                        seal.lock()
                            .map_err(|_| SyncError::Closed)?
                            .seal(&b)
                            .map_err(terr)
                    }) {
                        Ok(msgs) => msgs
                            .into_iter()
                            .map(|m| Ok(Message::Binary(m.into())))
                            .collect(),
                        Err(e) => vec![Err(e)],
                    };
                futures::stream::iter(sealed)
            });
        let stream = stream.filter_map(move |msg| {
            let cipher = cipher.clone();
            async move {
                match msg {
                    Ok(Message::Binary(b)) => {
                        let opened = cipher
                            .lock()
                            .map_err(|_| SyncError::Closed)
                            .and_then(|mut c| c.open(&b).map_err(terr));
                        match opened {
                            Ok(Some(frame)) => {
                                Some(codec::decode::<ServerFrame>(Format::MsgPack, &frame))
                            }
                            Ok(None) => None,
                            Err(e) => Some(Err(e)),
                        }
                    }
                    Ok(Message::Close(_)) => Some(Err(SyncError::Closed)),
                    Ok(_) => None,
                    Err(e) => Some(Err(terr(e))),
                }
            }
        });
        Ok(Transport {
            sink: Box::pin(sink),
            stream: Box::pin(stream),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_and_chunked_frames() {
        let (ck, sk) = (generate_key().expect("k"), generate_key().expect("k"));
        let mut i = initiator(&ck).expect("i");
        let mut r = responder(&sk).expect("r");
        hs_read(&mut r, &hs_write(&mut i).expect("m1")).expect("r1");
        hs_read(&mut i, &hs_write(&mut r).expect("m2")).expect("r2");
        assert_eq!(
            i.get_remote_static().map(fingerprint),
            Some(fingerprint(&sk.public))
        );
        hs_read(&mut r, &hs_write(&mut i).expect("m3")).expect("r3");
        assert_eq!(
            r.get_remote_static().map(<[u8]>::to_vec),
            Some(ck.public.clone())
        );
        let (mut a, mut b) = (Cipher::new(i).expect("a"), Cipher::new(r).expect("b"));
        let big: Vec<u8> = (0..200_000u32).map(|x| (x % 251) as u8).collect();
        let msgs = a.seal(&big).expect("seal");
        assert_eq!(msgs.len(), 4);
        let mut got = None;
        for m in &msgs {
            got = b.open(m).expect("open");
        }
        assert_eq!(got, Some(big));
        let small = b.seal(b"hi").expect("seal");
        assert_eq!(a.open(&small[0]).expect("open"), Some(b"hi".to_vec()));
    }

    #[test]
    fn key_persists() {
        let dir = std::env::temp_dir().join(format!("astra-noise-{}", std::process::id()));
        let p = dir.join("key");
        let k1 = load_or_create_key(&p).expect("create");
        let k2 = load_or_create_key(&p).expect("load");
        assert_eq!(k1.public, k2.public);
        let _ = std::fs::remove_dir_all(dir);
    }
}
