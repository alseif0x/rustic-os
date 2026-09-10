// SPDX-License-Identifier: Apache-2.0
use super::super::{block::Service, record::Pending};
use super::{Action, Process};
use crate::arch::memory::Memory;
use rustic_abi::block::*;

pub(super) fn dispatch(
    number: u64,
    process: &mut Process,
    service: &mut Service,
    owner: u64,
    memory: &mut Memory,
) -> Action {
    let [handle, address, length] = process.frame.arguments();
    if number == INFO && [handle, address, length] == [0; 3] {
        return Action::Return(u64::from(VERSION));
    }
    if number == WAIT {
        if length != 0 {
            return Action::Return(Error::Request.code());
        }
        return match service.broker.wait(owner, handle, address) {
            Ok(()) => Action::Return(0),
            Err(Error::WouldBlock) => Action::Block(Pending::Block {
                handle,
                id: address,
            }),
            Err(error) => Action::Return(error.code()),
        };
    }
    let result = (|| match number {
        CLOSE => {
            if address != 0 || length != 0 {
                return Err(Error::Request);
            }
            service.broker.close(owner, handle)?;
            Ok(0)
        }
        CANCEL => {
            if length != 0 {
                return Err(Error::Request);
            }
            service.broker.cancel(owner, handle, address)
        }
        RESULT => {
            let result = service.broker.peek(owner, handle)?;
            if length != RESULT_BYTES as u64 {
                return Err(Error::Size);
            }
            memory
                .copy_to_user(&process.space, address, &result.encode())
                .map_err(|_| Error::Address)?;
            service.broker.consume(owner, handle)?;
            Ok(RESULT_BYTES as u64)
        }
        INFO => {
            let geometry = service
                .broker
                .geometry(owner, handle, service.geometry()?)?;
            if length != GEOMETRY_BYTES as u64 {
                return Err(Error::Size);
            }
            memory
                .copy_to_user(&process.space, address, &geometry.encode())
                .map_err(|_| Error::Address)?;
            Ok(GEOMETRY_BYTES as u64)
        }
        SUBMIT => {
            let geometry = service.geometry()?;
            service.broker.geometry(owner, handle, geometry)?;
            if length != REQUEST_BYTES as u64 {
                return Err(Error::Size);
            }
            let mut bytes = [0; REQUEST_BYTES];
            memory
                .copy_from_user(&process.space, address, &mut bytes)
                .map_err(|_| Error::Address)?;
            let request = Request::decode(&bytes)?;
            service
                .broker
                .check(owner, handle, request.operation, request.sector, geometry)?;
            let mut data = [0; SECTOR];
            if request.operation == Operation::Write {
                memory
                    .copy_from_user(&process.space, request.address, &mut data)
                    .map_err(|_| Error::Address)?;
            }
            service.broker.admit(
                owner,
                handle,
                request.operation,
                request.sector,
                data,
                geometry,
            )
        }
        _ => Err(Error::Request),
    })();
    Action::Return(result.unwrap_or_else(Error::code))
}
