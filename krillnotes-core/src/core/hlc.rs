// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

//! Hybrid Logical Clock (HLC) for distributed ordering of operations.
//!
//! An HLC combines a wall-clock timestamp with a logical counter to provide
//! causally consistent ordering across nodes without tight clock synchronization.

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Transaction};
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// A Hybrid Logical Clock timestamp.
///
/// Serializes as a compact 3-element JSON array `[wall_ms, counter, node_id]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HlcTimestamp {
    pub wall_ms: u64,
    pub counter: u32,
    pub node_id: u32,
}

impl HlcTimestamp {
    /// Construct an `HlcTimestamp` from a Unix timestamp in seconds.
    ///
    /// This is a compatibility helper for code that still generates
    /// `chrono::Utc::now().timestamp()` (seconds). Counter and node_id
    /// are zeroed; these timestamps should be replaced with proper HLC
    /// values once the workspace acquires an `HlcClock`.
    pub fn from_unix_secs(secs: i64) -> Self {
        HlcTimestamp {
            wall_ms: (secs.max(0) as u64).saturating_mul(1_000),
            counter: 0,
            node_id: 0,
        }
    }
}

impl Ord for HlcTimestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.wall_ms
            .cmp(&other.wall_ms)
            .then(self.counter.cmp(&other.counter))
            .then(self.node_id.cmp(&other.node_id))
    }
}

impl PartialOrd for HlcTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Serialize for HlcTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tup = serializer.serialize_tuple(3)?;
        tup.serialize_element(&self.wall_ms)?;
        tup.serialize_element(&self.counter)?;
        tup.serialize_element(&self.node_id)?;
        tup.end()
    }
}

struct HlcTimestampVisitor;

impl<'de> Visitor<'de> for HlcTimestampVisitor {
    type Value = HlcTimestamp;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a 3-element array [wall_ms, counter, node_id]")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<HlcTimestamp, A::Error> {
        let wall_ms: u64 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let counter: u32 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
        let node_id: u32 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
        Ok(HlcTimestamp {
            wall_ms,
            counter,
            node_id,
        })
    }
}

impl<'de> Deserialize<'de> for HlcTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<HlcTimestamp, D::Error> {
        deserializer.deserialize_tuple(3, HlcTimestampVisitor)
    }
}

/// Derive a stable 32-bit node ID from a device UUID.
///
/// Uses the first 4 bytes of a BLAKE3 hash of the device UUID bytes.
pub fn node_id_from_device(device_id: &Uuid) -> u32 {
    let hash = blake3::hash(device_id.as_bytes());
    u32::from_le_bytes(hash.as_bytes()[..4].try_into().unwrap())
}

/// Returns the current wall-clock time as milliseconds since the Unix epoch.
fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// A Hybrid Logical Clock that can be advanced locally or by observing remote timestamps.
pub struct HlcClock {
    wall_ms: u64,
    counter: u32,
    node_id: u32,
}

impl HlcClock {
    /// Create a new clock with the given node ID, starting at epoch zero.
    pub fn new(node_id: u32) -> Self {
        HlcClock {
            wall_ms: 0,
            counter: 0,
            node_id,
        }
    }

    /// Saturate a counter at `u32::MAX` rather than wrapping or panicking.
    ///
    /// In practice this requires billions of operations per millisecond, which is
    /// impossible in any real deployment. If saturation does occur, timestamps at the
    /// same `wall_ms` are still unique because they carry different `node_id` values.
    fn saturating_increment(counter: u32) -> u32 {
        counter.saturating_add(1)
    }

    /// Advance the clock and return the next timestamp.
    ///
    /// Guarantees monotonically increasing timestamps.
    pub fn now(&mut self) -> HlcTimestamp {
        let wall = wall_clock_ms();
        let new_wall_ms = wall.max(self.wall_ms);
        let counter = if new_wall_ms > self.wall_ms {
            0
        } else {
            Self::saturating_increment(self.counter)
        };
        self.wall_ms = new_wall_ms;
        self.counter = counter;
        HlcTimestamp {
            wall_ms: new_wall_ms,
            counter,
            node_id: self.node_id,
        }
    }

    /// Update the clock by observing a remote timestamp.
    ///
    /// Does not return a timestamp — use `now()` afterwards if you need one.
    pub fn observe(&mut self, remote: HlcTimestamp) {
        let wall = wall_clock_ms();
        let new_wall_ms = wall.max(self.wall_ms).max(remote.wall_ms);
        let counter = if self.wall_ms == remote.wall_ms && remote.wall_ms == new_wall_ms {
            // All three agree — take max counter and increment
            Self::saturating_increment(self.counter.max(remote.counter))
        } else if self.wall_ms == new_wall_ms {
            // Local wall clock led
            Self::saturating_increment(self.counter)
        } else if remote.wall_ms == new_wall_ms {
            // Remote wall clock led
            Self::saturating_increment(remote.counter)
        } else {
            // Physical clock led (new_wall_ms == wall, which is ahead of both)
            0
        };
        self.wall_ms = new_wall_ms;
        self.counter = counter;
    }

    /// Load the HLC state from the `hlc_state` table.
    ///
    /// Returns `Err` if the table does not exist yet; returns `HlcClock::new(node_id)` if the
    /// table exists but has no row. The `node_id` parameter is used only for the fallback —
    /// when a row exists, the stored `node_id` takes priority.
    pub fn load_from_db(conn: &Connection, node_id: u32) -> Result<Self, rusqlite::Error> {
        let result = conn.query_row(
            "SELECT wall_ms, counter, node_id FROM hlc_state WHERE id = 1",
            [],
            |row| {
                // rusqlite does not implement FromSql/ToSql for u64/u32; store as i64 and cast.
                // SQLite INTEGER is signed i64; wall_ms won't overflow i64 until year ~292M — safe cast.
                let wall_ms = row.get::<_, i64>(0)? as u64;
                let counter = row.get::<_, i64>(1)? as u32;
                let node_id = row.get::<_, i64>(2)? as u32;
                Ok(HlcClock {
                    wall_ms,
                    counter,
                    node_id,
                })
            },
        );
        match result {
            Ok(clock) => Ok(clock),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(HlcClock::new(node_id)),
            Err(e) => Err(e),
        }
    }

    /// Persist the current HLC state to the `hlc_state` table within a transaction.
    pub fn save_to_db(&self, tx: &Transaction) -> Result<(), rusqlite::Error> {
        // Cast to i64 — rusqlite has no ToSql impl for u64/u32.
        // SQLite INTEGER is signed i64; wall_ms won't overflow i64 until year ~292M — safe cast.
        tx.execute(
            "INSERT OR REPLACE INTO hlc_state (id, wall_ms, counter, node_id) VALUES (1, ?, ?, ?)",
            rusqlite::params![self.wall_ms as i64, self.counter as i64, self.node_id as i64],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_monotonic() {
        let mut clock = HlcClock::new(1);
        let mut prev = clock.now();
        for _ in 0..999 {
            let next = clock.now();
            assert!(
                next >= prev,
                "clock went backwards: {:?} < {:?}",
                next,
                prev
            );
            prev = next;
        }
    }

    #[test]
    fn observe_updates_clock_local_wins() {
        let mut clock = HlcClock::new(1);
        // Use a far-future wall_ms so the real system time does not overtake the seeded value.
        // This ensures the "local led" branch is taken (self.wall_ms == new_wall_ms).
        let future_ms = 2_000_000_000_000u64; // year 2033+
        clock.wall_ms = future_ms;
        clock.counter = 5;

        let remote = HlcTimestamp {
            wall_ms: future_ms - 1_000,
            counter: 10,
            node_id: 2,
        };
        clock.observe(remote);
        // Local wall_ms dominates (it was already in the future relative to system time)
        assert_eq!(clock.wall_ms, future_ms, "wall_ms should stay at future_ms");
        // Local led: counter = self.counter + 1 = 6
        assert_eq!(clock.counter, 6, "counter should be self.counter + 1");
    }

    #[test]
    fn observe_updates_clock_remote_wins() {
        let mut clock = HlcClock::new(1);
        clock.wall_ms = 500;
        clock.counter = 3;

        let remote = HlcTimestamp {
            wall_ms: 2_000_000_000_000, // far future
            counter: 7,
            node_id: 2,
        };
        clock.observe(remote);
        assert_eq!(clock.wall_ms, 2_000_000_000_000);
        // remote led, so counter = remote.counter + 1
        assert_eq!(clock.counter, 8);
    }

    #[test]
    fn observe_updates_clock_tie() {
        let mut clock = HlcClock::new(1);
        // Set wall_ms to the same value as the remote AND ensure system time does not exceed it.
        // We use a large future value so the real clock won't catch up.
        let future_ms = 2_000_000_000_000u64; // year 2033+
        clock.wall_ms = future_ms;
        clock.counter = 3;

        let remote = HlcTimestamp {
            wall_ms: future_ms,
            counter: 5,
            node_id: 2,
        };
        clock.observe(remote);
        // Both local and remote share new_wall_ms → counter = max(3,5)+1 = 6
        assert_eq!(clock.wall_ms, future_ms);
        assert_eq!(clock.counter, 6);
    }

    #[test]
    fn node_id_from_device_is_stable() {
        let uuid1 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let uuid2 = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();

        let id1a = node_id_from_device(&uuid1);
        let id1b = node_id_from_device(&uuid1);
        let id2 = node_id_from_device(&uuid2);

        assert_eq!(id1a, id1b, "same UUID must produce same node_id");
        assert_ne!(id1a, id2, "different UUIDs should produce different node_ids");
    }

    #[test]
    fn hlc_timestamp_serde_round_trip() {
        let ts = HlcTimestamp {
            wall_ms: 1_700_000_000_000,
            counter: 42,
            node_id: 99,
        };

        let json = serde_json::to_string(&ts).unwrap();
        // Must be a 3-element array, not an object
        assert_eq!(json, "[1700000000000,42,99]");

        let decoded: HlcTimestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ts);
    }

    #[test]
    fn hlc_timestamp_ordering() {
        let a = HlcTimestamp {
            wall_ms: 100,
            counter: 0,
            node_id: 1,
        };
        let b = HlcTimestamp {
            wall_ms: 200,
            counter: 0,
            node_id: 1,
        };
        let c = HlcTimestamp {
            wall_ms: 200,
            counter: 1,
            node_id: 1,
        };
        let d = HlcTimestamp {
            wall_ms: 200,
            counter: 1,
            node_id: 2,
        };

        assert!(a < b, "earlier wall_ms must be less");
        assert!(b < c, "same wall_ms, higher counter must be greater");
        assert!(c < d, "same wall_ms+counter, higher node_id must be greater");
        assert_eq!(a, a, "reflexive equality");
    }
}
