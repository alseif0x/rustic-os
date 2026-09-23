// SPDX-License-Identifier: Apache-2.0
//! One supervisor-owned native image transaction, isolated from file policy.
use super::super::{Error as ProcessError, application, manager::Manager};
use crate::arch::memory::{KernelBuffer, Memory};
use rustic_abi::{application as manifest_abi, runtime};
use rustic_kernel::process::lifecycle::Pid;

struct ImageStage {
    owner: u64,
    generation: u64,
    length: usize,
    received: usize,
    available: u64,
    manifest: [u8; manifest_abi::SIZE],
    buffer: KernelBuffer,
}

#[derive(Default)]
pub(in super::super) struct State {
    generation: u64,
    active: Option<ImageStage>,
}

impl Manager {
    pub(in super::super) fn preserve_stage_on_refusal(&self, words: [u64; 8]) -> bool {
        match words[0] {
            runtime::STAGE_BEGIN => self.image_stage.active.is_some(),
            runtime::STAGE_COPY | runtime::STAGE_COMMIT | runtime::STAGE_ABORT => self
                .image_stage
                .active
                .as_ref()
                .is_some_and(|stage| stage.generation != words[1]),
            _ => false,
        }
    }

    pub(in super::super) fn clear_image_stage(&mut self, memory: &mut Memory) {
        if let Some(stage) = self.image_stage.active.take() {
            release_stage(stage, memory);
        }
    }

    pub(in super::super) fn staging_operation(
        &mut self,
        caller: u64,
        words: [u64; 8],
        memory: &mut Memory,
    ) -> Result<[u64; 8], runtime::Error> {
        if caller == 0 || caller != self.session.supervisor {
            return Err(runtime::Error::Denied);
        }
        match words[0] {
            runtime::STAGE_BEGIN => self.stage_begin(caller, words, memory),
            runtime::STAGE_COPY => self.stage_copy(caller, words, memory),
            runtime::STAGE_COMMIT => self.stage_commit(caller, words[1], memory),
            runtime::STAGE_ABORT => self.stage_abort(caller, words[1], memory),
            _ => Err(runtime::Error::Invalid),
        }
    }

    fn stage_begin(
        &mut self,
        caller: u64,
        words: [u64; 8],
        memory: &mut Memory,
    ) -> Result<[u64; 8], runtime::Error> {
        if self.image_stage.active.is_some() {
            return Err(runtime::Error::Busy);
        }
        let length = usize::try_from(words[1]).map_err(|_| runtime::Error::Size)?;
        if !(64..=runtime::MAX_STAGED_IMAGE_BYTES).contains(&length) {
            return Err(runtime::Error::Size);
        }
        let available = words[3];
        if available & !manifest_abi::KNOWN != 0 {
            return Err(runtime::Error::Invalid);
        }
        let mut manifest = [0; manifest_abi::SIZE];
        let copied = self.process(Pid(caller)).is_ok_and(|source| {
            memory
                .copy_from_user(&source.space, words[2], &mut manifest)
                .is_ok()
        });
        if !copied {
            return Err(runtime::Error::Address);
        }
        let parsed =
            manifest_abi::Manifest::parse(&manifest).map_err(|_| runtime::Error::Protocol)?;
        if !parsed.admitted(available) {
            return Err(runtime::Error::Denied);
        }
        let generation = self
            .image_stage
            .generation
            .checked_add(1)
            .filter(|next| *next != 0)
            .ok_or(runtime::Error::Full)?;
        let buffer = memory
            .allocate_kernel_buffer(length)
            .map_err(|_| runtime::Error::Full)?;
        self.image_stage.generation = generation;
        self.image_stage.active = Some(ImageStage {
            owner: caller,
            generation,
            length,
            received: 0,
            available,
            manifest,
            buffer,
        });
        let mut result = [0; 8];
        result[0] = generation;
        result[1] = runtime::STAGE_CHUNK_BYTES as u64;
        Ok(result)
    }

    fn stage_copy(
        &mut self,
        caller: u64,
        words: [u64; 8],
        memory: &mut Memory,
    ) -> Result<[u64; 8], runtime::Error> {
        let Some(stage) = self.image_stage.active.as_ref() else {
            return Err(runtime::Error::Busy);
        };
        if stage.owner != caller || stage.generation != words[1] {
            return Err(runtime::Error::Invalid);
        }
        let (received, total) = (stage.received, stage.length);
        let offset = match usize::try_from(words[2]) {
            Ok(offset) => offset,
            Err(_) => {
                self.clear_image_stage(memory);
                return Err(runtime::Error::Invalid);
            }
        };
        let length = match usize::try_from(words[4]) {
            Ok(length) => length,
            Err(_) => {
                self.clear_image_stage(memory);
                return Err(runtime::Error::Size);
            }
        };
        if offset != received {
            self.clear_image_stage(memory);
            return Err(runtime::Error::Invalid);
        }
        if length == 0 || length > runtime::STAGE_CHUNK_BYTES {
            self.clear_image_stage(memory);
            return Err(runtime::Error::Size);
        }
        if offset.checked_add(length).is_none_or(|end| end > total) {
            self.clear_image_stage(memory);
            return Err(runtime::Error::Size);
        }

        let mut chunk = [0; runtime::STAGE_CHUNK_BYTES];
        let copy = {
            self.process(Pid(caller)).is_ok_and(|source| {
                memory
                    .copy_from_user(&source.space, words[3], &mut chunk[..length])
                    .is_ok()
            })
        };
        if !copy {
            self.clear_image_stage(memory);
            return Err(runtime::Error::Address);
        }
        let stage = self
            .image_stage
            .active
            .as_mut()
            .expect("checked active stage");
        if stage.buffer.write(offset, &chunk[..length]).is_err() {
            self.clear_image_stage(memory);
            return Err(runtime::Error::Protocol);
        }
        stage.received += length;
        let mut result = [0; 8];
        result[0] = stage.received as u64;
        Ok(result)
    }

    fn stage_commit(
        &mut self,
        caller: u64,
        generation: u64,
        memory: &mut Memory,
    ) -> Result<[u64; 8], runtime::Error> {
        let Some(active) = self.image_stage.active.as_ref() else {
            return Err(runtime::Error::Busy);
        };
        if active.owner != caller || active.generation != generation {
            return Err(runtime::Error::Invalid);
        }
        let stage = self
            .image_stage
            .active
            .take()
            .expect("checked active image");
        if stage.received != stage.length {
            release_stage(stage, memory);
            return Err(runtime::Error::Size);
        }
        let result = (|| {
            let parsed = manifest_abi::Manifest::parse(&stage.manifest)
                .map_err(|_| runtime::Error::Protocol)?;
            if !parsed.admitted(stage.available) {
                return Err(runtime::Error::Denied);
            }
            application::launch_dormant(
                self,
                memory,
                &stage.manifest,
                parsed.executable,
                stage.buffer.as_slice(),
                stage.available,
                application::Registration {
                    parent: caller,
                    program: runtime::DYNAMIC_IMAGE,
                },
            )
            .map_err(map_launch_error)
        })();
        release_stage(stage, memory);
        let pid = result?;
        let mut result = [0; 8];
        result[0] = pid.0;
        result[1] = generation;
        Ok(result)
    }

    fn stage_abort(
        &mut self,
        caller: u64,
        generation: u64,
        memory: &mut Memory,
    ) -> Result<[u64; 8], runtime::Error> {
        let Some(stage) = self.image_stage.active.as_ref() else {
            return Err(runtime::Error::Busy);
        };
        if stage.owner != caller || stage.generation != generation {
            return Err(runtime::Error::Invalid);
        }
        self.clear_image_stage(memory);
        Ok([0; 8])
    }
}

fn map_launch_error(error: application::Error) -> runtime::Error {
    match error {
        application::Error::Manifest(_)
        | application::Error::Executable
        | application::Error::ArtifactDigest => runtime::Error::Protocol,
        application::Error::Denied => runtime::Error::Denied,
        application::Error::Process(ProcessError::Elf(_)) => runtime::Error::Protocol,
        application::Error::Process(ProcessError::Memory(_) | ProcessError::Process(_)) => {
            runtime::Error::Full
        }
    }
}

fn release_stage(mut stage: ImageStage, memory: &mut Memory) {
    stage.manifest.fill(0);
    memory.release_kernel_buffer(stage.buffer);
}
