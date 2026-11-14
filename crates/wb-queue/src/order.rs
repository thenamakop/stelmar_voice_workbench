//! In-order commit (Phase 4 step 2). Work units finish in any order on two engines; the app buffers the
//! finished ones and commits them in audio order, advancing `cursor_ms` only over a continuous committed
//! run, so the transcript never jumps and a unit is never committed before the unit before it.

use std::collections::BTreeMap;

use crate::{Error, Result};

pub struct OrderBuffer<T> {
    /// The index of the next unit that may be committed.
    next: u32,
    cursor_ms: u64,
    waiting: BTreeMap<u32, (u64, T)>,
}

impl<T> OrderBuffer<T> {
    /// `next_idx` is the first unit not yet committed and `cursor_ms` the committed cursor (a restart passes
    /// what the database holds).
    pub fn new(next_idx: u32, cursor_ms: u64) -> Self {
        OrderBuffer {
            next: next_idx,
            cursor_ms,
            waiting: BTreeMap::new(),
        }
    }

    /// A unit finished decoding. `end_ms` is the end of its audio range. Finishing a unit twice, or a unit
    /// that is already committed, is an error (the caller has lost track of it).
    pub fn finished(&mut self, idx: u32, end_ms: u64, payload: T) -> Result<()> {
        if idx < self.next {
            return Err(Error::Invalid(format!("unit {idx} is already committed")));
        }
        if self.waiting.contains_key(&idx) {
            return Err(Error::Invalid(format!("unit {idx} finished twice")));
        }
        self.waiting.insert(idx, (end_ms, payload));
        Ok(())
    }

    /// The next unit that may be committed now, if it has finished: its index, its end and its payload. The
    /// cursor moves to its end.
    pub fn pop_ready(&mut self) -> Option<(u32, u64, T)> {
        let (end_ms, payload) = self.waiting.remove(&self.next)?;
        let idx = self.next;
        self.next += 1;
        self.cursor_ms = self.cursor_ms.max(end_ms);
        Some((idx, end_ms, payload))
    }

    /// Every unit that may be committed now, in audio order.
    pub fn drain_ready(&mut self) -> Vec<(u32, u64, T)> {
        std::iter::from_fn(|| self.pop_ready()).collect()
    }

    /// Everything up to here is committed.
    pub fn cursor_ms(&self) -> u64 {
        self.cursor_ms
    }

    pub fn next_idx(&self) -> u32 {
        self.next
    }

    /// Units that finished but wait for an earlier one.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Forgets a finished unit that was not committed (its engine's output is being redone).
    pub fn discard(&mut self, idx: u32) -> bool {
        self.waiting.remove(&idx).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_later_unit_waits_for_the_earlier_one() {
        let mut o = OrderBuffer::new(0, 0);
        o.finished(1, 600, "b").unwrap();
        assert!(o.pop_ready().is_none());
        assert_eq!(
            o.cursor_ms(),
            0,
            "the cursor does not skip over the unit that is missing"
        );
        o.finished(0, 300, "a").unwrap();
        assert_eq!(o.drain_ready(), vec![(0, 300, "a"), (1, 600, "b")]);
        assert_eq!(o.cursor_ms(), 600);
    }

    #[test]
    fn a_unit_cannot_finish_twice_or_after_it_was_committed() {
        let mut o = OrderBuffer::new(0, 0);
        o.finished(0, 300, ()).unwrap();
        assert!(o.finished(0, 300, ()).is_err());
        o.pop_ready().unwrap();
        assert!(o.finished(0, 300, ()).is_err());
    }

    #[test]
    fn a_restart_continues_from_the_committed_cursor() {
        let mut o = OrderBuffer::new(3, 900);
        o.finished(3, 1_200, ()).unwrap();
        assert_eq!(o.pop_ready().map(|p| p.0), Some(3));
        assert_eq!(o.cursor_ms(), 1_200);
    }

    proptest! {
        /// Units finish in any order on two engines; they commit in index order, each exactly once, and the
        /// cursor never moves back and never passes a unit that is still missing.
        #[test]
        fn any_completion_order_commits_in_audio_order(order in Just((0u32..40).collect::<Vec<_>>()).prop_shuffle()) {
            let mut o = OrderBuffer::new(0, 0);
            let mut committed = Vec::new();
            let mut cursor = 0;
            let mut finished = std::collections::BTreeSet::new();
            for idx in order {
                o.finished(idx, u64::from(idx + 1) * 300_000, idx).unwrap();
                finished.insert(idx);
                for (i, end, _) in o.drain_ready() {
                    committed.push(i);
                    prop_assert_eq!(end, u64::from(i + 1) * 300_000);
                }
                prop_assert!(o.cursor_ms() >= cursor);
                cursor = o.cursor_ms();
                // The cursor is the end of the longest continuous run of finished units from 0.
                let run = (0u32..).take_while(|i| finished.contains(i)).count() as u64;
                prop_assert_eq!(cursor, run * 300_000);
            }
            prop_assert_eq!(committed, (0u32..40).collect::<Vec<_>>());
        }
    }
}
