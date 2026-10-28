use crate::settings::PadId;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueueEntry {
    pub id: u64,
    pub pad: PadId,
}

/// What's playing and what's next. Entry ids start at 1 and are never reused.
#[derive(Default)]
pub struct Queue {
    current: Option<QueueEntry>,
    up_next: VecDeque<QueueEntry>,
    next_id: u64,
}

impl Queue {
    pub fn enqueue(&mut self, pad: PadId) -> u64 {
        self.next_id += 1;
        self.up_next.push_back(QueueEntry { id: self.next_id, pad });
        self.next_id
    }

    pub fn advance(&mut self) -> Option<QueueEntry> {
        self.current = self.up_next.pop_front();
        self.current
    }

    pub fn stop(&mut self) {
        self.current = None;
        self.up_next.clear();
    }

    pub fn remove(&mut self, entry: u64) -> bool {
        let before = self.up_next.len();
        self.up_next.retain(|e| e.id != entry);
        self.up_next.len() != before
    }

    pub fn move_entry(&mut self, entry: u64, index: usize) {
        if let Some(from) = self.up_next.iter().position(|e| e.id == entry) {
            let e = self.up_next.remove(from).unwrap();
            self.up_next.insert(index.min(self.up_next.len()), e);
        }
    }

    /// Purge a deleted pad from up-next; true if it's the one playing now.
    pub fn remove_pad(&mut self, pad: PadId) -> bool {
        self.up_next.retain(|e| e.pad != pad);
        self.current.is_some_and(|e| e.pad == pad)
    }

    pub fn current(&self) -> Option<QueueEntry> {
        self.current
    }

    pub fn up_next(&self) -> &VecDeque<QueueEntry> {
        &self.up_next
    }

    pub fn position_of(&self, pad: PadId) -> Option<usize> {
        self.up_next.iter().position(|e| e.pad == pad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pads(q: &Queue) -> Vec<PadId> {
        q.up_next().iter().map(|e| e.pad).collect()
    }
    fn ids(q: &Queue) -> Vec<u64> {
        q.up_next().iter().map(|e| e.id).collect()
    }

    #[test]
    fn order_holds_across_enqueue_skip_remove_reorder_stop() {
        let mut q = Queue::default();
        let (a, b, c, d) = (q.enqueue(1), q.enqueue(2), q.enqueue(3), q.enqueue(2));
        assert_eq!(q.advance().map(|e| e.id), Some(a));
        assert_eq!(pads(&q), vec![2, 3, 2]);
        assert!(q.remove(c));
        assert_eq!(pads(&q), vec![2, 2]);
        q.move_entry(d, 0);
        assert_eq!(ids(&q), vec![d, b]);
        assert_eq!(q.position_of(2), Some(0));
        assert_eq!(q.advance().map(|e| e.id), Some(d));
        assert_eq!(q.current().map(|e| e.id), Some(d));
        q.stop();
        assert!(q.current().is_none() && q.up_next().is_empty());
        assert!(q.advance().is_none());
    }

    #[test]
    fn same_pad_twice_gets_distinct_ids() {
        let mut q = Queue::default();
        assert_ne!(q.enqueue(7), q.enqueue(7));
    }

    #[test]
    fn move_entry_clamps_and_ignores_unknown() {
        let mut q = Queue::default();
        let (a, b) = (q.enqueue(1), q.enqueue(2));
        q.move_entry(a, 99);
        assert_eq!(ids(&q), vec![b, a]);
        q.move_entry(12345, 0);
        assert_eq!(ids(&q), vec![b, a]);
        assert!(!q.remove(12345));
    }

    #[test]
    fn deleting_a_pad_purges_it() {
        let mut q = Queue::default();
        q.enqueue(1);
        q.enqueue(2);
        q.enqueue(1);
        q.advance();
        assert!(q.remove_pad(1));
        assert_eq!(pads(&q), vec![2]);
    }
}
