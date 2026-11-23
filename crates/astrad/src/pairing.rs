//! Pairing state on disk (`<data-dir>/lan/`): node key, one-time codes, paired devices.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use astra_node::noise::{self, StaticKey};
use serde::{Deserialize, Serialize};

/// Files under `<data-dir>/lan`.
#[derive(Debug, Clone)]
pub struct LanFiles {
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Code {
    code: String,
    expires_at: i64,
}

/// A paired device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Hex Noise static public key.
    pub public_key: String,
    pub node_id: String,
    pub paired_at: String,
}

/// Where a running LAN server listens (written by `astrad --serve-lan`, read by `astrad pair`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerInfo {
    pub host: String,
    pub port: u16,
}

fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(tmp, path)
}

const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTVWXYZ";

impl LanFiles {
    /// Files for a node data dir.
    pub fn new(data_dir: &Path) -> Self {
        LanFiles {
            dir: data_dir.join("lan"),
        }
    }

    /// The node's Noise static key (created on first use).
    pub fn key(&self) -> Result<StaticKey, noise::NoiseError> {
        noise::load_or_create_key(&self.dir.join("identity.key"))
    }

    /// A new one-time pairing code valid for `ttl_secs`.
    pub fn create_code(&self, ttl_secs: i64) -> std::io::Result<String> {
        let rnd = ulid::Ulid::generate().random();
        let code: String = (0..8)
            .map(|i| ALPHABET[((rnd >> (i * 5)) % ALPHABET.len() as u128) as usize] as char)
            .collect();
        let now = chrono::Utc::now().timestamp();
        let path = self.dir.join("pairing.json");
        let mut codes: Vec<Code> = read_json(&path);
        codes.retain(|c| c.expires_at > now);
        codes.push(Code {
            code: code.clone(),
            expires_at: now + ttl_secs,
        });
        write_json(&path, &codes)?;
        Ok(code)
    }

    /// Consume a code; true when it was valid.
    pub fn consume_code(&self, code: &str) -> bool {
        let now = chrono::Utc::now().timestamp();
        let path = self.dir.join("pairing.json");
        let mut codes: Vec<Code> = read_json(&path);
        let before = codes.len();
        let valid = codes
            .iter()
            .any(|c| c.code.eq_ignore_ascii_case(code.trim()) && c.expires_at > now);
        codes.retain(|c| c.expires_at > now && !c.code.eq_ignore_ascii_case(code.trim()));
        if valid || codes.len() != before {
            let _ = write_json(&path, &codes);
        }
        valid
    }

    /// Paired devices.
    pub fn devices(&self) -> Vec<Device> {
        read_json(&self.dir.join("devices.json"))
    }

    /// Whether a static key belongs to a paired device.
    pub fn is_paired(&self, public_key: &[u8]) -> bool {
        let k = hex::encode(public_key);
        self.devices().iter().any(|d| d.public_key == k)
    }

    /// Remember a paired device.
    pub fn add_device(&self, public_key: &[u8], node_id: &str) -> std::io::Result<()> {
        let mut devices = self.devices();
        let k = hex::encode(public_key);
        if !devices.iter().any(|d| d.public_key == k) {
            devices.push(Device {
                public_key: k,
                node_id: node_id.into(),
                paired_at: astra_protocol::now_rfc3339(),
            });
            write_json(&self.dir.join("devices.json"), &devices)?;
        }
        Ok(())
    }

    /// Record where the LAN server listens.
    pub fn write_server_info(&self, info: &ServerInfo) -> std::io::Result<()> {
        write_json(&self.dir.join("server.json"), info)
    }

    /// Where the LAN server listens, if running.
    pub fn server_info(&self) -> Option<ServerInfo> {
        std::fs::read_to_string(self.dir.join("server.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
    }
}

/// `astra://pair?host=..&port=..&code=..&fp=..`
pub fn pair_uri(host: &str, port: u16, code: &str, fp: &str) -> String {
    format!("astra://pair?host={host}&port={port}&code={code}&fp={fp}")
}

/// A QR code of `data` rendered with Unicode half blocks.
pub fn qr_text(data: &str) -> Result<String, qrcode::types::QrError> {
    let code = qrcode::QrCode::new(data.as_bytes())?;
    Ok(code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

/// The primary LAN address of this machine (no packets are sent).
pub fn local_ip() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    sock.local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_unspecified())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_devices_and_qr() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = LanFiles::new(dir.path());
        let code = f.create_code(60).expect("code");
        assert_eq!(code.len(), 8);
        assert!(!f.consume_code("WRONG123"));
        assert!(f.consume_code(&code.to_lowercase()));
        assert!(!f.consume_code(&code), "one-time");
        let expired = f.create_code(-1).expect("code");
        assert!(!f.consume_code(&expired));
        f.add_device(&[1, 2, 3], "n-phone").expect("add");
        f.add_device(&[1, 2, 3], "n-phone").expect("add");
        assert_eq!(f.devices().len(), 1);
        assert!(f.is_paired(&[1, 2, 3]) && !f.is_paired(&[9]));
        let key = f.key().expect("key");
        assert_eq!(f.key().expect("key").public, key.public);
        let uri = pair_uri(
            "192.168.1.5",
            7420,
            &code,
            &astra_node::noise::fingerprint(&key.public),
        );
        assert!(uri.starts_with("astra://pair?host=192.168.1.5&port=7420&code="));
        let qr = qr_text(&uri).expect("qr");
        assert!(qr.contains('█') || qr.contains('▀') || qr.contains('▄'));
    }
}
