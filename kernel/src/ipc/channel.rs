// SPDX-License-Identifier: Apache-2.0
use super::{Error, Message};

struct Queue {
    messages: [Option<Message>; 2],
    head: usize,
    len: usize,
}
impl Queue {
    const fn new() -> Self {
        Self {
            messages: [None; 2],
            head: 0,
            len: 0,
        }
    }
    fn push(&mut self, message: Message) -> Result<(), Error> {
        if self.len == 2 {
            return Err(Error::WouldBlock);
        }
        self.messages[(self.head + self.len) % 2] = Some(message);
        self.len += 1;
        Ok(())
    }
    fn peek(&self) -> Option<&Message> {
        self.messages[self.head].as_ref()
    }
    fn pop(&mut self) {
        assert!(self.len > 0);
        self.messages[self.head] = None;
        self.head = (self.head + 1) % 2;
        self.len -= 1;
    }
}

pub(super) struct Channel {
    pub(super) id: u64,
    open: [bool; 2],
    queues: [Queue; 2],
}
impl Channel {
    pub(super) const fn new(id: u64) -> Self {
        Self {
            id,
            open: [true; 2],
            queues: [Queue::new(), Queue::new()],
        }
    }
    pub(super) fn send(&mut self, side: usize, message: Message) -> Result<(), Error> {
        if !self.open[1 - side] {
            return Err(Error::Closed);
        }
        self.queues[1 - side].push(message)
    }
    pub(super) fn peek(&self, side: usize) -> Result<&Message, Error> {
        self.queues[side].peek().ok_or(if self.open[1 - side] {
            Error::WouldBlock
        } else {
            Error::Closed
        })
    }
    pub(super) fn pop(&mut self, side: usize) {
        self.queues[side].pop();
    }
    pub(super) fn close(&mut self, side: usize) -> bool {
        self.open[side] = false;
        self.queues[side] = Queue::new();
        !self.open[0] && !self.open[1]
    }
}
