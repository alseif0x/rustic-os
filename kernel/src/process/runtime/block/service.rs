// SPDX-License-Identifier: Apache-2.0
use crate::{arch::memory::Memory, drivers::block::Device};
use rustic_abi::block::{Error, Operation, SECTOR, Status};
use rustic_kernel::block::{Error as DeviceError, Geometry, access::Broker};

pub(in super::super) struct Service {
    pub(in super::super) broker: Broker,
    device: Option<Device>,
    geometry: Geometry,
    #[cfg(feature = "sdk-test")]
    pub(in super::super) hold_next: bool,
    #[cfg(feature = "sdk-test")]
    pub(in super::super) reject_next: bool,
}
impl Service {
    pub(in super::super) fn new() -> Self {
        Self {
            broker: Broker::new(),
            device: None,
            geometry: Geometry {
                sectors: 0,
                read_only: false,
            },
            #[cfg(feature = "sdk-test")]
            hold_next: false,
            #[cfg(feature = "sdk-test")]
            reject_next: false,
        }
    }
    #[cfg(feature = "sdk-test")]
    pub(in super::super) fn open(&mut self, memory: &mut Memory) -> Result<(), DeviceError> {
        if self.device.is_some() || self.broker.counts() != (0, 0) {
            return Err(DeviceError::Busy);
        }
        let device = Device::open(memory)?;
        self.geometry = device.geometry();
        self.device = Some(device);
        Ok(())
    }
    pub(in super::super) fn geometry(&self) -> Result<Geometry, Error> {
        if self.device.is_none() {
            return Err(Error::Unavailable);
        }
        Ok(self.geometry)
    }
    pub(in super::super) fn tick(&mut self, memory: &mut Memory) {
        if let Some(id) = self.broker.active() {
            let mut data = [0; SECTOR];
            if let Some(result) = self
                .device
                .as_mut()
                .expect("active request owns device")
                .poll(&mut data)
            {
                let status = match result {
                    Ok(()) => Status::Success,
                    Err(DeviceError::Timeout) => Status::Timeout,
                    Err(DeviceError::Protocol) => Status::Protocol,
                    Err(_) => Status::Io,
                };
                self.broker.finish(id, status, data);
                if matches!(result, Err(DeviceError::Timeout | DeviceError::Protocol)) {
                    self.recover(memory);
                }
            }
        }
        if let Some(request) = self.broker.start() {
            let Some(device) = self.device.as_mut() else {
                self.broker
                    .finish(request.id, Status::Unavailable, [0; SECTOR]);
                return;
            };
            let kind = match request.operation {
                Operation::Read => 0,
                Operation::Write => 1,
                Operation::Flush => 4,
            };
            let notify = true;
            #[cfg(feature = "sdk-test")]
            let (kind, notify) = (
                if core::mem::take(&mut self.reject_next) {
                    0xffff
                } else {
                    kind
                },
                !core::mem::take(&mut self.hold_next) && notify,
            );
            let length = if request.operation == Operation::Flush {
                0
            } else {
                SECTOR
            };
            if let Err(error) = device.start(kind, request.sector, &request.data[..length], notify)
            {
                self.broker
                    .finish(request.id, Status::Unavailable, [0; SECTOR]);
                if error == DeviceError::Protocol {
                    self.recover(memory);
                }
            }
        }
    }
    fn recover(&mut self, memory: &mut Memory) {
        // A reset failure deliberately retains DMA frames/PCI claim. Pending grants
        // cannot start more requests while unavailable; completions remain readable.
        if self.device.take().unwrap().shutdown(memory).is_err() {
            return;
        }
        if let Ok(device) = Device::open(memory) {
            if device.geometry() == self.geometry {
                self.device = Some(device);
            } else {
                let _ = device.shutdown(memory);
            }
        }
    }
    #[cfg(feature = "sdk-test")]
    pub(in super::super) fn shutdown(&mut self, memory: &mut Memory) -> Result<(), DeviceError> {
        if self.broker.counts() != (0, 0) {
            return Err(DeviceError::Busy);
        }
        if let Some(device) = self.device.take() {
            device.shutdown(memory)?;
        }
        Ok(())
    }
}
