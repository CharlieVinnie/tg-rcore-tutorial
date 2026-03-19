extern crate alloc;

use core::any::Any;
use spin::Mutex;
use virtio_drivers::{Hal, MmioTransport, VirtIOBlk};

use crate::devices::Device;

pub trait BlockDevice: Device + Send + Sync + Any {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    fn write_block(&self, block_id: usize, buf: &[u8]);
}

pub struct VirtIOBlockWrapper<H: Hal> {
    block: Mutex<VirtIOBlk<H, MmioTransport>>,
}

unsafe impl<H: Hal> Send for VirtIOBlockWrapper<H> {}
unsafe impl<H: Hal> Sync for VirtIOBlockWrapper<H> {}

impl<H: Hal> VirtIOBlockWrapper<H> {
    pub fn new(transport: MmioTransport) -> Result<Self, virtio_drivers::Error> {
        let virtio = VirtIOBlk::new(transport)?;
        Ok(Self {
            block: Mutex::new(virtio),
        })
    }
}

impl<H: Hal + 'static> BlockDevice for VirtIOBlockWrapper<H> {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        self.block.lock().read_block(block_id, buf).expect("Error when reading VirtIOBlk");
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        self.block.lock().write_block(block_id, buf).expect("Error when writing VirtIOBlk");
    }
}

impl<H: Hal + 'static> Device for VirtIOBlockWrapper<H> {
    fn handle_irq(&self) {}
}
