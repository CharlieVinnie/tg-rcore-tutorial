extern crate alloc;

use alloc::vec::Vec;
use core::any::Any;
use embedded_graphics::pixelcolor::Rgb888;
use spin::Mutex;
use tinybmp::Bmp;
use virtio_drivers::{Hal, MmioTransport, VirtIOGpu};

use crate::devices::Device;

pub trait GpuDevice: Device + Send + Sync + Any {
    fn get_framebuffer(&self) -> Result<&mut [u8], virtio_drivers::Error>;
    fn resolution(&self) -> Result<(u32, u32), virtio_drivers::Error>;
    fn flush(&self) -> Result<(), virtio_drivers::Error>;
}

pub struct VirtIOGpuWrapper<H: Hal> {
    gpu: Mutex<VirtIOGpu<'static, H, MmioTransport>>,
    fb: &'static [u8],
}

unsafe impl<H: Hal> Send for VirtIOGpuWrapper<H> {}
unsafe impl<H: Hal> Sync for VirtIOGpuWrapper<H> {}

static BMP_DATA: &[u8] = include_bytes!("mouse.bmp");

impl<H: Hal> VirtIOGpuWrapper<H> {
    pub fn new(transport: MmioTransport) -> Result<Self, virtio_drivers::Error> {
        let mut virtio = VirtIOGpu::new(transport)?;

        let fbuffer = virtio.setup_framebuffer()?;
        let len = fbuffer.len();
        let ptr = fbuffer.as_mut_ptr();
        // SAFETY: We own the VirtIOGpu device, and the framebuffer backing memory will live 
        // as long as the device lives, which is 'static.
        let fb = unsafe { core::slice::from_raw_parts_mut(ptr, len) };

        let bmp = Bmp::<Rgb888>::from_slice(BMP_DATA).expect("Failed to parse mouse.bmp");
        let raw = bmp.as_raw();
        let mut b = Vec::new();
        for i in raw.image_data().chunks(3) {
            let mut v = i.to_vec();
            b.append(&mut v);
            if i == [255, 255, 255] {
                b.push(0x0)
            } else {
                b.push(0xff)
            }
        }
        virtio.setup_cursor(b.as_slice(), 50, 50, 50, 50)?;

        Ok(Self {
            gpu: Mutex::new(virtio),
            fb,
        })
    }
}

impl<H: Hal + 'static> GpuDevice for VirtIOGpuWrapper<H> {
    fn flush(&self) -> Result<(), virtio_drivers::Error> {
        self.gpu.lock().flush()
    }

    fn resolution(&self) -> Result<(u32, u32), virtio_drivers::Error> {
        self.gpu.lock().resolution()
    }
    
    fn get_framebuffer(&self) -> Result<&mut [u8], virtio_drivers::Error> {
        // SAFETY: Mutating the framebuffer is safe because the framebuffer slice is not aliased
        // by the GPU device in a way that would cause data races on the CPU.
        unsafe {
            let ptr = self.fb.as_ptr() as *mut u8;
            Ok(core::slice::from_raw_parts_mut(ptr, self.fb.len()))
        }
    }
}

impl<H: Hal + 'static> Device for VirtIOGpuWrapper<H> {
    fn handle_irq(&self) {}
}