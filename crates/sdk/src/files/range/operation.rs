// SPDX-License-Identifier: Apache-2.0
//! Transport-independent state for a pollable range read.
use super::collector::{Collector, VerifiedRange};
use rustic_abi::files::{Error, Packet, READ_OPEN, read::Request};

pub(super) const PENDING_MARKER_OP: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeProgress {
    Pending,
    Complete,
}

#[derive(Clone, Copy)]
enum Stage {
    Send(Packet),
    Receive(u8),
    Complete,
    Failed(Error),
}

pub(super) trait Transport {
    fn token(&self) -> u64;
    fn context(&self) -> u32;
    fn active_range_id(&self) -> Option<u64>;
    fn release_range(&mut self, id: u64);
    fn send(&mut self, packet: Packet) -> Result<(), Error>;
    fn receive(&mut self, op: u8, context: u32) -> Result<Option<Packet>, Error>;
}

pub(super) struct Operation {
    id: u64,
    token: u64,
    context: u32,
    collector: Option<Collector>,
    verified: Option<VerifiedRange>,
    stage: Stage,
}

impl Operation {
    pub(super) fn new(request: Request, token: u64, context: u32, id: u64) -> Result<Self, Error> {
        if token == 0 || id == 0 {
            return Err(Error::Unavailable);
        }
        let collector = Collector::new(request, context)?;
        let open = request.packet(READ_OPEN, context)?;
        Ok(Self {
            id,
            token,
            context,
            collector: Some(collector),
            verified: None,
            stage: Stage::Send(open),
        })
    }

    pub(super) fn poll<T: Transport>(&mut self, transport: &mut T) -> Result<RangeProgress, Error> {
        match self.stage {
            Stage::Complete => return Ok(RangeProgress::Complete),
            Stage::Failed(error) => return Err(error),
            _ => {}
        }
        if transport.token() != self.token || transport.context() != self.context {
            return self.fail(Error::Unavailable);
        }
        if transport.active_range_id() != Some(self.id) {
            return self.fail(Error::Unavailable);
        }
        match self.stage {
            Stage::Send(packet) => match transport.send(packet) {
                Ok(()) => {
                    self.stage = Stage::Receive(packet.op);
                    Ok(RangeProgress::Pending)
                }
                Err(Error::Busy) => Err(Error::Busy),
                Err(error) => self.fail(error),
            },
            Stage::Receive(op) => {
                let reply = match transport.receive(op, self.context) {
                    Ok(Some(reply)) => reply,
                    Ok(None) => return Ok(RangeProgress::Pending),
                    Err(error) => return self.fail(error),
                };
                let reply = match reply.checked_reply(op, self.context) {
                    Ok(reply) => reply,
                    Err(error) => return self.fail(error),
                };
                let Some(collector) = self.collector.as_mut() else {
                    return self.fail(Error::Protocol);
                };
                let result = if op == READ_OPEN {
                    collector.open(reply)
                } else {
                    collector.chunk(reply)
                };
                if let Err(error) = result {
                    return self.fail(error);
                }
                let next = match collector.next() {
                    Ok(next) => next,
                    Err(error) => return self.fail(error),
                };
                match next {
                    Some(packet) => {
                        self.stage = Stage::Send(packet);
                        Ok(RangeProgress::Pending)
                    }
                    None => {
                        let Some(collector) = self.collector.take() else {
                            return self.fail(Error::Protocol);
                        };
                        match collector.finish() {
                            Ok(verified) => {
                                self.verified = Some(verified);
                                self.stage = Stage::Complete;
                                Ok(RangeProgress::Complete)
                            }
                            Err(error) => self.fail(error),
                        }
                    }
                }
            }
            Stage::Complete => Ok(RangeProgress::Complete),
            Stage::Failed(error) => Err(error),
        }
    }

    pub(super) fn finish(mut self) -> Result<VerifiedRange, Error> {
        match self.stage {
            Stage::Complete => self.verified.take().ok_or(Error::Protocol),
            Stage::Failed(error) => Err(error),
            _ => Err(Error::Busy),
        }
    }

    fn fail<T>(&mut self, error: Error) -> Result<T, Error> {
        self.collector = None;
        self.verified = None;
        self.stage = Stage::Failed(error);
        Err(error)
    }

    #[cfg(test)]
    pub(super) fn observe_drop(&mut self, probe: &'static core::sync::atomic::AtomicBool) {
        if let Some(collector) = &mut self.collector {
            collector.observe_drop(probe);
        }
    }
}

pub(super) fn poll_operation<T: Transport>(
    operation: &mut Operation,
    transport: &mut T,
) -> Result<RangeProgress, Error> {
    match operation.stage {
        Stage::Complete => return Ok(RangeProgress::Complete),
        Stage::Failed(error) => return Err(error),
        _ => {}
    }
    if transport.token() != operation.token || transport.context() != operation.context {
        return operation.fail(Error::Unavailable);
    }
    if transport.active_range_id() != Some(operation.id) {
        return operation.fail(Error::Unavailable);
    }
    let result = operation.poll(transport);
    if matches!(operation.stage, Stage::Complete | Stage::Failed(_)) {
        transport.release_range(operation.id);
    }
    result
}

pub(super) fn pending_marker(context: u32, id: u64) -> Packet {
    let mut marker = Packet::new(PENDING_MARKER_OP);
    marker.context = context;
    marker.version = id;
    marker
}

pub(super) fn is_pending_marker(packet: Packet) -> bool {
    packet.op == PENDING_MARKER_OP
        && packet.status == 0
        && packet.count == 0
        && packet.id == 0
        && packet.arg == 0
        && packet.version != 0
        && packet.data.iter().all(|byte| *byte == 0)
}
