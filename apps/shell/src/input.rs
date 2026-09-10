// SPDX-License-Identifier: Apache-2.0
//! Owned bounded typeahead. Interrupted or overflowing lines never execute a prefix.
pub struct Queue {
    bytes: [u8; 1024],
    head: usize,
    len: usize,
    discard: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Buffered,
    Interrupted,
    Overflow,
}
impl Default for Queue {
    fn default() -> Self {
        Self {
            bytes: [0; 1024],
            head: 0,
            len: 0,
            discard: false,
        }
    }
}
impl Queue {
    pub fn discarding(&self) -> bool {
        self.discard
    }
    pub fn push(&mut self, byte: u8) -> Event {
        if byte == 3 {
            self.head = 0;
            self.len = 0;
            self.discard = false;
            return Event::Interrupted;
        }
        if self.discard {
            if matches!(byte, b'\r' | b'\n') {
                self.discard = false;
            }
            return Event::Buffered;
        }
        if self.len == self.bytes.len() {
            self.head = 0;
            self.len = 0;
            self.discard = !matches!(byte, b'\r' | b'\n');
            return Event::Overflow;
        }
        self.bytes[(self.head + self.len) % self.bytes.len()] = byte;
        self.len += 1;
        Event::Buffered
    }
    pub fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let b = self.bytes[self.head];
        self.head = (self.head + 1) % self.bytes.len();
        self.len -= 1;
        Some(b)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interruption_discards_partial_input_but_keeps_following_commands() {
        let mut q = Queue::default();
        q.push(b'x');
        assert_eq!(q.push(3), Event::Interrupted);
        assert_eq!(q.pop(), None);
        for b in b"mem\r" {
            q.push(*b);
        }
        for b in b"mem\r" {
            assert_eq!(q.pop(), Some(*b));
        }
        assert_eq!(q.pop(), None);
    }
    #[test]
    fn overflow_never_executes_a_truncated_line_and_wrap_preserves_order() {
        let mut q = Queue::default();
        for _ in 0..1024 {
            q.push(b'x');
        }
        assert_eq!(q.push(b'y'), Event::Overflow);
        q.push(b'z');
        q.push(b'\r');
        assert_eq!(q.pop(), None);
        for _ in 0..2048 {
            q.push(b'a');
            assert_eq!(q.pop(), Some(b'a'));
        }
        assert_eq!(q.pop(), None);
        for _ in 0..1024 {
            q.push(b'x');
        }
        assert_eq!(q.push(b'\r'), Event::Overflow);
        assert!(!q.discarding());
        q.push(b'b');
        assert_eq!(q.pop(), Some(b'b'));
    }
}
