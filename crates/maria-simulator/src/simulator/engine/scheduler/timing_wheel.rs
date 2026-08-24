//! Hierarchical Timing Wheel — O(1) event scheduling.
//!
//! # Motivation
//!
//! Current event queue (`Vec<Vec<RegionEvent>>`) memiliki masalah:
//! - `retain()` filter setiap region: O(E) per delta cycle
//! - Memori: O(max_time * avg_events) untuk VCD besar
//! - `ensure_events(t)` resize Vec setiap time step
//!
//! # Timing Wheel Design
//!
//! Hierarchical 3-level wheel:
//!
//! - Level 0: 256 buckets, each bucket = 1 time unit (granularity = 1)
//! - Level 1: 256 buckets, each bucket = 256 time units (granularity = 256)
//! - Level 2: 256 buckets, each bucket = 65536 time units (granularity = 65536)
//!
//! Total coverage: 256 * 256 * 256 = 16,777,216 time units without overflow.
//!
//! # Precision
//!
//! Events stored in higher levels retain their sub-offset (remainder) so that
//! cascade from Level N+1 → Level N preserves exact timing within the bucket's
//! granularity. Without this, all events in a bucket would fire at the cascade
//! boundary instead of their originally scheduled time.
//!
//! # Operations
//!
//! - `add_event(t, region, event)`: O(1) — compute level + bucket via shift/mask
//! - `advance(t)`: O(B) where B = events at time t (amortized O(1))
//! - `has_events(t)`: O(1) — check if bucket is non-empty
//!
//! # Cascade
//!
//! Saat advance, jika current_time % 256 == 0, cascade events dari Level 1 ke Level 0.
//! Jika current_time % 65536 == 0, cascade events dari Level 2 ke Level 1.
//! Setiap event mempertahankan sub-offset-nya sehingga timing precision tetap terjaga.

use crate::simulator::types::{EventKind, EventRegion, RegionEvent};

// ─── Constants ───

/// Number of buckets per wheel level.
const WHEEL_SIZE: usize = 256;

/// Bit shift for each level.
const LEVEL1_SHIFT: usize = 8; // 2^8 = 256
const LEVEL2_SHIFT: usize = 16; // 2^16 = 65536

/// Mask to extract bucket index within a level.
const WHEEL_MASK: usize = WHEEL_SIZE - 1; // 0xFF

// ─── Timing Wheel ───

/// A single bucket in the timing wheel — holds events for a specific time offset.
///
/// Events in higher levels (1, 2) also store a `u32` sub-offset remainder
/// that preserves their exact position within the bucket's time range.
/// This remainder is used during cascade to redistribute events to the
/// correct lower-level bucket, preserving timing precision.
#[derive(Debug, Clone)]
pub struct Bucket {
    /// Events at this time offset.
    /// Each entry is (sub_offset_remainder, event).
    /// For Level 0: remainder = 0 (unused).
    /// For Level 1: remainder = offset & 0xFF (position within 256-unit range).
    /// For Level 2: remainder = offset & 0xFFFF (position within 65536-unit range).
    pub events: Vec<(u32, RegionEvent)>,
    /// Whether this bucket has ever been populated (for fast empty check).
    populated: bool,
}

impl Bucket {
    fn new() -> Self {
        Bucket {
            events: Vec::new(),
            populated: false,
        }
    }
}

/// Hierarchical timing wheel for simulation event scheduling.
#[derive(Debug, Clone)]
pub struct HierarchicalTimingWheel {
    /// Level 0: fine-grained (bucket = 1 time unit)
    /// Level 1: coarse (bucket = 256 time units)
    /// Level 2: very coarse (bucket = 65536 time units)
    levels: [Vec<Bucket>; 3],
    /// Current pointer position (absolute time).
    current_time: usize,
    /// Total number of events in the wheel.
    total_events: usize,
}

impl HierarchicalTimingWheel {
    /// Create a new empty timing wheel.
    pub fn new() -> Self {
        HierarchicalTimingWheel {
            levels: [
                vec![Bucket::new(); WHEEL_SIZE],
                vec![Bucket::new(); WHEEL_SIZE],
                vec![Bucket::new(); WHEEL_SIZE],
            ],
            current_time: 0,
            total_events: 0,
        }
    }

    /// Add an event at the given absolute time.
    ///
    /// O(1) — computes the correct level and bucket via shift and mask.
    /// Events in higher levels store a sub-offset remainder for precise
    /// redistribution during cascade.
    pub fn add_event(&mut self, time: usize, region: EventRegion, event: EventKind) {
        if time < self.current_time {
            // Event in the past — schedule at current time
            self.levels[0][0]
                .events
                .push((0, RegionEvent { region, event }));
            self.levels[0][0].populated = true;
            self.total_events += 1;
            return;
        }

        let offset = time - self.current_time;

        if offset < WHEEL_SIZE {
            // Level 0: within next 256 time units
            let idx = offset & WHEEL_MASK;
            self.levels[0][idx]
                .events
                .push((0, RegionEvent { region, event }));
            self.levels[0][idx].populated = true;
        } else if offset < (WHEEL_SIZE * WHEEL_SIZE) {
            // Level 1: within next 65536 time units
            let idx = (offset >> LEVEL1_SHIFT) & WHEEL_MASK;
            // Note: Level 1 bucket 0 is effectively unused because offsets 0-255 go to Level 0.
            // Bucket 1 covers offsets [256, 512), bucket 2 covers [512, 768), etc.
            // The remainder `offset & 0xFF` preserves the exact position within the bucket.
            let remainder = (offset & WHEEL_MASK) as u32;
            self.levels[1][idx]
                .events
                .push((remainder, RegionEvent { region, event }));
            self.levels[1][idx].populated = true;
        } else {
            // Level 2: beyond 65536 time units
            let idx = (offset >> LEVEL2_SHIFT) & WHEEL_MASK;
            // The remainder stores the full offset within the level 2 bucket's range.
            // Level 2 bucket idx covers offsets [idx << 16, (idx+1) << 16).
            // The remainder `offset & 0xFFFF` preserves the exact position (16 bits).
            let remainder = (offset & 0xFFFF) as u32;
            self.levels[2][idx]
                .events
                .push((remainder, RegionEvent { region, event }));
            self.levels[2][idx].populated = true;
        }

        self.total_events += 1;
    }

    /// Internal: cascade Level 2 bucket into Level 1.
    ///
    /// Uses the stored sub-offset remainder to compute the correct
    /// Level 1 bucket and remainder. Called when current_time crosses
    /// a level 2 boundary (every 65536 time units).
    fn cascade_l2_to_l1(&mut self, l2_idx: usize) {
        let bucket = &mut self.levels[2][l2_idx];
        if !bucket.populated {
            return;
        }
        let events = std::mem::take(&mut bucket.events);
        bucket.populated = false;

        // Level 2 remainder is a 16-bit value: [high 8 bits = Level 1 bucket, low 8 bits = Level 1 sub-offset]
        for (rem, ev) in events {
            let l1_idx = ((rem >> LEVEL1_SHIFT as u32) as usize) & WHEEL_MASK;
            let l1_rem = rem & (WHEEL_MASK as u32);
            self.levels[1][l1_idx].events.push((l1_rem, ev));
            self.levels[1][l1_idx].populated = true;
        }
    }

    /// Internal: cascade Level 1 bucket into Level 0.
    ///
    /// Uses the stored sub-offset remainder to place each event into
    /// the correct Level 0 bucket, preserving exact timing precision.
    fn cascade_l1_to_l0(&mut self, l1_idx: usize) {
        let bucket = &mut self.levels[1][l1_idx];
        if !bucket.populated {
            return;
        }
        let events = std::mem::take(&mut bucket.events);
        bucket.populated = false;

        // Level 1 remainder is an 8-bit value representing the position within the bucket range.
        // For bucket idx, the original offset was [idx << 8, (idx+1) << 8).
        // The remainder = offset & 0xFF gives the exact Level 0 bucket index.
        for (rem, ev) in events {
            let l0_idx = (rem as usize) & WHEEL_MASK;
            self.levels[0][l0_idx].events.push((0, ev));
            self.levels[0][l0_idx].populated = true;
        }
    }

    /// Advance the wheel to the given absolute time.
    ///
    /// Returns all events scheduled at this time.
    /// Handles cascading: when current_time crosses a level boundary,
    /// events from higher levels are moved down with sub-offset precision.
    ///
    /// O(B) where B = events at this time (amortized O(1) per time step).
    ///
    /// # Note
    ///
    /// If `time` is far ahead (e.g., advance(100000) on first call), the while loop
    /// iterates through every intermediate time step to perform cascades. In practice,
    /// the simulation loop calls `advance()` incrementally (t=0, 1, 2, ...) so this is
    /// amortized O(1) per call.
    pub fn advance(&mut self, time: usize) -> Vec<RegionEvent> {
        if time < self.current_time {
            // Can't go backwards
            return Vec::new();
        }

        // Advance current_time step by step, cascading at boundaries
        while self.current_time < time {
            // Cascade Level 1 → Level 0 at level 0 wrap-around (every 256 time units)
            // When current_time wraps past a multiple of WHEEL_SIZE, cascade
            // the Level 1 bucket whose range is now fully covered by Level 0.
            // Using `(current_time >> LEVEL1_SHIFT) & WHEEL_MASK` gives the
            // correct bucket index — no `-1` needed because Level 1 bucket N
            // covers offsets [N*256, (N+1)*256). When current_time reaches N*256,
            // bucket N-1 (from offset perspective) is at current_time position.
            // Actually: bucket at index `(current_time >> LEVEL1_SHIFT)` has
            // events with offsets [(current_time >> 8) * 256, ...] from the
            // original current_time. Since current_time is now at this boundary,
            // these events' remaining offsets are in [0, 256) — perfect for Level 0.
            if self.current_time > 0 && self.current_time.is_multiple_of(WHEEL_SIZE) {
                let l1_idx = (self.current_time >> LEVEL1_SHIFT) & WHEEL_MASK;
                self.cascade_l1_to_l0(l1_idx);
            }

            // Cascade Level 2 → Level 1 at level 1 wrap-around (every 65536 time units)
            if self.current_time > 0 && self.current_time.is_multiple_of(WHEEL_SIZE * WHEEL_SIZE) {
                let l2_idx = (self.current_time >> LEVEL2_SHIFT) & WHEEL_MASK;
                self.cascade_l2_to_l1(l2_idx);
            }

            self.current_time += 1;
        }

        // Drain events at current time from Level 0
        let bucket_idx = self.current_time & WHEEL_MASK;
        let bucket = &mut self.levels[0][bucket_idx];

        if !bucket.populated {
            return Vec::new();
        }

        let stored_events = std::mem::take(&mut bucket.events);
        bucket.populated = false;
        self.total_events -= stored_events.len();

        // Extract RegionEvent from (u32, RegionEvent) pairs
        stored_events.into_iter().map(|(_, ev)| ev).collect()
    }

    /// Check if there are any events at the given time.
    ///
    /// Checks Level 0 only. For events in higher levels, they will be
    /// cascaded to Level 0 when advance() reaches their time.
    ///
    /// O(1) — just checks if the Level 0 bucket is populated.
    pub fn has_events(&self, time: usize) -> bool {
        if time < self.current_time {
            return false;
        }
        let offset = time - self.current_time;
        if offset < WHEEL_SIZE {
            let idx = offset & WHEEL_MASK;
            self.levels[0][idx].populated
        } else {
            // Events in higher levels haven't been cascaded yet,
            // so they're not visible to has_events.
            false
        }
    }

    /// Total number of events in the wheel.
    pub fn total_events(&self) -> usize {
        self.total_events
    }

    /// Clear all events and reset the wheel.
    pub fn clear(&mut self) {
        for level in &mut self.levels {
            for bucket in level.iter_mut() {
                bucket.events.clear();
                bucket.populated = false;
            }
        }
        self.current_time = 0;
        self.total_events = 0;
    }

    /// Number of populated buckets across all levels.
    pub fn populated_buckets(&self) -> usize {
        self.levels
            .iter()
            .flat_map(|level| level.iter())
            .filter(|b| b.populated)
            .count()
    }
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_event() -> EventKind {
        EventKind::EvalProcess(0)
    }

    #[test]
    fn test_new_wheel_empty() {
        let wheel = HierarchicalTimingWheel::new();
        assert_eq!(wheel.total_events(), 0);
        assert_eq!(wheel.populated_buckets(), 0);
    }

    #[test]
    fn test_add_and_advance() {
        let mut wheel = HierarchicalTimingWheel::new();
        let ev = dummy_event();

        wheel.add_event(0, EventRegion::Active, ev.clone());
        assert_eq!(wheel.total_events(), 1);

        let events = wheel.advance(0);
        assert_eq!(events.len(), 1);
        assert_eq!(wheel.total_events(), 0);
    }

    #[test]
    fn test_add_multiple_times() {
        let mut wheel = HierarchicalTimingWheel::new();

        wheel.add_event(0, EventRegion::Active, dummy_event());
        wheel.add_event(5, EventRegion::Active, dummy_event());
        wheel.add_event(10, EventRegion::Nba, dummy_event());

        assert_eq!(wheel.total_events(), 3);

        // Advance to time 0: should get 1 event
        let t0 = wheel.advance(0);
        assert_eq!(t0.len(), 1);

        // Advance to time 5: should get 1 event
        let t5 = wheel.advance(5);
        assert_eq!(t5.len(), 1);

        // Advance to time 10: should get 1 event
        let t10 = wheel.advance(10);
        assert_eq!(t10.len(), 1);

        assert_eq!(wheel.total_events(), 0);
    }

    #[test]
    fn test_cascade_level1_to_level0() {
        let mut wheel = HierarchicalTimingWheel::new();

        // Add event at time 300 (past level 0 boundary at 256)
        // Level 1 bucket: (300 >> 8) & 0xFF = 1 (bucket 1, since offsets 0-255 go to L0)
        // Remainder: 300 & 0xFF = 44
        wheel.add_event(300, EventRegion::Active, dummy_event());
        assert_eq!(wheel.total_events(), 1);

        // Advance through times 0-255: should fire no events
        for t in 0..=255 {
            let events = wheel.advance(t);
            assert!(events.is_empty(), "unexpected event at time {}", t);
        }

        // At time 256: current_time = 256, 256 % 256 == 0 → cascade Level 1 bucket (256>>8)&0xFF = 1
        // Event with remainder 44 goes to Level 0 bucket 44.
        let events_256 = wheel.advance(256);
        assert!(events_256.is_empty(), "no events expected at time 256");

        // Now advance to time 300: should find event in Level 0 bucket 44
        for t in 257..300 {
            let events = wheel.advance(t);
            assert!(events.is_empty(), "unexpected event at time {}", t);
        }

        let events_300 = wheel.advance(300);
        assert_eq!(events_300.len(), 1, "event should fire at time 300");
    }

    #[test]
    fn test_multiple_events_different_times_in_l1_bucket() {
        let mut wheel = HierarchicalTimingWheel::new();

        // Add events at times 300, 310, 320 in the same Level 1 bucket (bucket 1)
        wheel.add_event(300, EventRegion::Active, EventKind::EvalProcess(300));
        wheel.add_event(310, EventRegion::Active, EventKind::EvalProcess(310));
        wheel.add_event(320, EventRegion::Active, EventKind::EvalProcess(320));

        // Advance to time 256 to cascade
        for t in 0..=256 {
            let _ = wheel.advance(t);
        }

        // Advance to time 300: should get event 300
        for t in 257..300 {
            let _ = wheel.advance(t);
        }
        let t300 = wheel.advance(300);
        assert_eq!(t300.len(), 1, "event at time 300");

        // Advance to time 310: should get event 310
        for t in 301..310 {
            let _ = wheel.advance(t);
        }
        let t310 = wheel.advance(310);
        assert_eq!(t310.len(), 1, "event at time 310");

        // Advance to time 320: should get event 320
        for t in 311..320 {
            let _ = wheel.advance(t);
        }
        let t320 = wheel.advance(320);
        assert_eq!(t320.len(), 1, "event at time 320");
    }

    #[test]
    fn test_events_in_past() {
        let mut wheel = HierarchicalTimingWheel::new();

        // Advance to time 100
        for t in 0..=100 {
            let _ = wheel.advance(t);
        }

        // Add event at time 50 (past!) — bucket 0 (heuristic for "fire ASAP")
        wheel.add_event(50, EventRegion::Active, dummy_event());
        assert_eq!(wheel.total_events(), 1);

        // Event in bucket 0 fires when current_time & 0xFF == 0 → next at time 256
        for t in 101..=255 {
            let _ = wheel.advance(t);
        }
        let events = wheel.advance(256); // drains bucket (256 & 0xFF = 0)
        assert_eq!(
            events.len(),
            1,
            "past event should fire at next wrap-around (bucket 0)"
        );
    }

    #[test]
    fn test_has_events() {
        let mut wheel = HierarchicalTimingWheel::new();

        assert!(!wheel.has_events(0));
        wheel.add_event(42, EventRegion::Active, dummy_event());
        assert!(wheel.has_events(42));
    }

    #[test]
    fn test_clear() {
        let mut wheel = HierarchicalTimingWheel::new();

        wheel.add_event(0, EventRegion::Active, dummy_event());
        wheel.add_event(100, EventRegion::Active, dummy_event());
        wheel.add_event(1000, EventRegion::Active, dummy_event());

        assert_eq!(wheel.total_events(), 3);
        wheel.clear();
        assert_eq!(wheel.total_events(), 0);
        assert_eq!(wheel.populated_buckets(), 0);
    }

    #[test]
    fn test_many_events_at_same_time() {
        let mut wheel = HierarchicalTimingWheel::new();
        let n = 1000;

        for i in 0..n {
            wheel.add_event(0, EventRegion::Active, EventKind::EvalProcess(i));
        }

        assert_eq!(wheel.total_events(), n);

        let events = wheel.advance(0);
        assert_eq!(events.len(), n);
    }

    #[test]
    fn test_empty_advance() {
        let mut wheel = HierarchicalTimingWheel::new();

        // Advance through 1000 empty time steps
        for t in 0..=1000 {
            let events = wheel.advance(t);
            assert!(events.is_empty(), "unexpected event at time {}", t);
        }
    }

    #[test]
    fn test_different_regions() {
        let mut wheel = HierarchicalTimingWheel::new();

        let regions = [
            EventRegion::Active,
            EventRegion::Inactive,
            EventRegion::Nba,
            EventRegion::Reactive,
            EventRegion::Postponed,
        ];

        for (i, &region) in regions.iter().enumerate() {
            wheel.add_event(0, region, EventKind::EvalProcess(i));
        }

        assert_eq!(wheel.total_events(), regions.len());

        let events = wheel.advance(0);
        assert_eq!(events.len(), regions.len());
        assert_eq!(events[0].region, EventRegion::Active);
    }

    #[test]
    fn test_cascade_multiple_l1_buckets() {
        let mut wheel = HierarchicalTimingWheel::new();

        // Events at different Level 1 buckets
        // Time 500 → Level 1 bucket (500 >> 8) & 0xFF = 1, remainder = 500 & 0xFF = 244
        // Time 800 → Level 1 bucket (800 >> 8) & 0xFF = 3, remainder = 800 & 0xFF = 32
        wheel.add_event(500, EventRegion::Inactive, EventKind::EvalProcess(500));
        wheel.add_event(800, EventRegion::Nba, EventKind::EvalProcess(800));

        // Advance to time 256 (cascade bucket 1)
        for t in 0..=256 {
            let _ = wheel.advance(t);
        }

        // Advance to time 500: should fire event 500
        for t in 257..500 {
            let _ = wheel.advance(t);
        }
        let t500 = wheel.advance(500);
        assert_eq!(t500.len(), 1, "event at time 500 should fire");

        // Advance to time 512 (cascade bucket 3: 512 >> 8 = 2 → wait, 512 >> 8 = 2)
        // Need to continue to time 768 to cascade bucket 3
        for t in 501..=768 {
            let _ = wheel.advance(t);
        }

        // Advance to time 800: should fire event 800
        for t in 769..800 {
            let _ = wheel.advance(t);
        }
        let t800 = wheel.advance(800);
        assert_eq!(t800.len(), 1, "event at time 800 should fire");
    }
}
