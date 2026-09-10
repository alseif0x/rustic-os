// SPDX-License-Identifier: Apache-2.0
//! Bounded owner-operation identity and results, separate from transport and execution.
pub const PENDING: u64 = 5;
#[derive(Clone, Copy)]
pub struct Ticket {
    pub id: u64,
    pub kind: u64,
}
#[derive(Clone, Copy)]
struct Completion {
    ticket: Ticket,
    result: [u64; 8],
}
#[derive(Default)]
pub struct History {
    next: u64,
    active: Option<Ticket>,
    completed: [Option<Completion>; 2],
    cursor: usize,
}
impl History {
    pub fn start(&mut self, kind: u64) -> Option<Ticket> {
        if self.active.is_some() {
            return None;
        }
        self.next = self.next.checked_add(1)?;
        let t = Ticket {
            id: self.next,
            kind,
        };
        self.active = Some(t);
        Some(t)
    }
    pub fn finish(&mut self, id: u64, result: [u64; 8]) -> bool {
        let Some(t) = self.active.filter(|t| t.id == id) else {
            return false;
        };
        self.active = None;
        self.completed[self.cursor] = Some(Completion { ticket: t, result });
        self.cursor = (self.cursor + 1) % 2;
        true
    }
    pub fn status(&self, mut id: u64, phase: u64, server: u64, io: u64) -> Option<[u64; 8]> {
        if id == 0 {
            id = self.next;
        }
        if let Some(t) = self.active.filter(|t| t.id == id) {
            return Some([PENDING, id, t.kind, phase, server, io, 0, 0]);
        }
        self.completed
            .iter()
            .flatten()
            .find(|c| c.ticket.id == id)
            .map(|c| {
                [
                    0,
                    id,
                    c.ticket.kind,
                    c.result[0],
                    c.result[1],
                    c.result[2],
                    c.result[3],
                    0,
                ]
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_completion_cannot_complete_a_new_operation() {
        let mut h = History::default();
        let a = h.start(11).unwrap();
        assert!(h.start(3).is_none());
        assert!(!h.finish(a.id + 1, [0; 8]));
        assert_eq!(h.status(0, 2, 7, 1).unwrap()[0], PENDING);
        assert!(h.finish(a.id, [6, 0, 0, 0, 0, 0, 0, 0]));
        let b = h.start(3).unwrap();
        assert!(!h.finish(a.id, [0; 8]));
        assert_eq!(h.status(a.id, 0, 0, 0).unwrap()[3], 6);
        assert_eq!(h.status(b.id, 1, 7, 0).unwrap()[0], PENDING);
    }
    #[test]
    fn result_retention_is_bounded_and_unknown_ids_are_not_success() {
        let mut h = History::default();
        for _ in 0..3 {
            let t = h.start(3).unwrap();
            assert!(h.finish(t.id, [0, 9, 0, 0, 0, 0, 0, 0]));
        }
        assert!(h.status(1, 0, 0, 0).is_none());
        assert!(h.status(4, 0, 0, 0).is_none());
        assert_eq!(h.status(0, 0, 0, 0).unwrap()[4], 9);
    }
}
