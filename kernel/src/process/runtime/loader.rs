// SPDX-License-Identifier: Apache-2.0
use super::Error;
use crate::arch::memory::{Memory, UserSpace};
use rustic_kernel::{
    memory::{PAGE_SIZE, PagePermissions},
    process::elf::{Image, STACK_PAGES, STACK_TOP},
};

pub(super) struct Loaded {
    pub(super) space: UserSpace,
    pub(super) entry: u64,
}

pub(super) fn load(memory: &mut Memory, bytes: &[u8]) -> Result<Loaded, Error> {
    // Validate every segment before acquiring a root or frame.
    let image = Image::parse(bytes).map_err(Error::Elf)?;
    let mut space = memory.create_user().map_err(Error::Memory)?;
    let result = (|| {
        for segment in image.segments() {
            for page in (segment.start_page()..segment.end_page()).step_by(PAGE_SIZE as usize) {
                let start = page.max(segment.address);
                let end = (page + PAGE_SIZE).min(segment.address + segment.file_size as u64);
                let (offset, data) = if start < end {
                    (
                        (start - page) as usize,
                        &image.data(segment)
                            [(start - segment.address) as usize..(end - segment.address) as usize],
                    )
                } else {
                    (0, &[][..])
                };
                memory.load_page(
                    &mut space,
                    page,
                    PagePermissions {
                        writable: segment.writable,
                        executable: segment.executable,
                        user: true,
                    },
                    offset,
                    data,
                )?;
            }
        }
        for page in (STACK_TOP - STACK_PAGES * PAGE_SIZE..STACK_TOP).step_by(PAGE_SIZE as usize) {
            memory.load_page(
                &mut space,
                page,
                PagePermissions {
                    user: true,
                    ..PagePermissions::READ_WRITE
                },
                0,
                &[],
            )?;
        }
        Ok::<(), crate::arch::memory::Error>(())
    })();
    if let Err(error) = result {
        memory
            .destroy_user(&mut space)
            .expect("rollback owned inactive image");
        return Err(Error::Memory(error));
    }
    Ok(Loaded {
        space,
        entry: image.entry(),
    })
}
