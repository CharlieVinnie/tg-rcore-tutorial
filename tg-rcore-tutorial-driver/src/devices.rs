use alloc::sync::Arc;
use virtio_drivers::{Hal, VirtIOHeader};

use crate::{GpuDevice, VirtIOGpuWrapper};

pub struct DeviceManager {
    gpu: Option<Arc<dyn GpuDevice>>,
}

impl DeviceManager {
    pub fn new<H: Hal + 'static>() -> Self {
        let mut gpu: Option<Arc<dyn GpuDevice>> = None;

        for addr in (0x1000_1000..=0x1000_8000).step_by(0x1000) {
            let header = unsafe { &mut *(addr as *mut VirtIOHeader) };
            
            if let Ok(g) = VirtIOGpuWrapper::<H>::new(header) {
                gpu = Some(Arc::new(g));
                break;
            }
        }

        Self {
            gpu
        }
    }

    pub fn get_gpu(&self) -> Option<Arc<dyn GpuDevice>> {
        self.gpu.clone()
    }
}