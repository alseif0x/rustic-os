// SPDX-License-Identifier: Apache-2.0
use crate::arch::{
    memory::{DmaRegion, Memory},
    pci::BlockTransport,
};
use rustic_kernel::block::{Error, Geometry, queue::Layout};
#[must_use = "call shutdown to confirm reset and release DMA ownership"]
pub(crate) struct Device {
    pub(super) transport: BlockTransport,
    pub(super) dma: DmaRegion,
    pub(super) layout: Layout,
    pub(super) geometry: Geometry,
    pub(super) index: u16,
    pub(super) failed: bool,
}
impl Device {
    pub(crate) fn geometry(&self) -> Geometry {
        self.geometry
    }
    pub(crate) fn open(memory: &mut Memory) -> Result<Self, Error> {
        let mut transport = BlockTransport::claim()?;
        transport.reset()?;
        let setup = (|| {
            transport.write8(18, 1);
            transport.write8(18, 3);
            let features = transport.read32(0);
            if features & (1 << 9) == 0 {
                return Err(Error::Unsupported);
            }
            transport.write32(4, features & ((1 << 9) | (1 << 5)));
            transport.write16(14, 0);
            let layout = Layout::new(usize::from(transport.read16(12)))?;
            if transport.read32(8) != 0 {
                return Err(Error::Busy);
            }
            let capacity =
                |t: &BlockTransport| u64::from(t.read32(20)) | (u64::from(t.read32(24)) << 32);
            let mut sectors = None;
            for _ in 0..8 {
                let first = capacity(&transport);
                if first != 0 && first == capacity(&transport) {
                    sectors = Some(first);
                    break;
                }
            }
            let geometry = Geometry {
                sectors: sectors.ok_or(Error::Protocol)?,
                read_only: features & (1 << 5) != 0,
            };
            let dma = memory
                .allocate_dma(layout.pages + 1)
                .map_err(|_| Error::Memory)?;
            Ok((layout, geometry, dma))
        })();
        let (layout, geometry, mut dma) = match setup {
            Ok(value) => value,
            Err(error) => {
                if transport.reset().is_ok() {
                    transport.release();
                }
                return Err(error);
            }
        };
        dma.write(layout.available, 2, 1); // Suppress INTx; completion is polled.
        transport.write32(8, (dma.physical(0) / 4096) as u32);
        transport.write8(18, 7);
        let failed = transport.read8(18) != 7;
        let device = Self {
            transport,
            dma,
            layout,
            geometry,
            index: 0,
            failed,
        };
        if failed {
            device.shutdown(memory)?;
            return Err(Error::Protocol);
        }
        Ok(device)
    }
    /// Consumes ownership. Failed reset quarantines allocated frames and the PCI claim.
    pub(crate) fn shutdown(mut self, memory: &mut Memory) -> Result<(), Error> {
        self.transport.reset()?;
        // SAFETY: Status read back as zero: VirtIO guarantees no more queue/DMA
        // interaction until reinitialized. No CPU buffer references escape.
        unsafe {
            memory.release_dma(self.dma);
        }
        self.transport.release();
        Ok(())
    }
}
