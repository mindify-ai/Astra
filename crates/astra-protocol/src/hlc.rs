//! Hybrid logical clocks.
//!
//! Wire format: `"<ms 16 digits>:<counter 6 digits>:<nodeId>"`, so lexicographic string order equals
//! causal order (ties broken by node id).

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A hybrid logical clock timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Hlc {
    /// Wall-clock milliseconds since the Unix epoch.
    pub ms: u64,
    /// Logical counter within the same millisecond.
    pub counter: u32,
    /// Node that produced the timestamp.
    pub node: String,
}

/// Errors parsing an HLC string.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HlcParseError {
    #[error("hlc must have the form <ms>:<counter>:<node>")]
    Shape,
    #[error("hlc milliseconds must be 16 digits")]
    Ms,
    #[error("hlc counter must be 6 digits")]
    Counter,
}

impl Hlc {
    /// The zero clock: older than any real timestamp. Initial entities carry this clock.
    pub const fn zero() -> Self {
        Hlc {
            ms: 0,
            counter: 0,
            node: String::new(),
        }
    }

    /// True for [`Hlc::zero`].
    pub fn is_zero(&self) -> bool {
        self.ms == 0 && self.counter == 0 && self.node.is_empty()
    }

    /// Construct a timestamp.
    pub fn new(ms: u64, counter: u32, node: impl Into<String>) -> Self {
        Hlc {
            ms,
            counter,
            node: node.into(),
        }
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.ms, self.counter, self.node.as_str()).cmp(&(
            other.ms,
            other.counter,
            other.node.as_str(),
        ))
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return f.write_str("0");
        }
        write!(f, "{:016}:{:06}:{}", self.ms, self.counter, self.node)
    }
}

impl FromStr for Hlc {
    type Err = HlcParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "0" {
            return Ok(Hlc::zero());
        }
        let mut parts = s.splitn(3, ':');
        let (Some(ms), Some(counter), Some(node)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(HlcParseError::Shape);
        };
        if ms.len() != 16 || !ms.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HlcParseError::Ms);
        }
        if counter.len() != 6 || !counter.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HlcParseError::Counter);
        }
        Ok(Hlc {
            ms: ms.parse().map_err(|_| HlcParseError::Ms)?,
            counter: counter.parse().map_err(|_| HlcParseError::Counter)?,
            node: node.to_string(),
        })
    }
}

impl Serialize for Hlc {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hlc {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A monotonic HLC generator for one node.
#[derive(Debug, Clone)]
pub struct HlcClock {
    node: String,
    last: Hlc,
}

const MAX_COUNTER: u32 = 999_999;

impl HlcClock {
    /// A clock for `node`, starting from zero.
    pub fn new(node: impl Into<String>) -> Self {
        let node = node.into();
        HlcClock {
            last: Hlc::new(0, 0, node.clone()),
            node,
        }
    }

    /// The node id this clock stamps.
    pub fn node(&self) -> &str {
        &self.node
    }

    /// The most recent timestamp produced or observed.
    pub fn last(&self) -> &Hlc {
        &self.last
    }

    fn wall_ms() -> u64 {
        u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
    }

    /// Produce a new timestamp strictly greater than every timestamp produced or observed so far.
    pub fn tick(&mut self) -> Hlc {
        self.tick_at(Self::wall_ms())
    }

    /// [`HlcClock::tick`] with an explicit wall-clock reading (for tests).
    pub fn tick_at(&mut self, wall_ms: u64) -> Hlc {
        let (ms, counter) = if wall_ms > self.last.ms {
            (wall_ms, 0)
        } else if self.last.counter >= MAX_COUNTER {
            (self.last.ms + 1, 0)
        } else {
            (self.last.ms, self.last.counter + 1)
        };
        self.last = Hlc::new(ms, counter, self.node.clone());
        self.last.clone()
    }

    /// Merge a remote timestamp so later ticks sort after it.
    pub fn observe(&mut self, remote: &Hlc) {
        self.observe_at(remote, Self::wall_ms());
    }

    /// [`HlcClock::observe`] with an explicit wall-clock reading (for tests).
    pub fn observe_at(&mut self, remote: &Hlc, wall_ms: u64) {
        let ms = wall_ms.max(self.last.ms).max(remote.ms);
        let counter = if ms == self.last.ms && ms == remote.ms {
            self.last.counter.max(remote.counter)
        } else if ms == self.last.ms {
            self.last.counter
        } else if ms == remote.ms {
            remote.counter
        } else {
            0
        };
        let candidate = Hlc::new(ms, counter, self.node.clone());
        if candidate > self.last {
            self.last = candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_format_roundtrip() {
        let s = "0001732180000500:000001:n2";
        let h: Hlc = s.parse().expect("parse");
        assert_eq!(h, Hlc::new(1_732_180_000_500, 1, "n2"));
        assert_eq!(h.to_string(), s);
        assert_eq!("0".parse::<Hlc>().expect("zero"), Hlc::zero());
        assert!("12:000001:n".parse::<Hlc>().is_err());
        assert!("0001732180000500:1:n".parse::<Hlc>().is_err());
        assert!("garbage".parse::<Hlc>().is_err());
    }

    #[test]
    fn order_matches_string_order() {
        let a: Hlc = "0001732180000500:000001:n1".parse().expect("a");
        let b: Hlc = "0001732180000500:000001:n2".parse().expect("b");
        let c: Hlc = "0001732180000500:000002:n0".parse().expect("c");
        assert!(a < b && b < c);
        assert!(Hlc::zero() < a);
        assert!(a.to_string() < b.to_string() && b.to_string() < c.to_string());
    }

    #[test]
    fn clock_is_monotonic() {
        let mut clock = HlcClock::new("n1");
        let t1 = clock.tick_at(100);
        let t2 = clock.tick_at(100);
        let t3 = clock.tick_at(50);
        assert!(t1 < t2 && t2 < t3);
        clock.observe_at(&Hlc::new(500, 7, "n9"), 100);
        let t4 = clock.tick_at(100);
        assert!(t4 > Hlc::new(500, 7, "n9"));
        assert_eq!(t4.ms, 500);
        let real = clock.tick();
        assert!(real > t4);
    }
}
