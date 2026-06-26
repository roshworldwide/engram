//! Identifiers.
//!
//! [`MemoryId`] is a 128-bit, time-sortable, ULID-style identifier: the high 48
//! bits are a millisecond timestamp and the low 80 bits are randomness, so ids
//! created later sort after ids created earlier. The remaining ids are simple
//! `u64` newtypes.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::IdParseError;
use crate::rng::Rng;
use crate::time::Clock;

const TS_BITS: u32 = 48;
const RAND_BITS: u32 = 80;
const TS_MASK: u64 = (1u64 << TS_BITS) - 1;
const RAND_MASK: u128 = (1u128 << RAND_BITS) - 1;

/// Crockford base32 alphabet (excludes I, L, O, U to avoid ambiguity).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// A canonical encoding is always 26 characters (130 bits, top 2 bits unused).
const ENCODED_LEN: usize = 26;

/// A 128-bit, time-sortable memory identifier (ULID layout).
///
/// ```
/// use engram_core::MemoryId;
/// let id = MemoryId::from_parts(1_700_000_000_000, 0xABCDEF);
/// // Round-trips through its 26-char Crockford base32 form.
/// assert_eq!(id, id.to_string().parse().unwrap());
/// assert!(id.timestamp_ms() == 1_700_000_000_000);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryId(pub u128);

impl MemoryId {
    /// Assemble an id from a millisecond timestamp (low 48 bits used) and
    /// randomness (low 80 bits used).
    #[must_use]
    pub const fn from_parts(timestamp_ms: u64, randomness: u128) -> Self {
        let ts = (timestamp_ms & TS_MASK) as u128;
        MemoryId((ts << RAND_BITS) | (randomness & RAND_MASK))
    }

    /// The embedded millisecond timestamp (high 48 bits).
    #[must_use]
    pub const fn timestamp_ms(self) -> u64 {
        ((self.0 >> RAND_BITS) as u64) & TS_MASK
    }

    /// The embedded randomness (low 80 bits).
    #[must_use]
    pub const fn randomness(self) -> u128 {
        self.0 & RAND_MASK
    }

    /// Generate a fresh id from a clock and rng. For a monotonic stream within a
    /// single millisecond, use [`MemoryIdGenerator`] instead.
    pub fn generate<C: Clock, R: Rng>(clock: &C, rng: &mut R) -> Self {
        let ms = clock.now().as_millis().max(0) as u64;
        MemoryId::from_parts(ms, rng.next_u128())
    }

    fn hi(self) -> u64 {
        (self.0 >> 64) as u64
    }

    fn lo(self) -> u64 {
        self.0 as u64
    }

    fn from_hi_lo(hi: u64, lo: u64) -> Self {
        MemoryId((u128::from(hi) << 64) | u128::from(lo))
    }

    fn to_crockford(self) -> [u8; ENCODED_LEN] {
        let mut v = self.0;
        let mut buf = [0u8; ENCODED_LEN];
        let mut i = ENCODED_LEN;
        while i > 0 {
            i -= 1;
            buf[i] = ALPHABET[(v & 0x1f) as usize];
            v >>= 5;
        }
        buf
    }
}

impl fmt::Display for MemoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let buf = self.to_crockford();
        // `buf` is always valid ASCII from `ALPHABET`; the fallback never fires.
        f.write_str(std::str::from_utf8(&buf).unwrap_or("<invalid-memory-id>"))
    }
}

impl fmt::Debug for MemoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MemoryId({self})")
    }
}

/// Map one Crockford character to its 5-bit value (lenient: accepts lowercase
/// and the ambiguous I/L → 1, O → 0).
fn decode_crockford(c: u8) -> Result<u8, IdParseError> {
    let v = match c {
        b'0'..=b'9' => c - b'0',
        b'A'..=b'H' => c - b'A' + 10,
        b'J' | b'K' => c - b'J' + 18,
        b'M' | b'N' => c - b'M' + 20,
        b'P'..=b'T' => c - b'P' + 22,
        b'V'..=b'Z' => c - b'V' + 27,
        b'a'..=b'h' => c - b'a' + 10,
        b'j' | b'k' => c - b'j' + 18,
        b'm' | b'n' => c - b'm' + 20,
        b'p'..=b't' => c - b'p' + 22,
        b'v'..=b'z' => c - b'v' + 27,
        b'I' | b'i' | b'L' | b'l' => 1,
        b'O' | b'o' => 0,
        _ => return Err(IdParseError::InvalidChar(c as char)),
    };
    Ok(v)
}

impl FromStr for MemoryId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != ENCODED_LEN {
            return Err(IdParseError::WrongLength(bytes.len()));
        }
        let mut v: u128 = 0;
        for (i, &c) in bytes.iter().enumerate() {
            let d = decode_crockford(c)?;
            // The leading character carries only 2 meaningful bits (130 − 128).
            if i == 0 && d > 7 {
                return Err(IdParseError::Overflow);
            }
            v = (v << 5) | u128::from(d);
        }
        Ok(MemoryId(v))
    }
}

impl Serialize for MemoryId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            // Crockford string for JSON / REST.
            serializer.collect_str(self)
        } else {
            // Compact (hi, lo) pair for MessagePack / on-disk.
            (self.hi(), self.lo()).serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for MemoryId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            s.parse().map_err(D::Error::custom)
        } else {
            let (hi, lo) = <(u64, u64)>::deserialize(deserializer)?;
            Ok(MemoryId::from_hi_lo(hi, lo))
        }
    }
}

/// A monotonic [`MemoryId`] generator: within a single millisecond it increments
/// the randomness field instead of redrawing, guaranteeing strictly increasing
/// ids even when many are created in the same instant or the clock stalls.
#[derive(Debug)]
pub struct MemoryIdGenerator<C: Clock, R: Rng> {
    clock: C,
    rng: R,
    last_ms: u64,
    last_rand: u128,
}

impl<C: Clock, R: Rng> MemoryIdGenerator<C, R> {
    /// Create a generator from a clock and rng.
    pub fn new(clock: C, rng: R) -> Self {
        MemoryIdGenerator {
            clock,
            rng,
            last_ms: 0,
            last_rand: 0,
        }
    }

    /// Produce the next id, strictly greater than the previous one.
    pub fn next_id(&mut self) -> MemoryId {
        let now_ms = self.clock.now().as_millis().max(0) as u64;
        if now_ms > self.last_ms {
            self.last_ms = now_ms;
            self.last_rand = self.rng.next_u128() & RAND_MASK;
        } else {
            // Same millisecond (or clock went backwards): stay in `last_ms` and
            // bump randomness so the id still increases.
            self.last_rand = self.last_rand.wrapping_add(1) & RAND_MASK;
        }
        MemoryId::from_parts(self.last_ms, self.last_rand)
    }
}

/// Identifies a logical agent.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct AgentId(pub u64);

/// Identifies one concurrent instance of an agent (ACC vector-clock dimension).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct AgentInstanceId(pub u64);

/// Identifies a session (a causally-ordered run of operations by one instance).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct SessionId(pub u64);

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "agent:{}", self.0)
    }
}

impl fmt::Display for AgentInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "instance:{}", self.0)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session:{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SplitMix64;
    use crate::time::{MockClock, Timestamp};

    #[test]
    fn parts_round_trip() {
        let id = MemoryId::from_parts(1_700_000_000_000, 0x0123_4567_89AB_CDEF);
        assert_eq!(id.timestamp_ms(), 1_700_000_000_000);
        assert_eq!(id.randomness(), 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn string_round_trips() {
        let id = MemoryId(0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210);
        let s = id.to_string();
        assert_eq!(s.len(), ENCODED_LEN);
        assert_eq!(id, s.parse::<MemoryId>().unwrap());
    }

    #[test]
    fn rejects_bad_strings() {
        assert_eq!(
            "tooshort".parse::<MemoryId>(),
            Err(IdParseError::WrongLength(8))
        );
        // 'U' is not in the Crockford alphabet.
        let bad = "U".repeat(26);
        assert!(matches!(
            bad.parse::<MemoryId>(),
            Err(IdParseError::InvalidChar('U'))
        ));
        // A leading char above 7 overflows 128 bits ('Z' = value 31).
        let overflow = "Z".repeat(26);
        assert_eq!(overflow.parse::<MemoryId>(), Err(IdParseError::Overflow));
    }

    #[test]
    fn lenient_decode_maps_ambiguous_chars() {
        // I/L decode as 1, O as 0 — same value as the canonical chars.
        let canonical = "0000000000000000000000000Z".parse::<MemoryId>().unwrap();
        let with_o = "OOOOOOOOOOOOOOOOOOOOOOOOOZ".parse::<MemoryId>().unwrap();
        assert_eq!(canonical, with_o);
    }

    #[test]
    fn generator_is_strictly_monotonic_within_a_millisecond() {
        let clock = MockClock::new(Timestamp::from_millis(1000));
        let mut g = MemoryIdGenerator::new(clock, SplitMix64::new(99));
        let mut prev = g.next_id();
        for _ in 0..10_000 {
            let cur = g.next_id();
            assert!(cur > prev, "{cur:?} !> {prev:?}");
            prev = cur;
        }
        // All share the same embedded timestamp.
        assert_eq!(prev.timestamp_ms(), 1000);
    }

    #[test]
    fn generator_tracks_advancing_clock() {
        use std::sync::Arc;
        let clock = Arc::new(MockClock::new(Timestamp::from_millis(1000)));
        let mut g = MemoryIdGenerator::new(Arc::clone(&clock), SplitMix64::new(1));
        let early = g.next_id();
        clock.advance(1_000_000); // +1 ms
        let late = g.next_id();
        assert_eq!(early.timestamp_ms(), 1000);
        assert_eq!(late.timestamp_ms(), 1001);
        assert!(late > early);
    }
}
