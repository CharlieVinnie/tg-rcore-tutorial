use spin::Once;

use tg_driver::{DeviceManager, Hal};

struct HalImpl;
impl Hal for HalImpl {
    fn dma_alloc(pages: usize) -> usize {
        #[repr(align(4096))]
        #[allow(dead_code)]
        struct DmaBuffer([u8; 1024 * 1024 * 2]);
        #[allow(dead_code)]
        static mut DMA_BUF: DmaBuffer = DmaBuffer([0; 1024 * 1024 * 2]);
        static mut OFFSET: usize = 0;
        unsafe {
            let base = core::ptr::addr_of_mut!(DMA_BUF) as usize;
            let paddr = base + OFFSET;
            OFFSET += pages * 4096;
            paddr
        }
    }
    
    fn dma_dealloc(_paddr: usize, _pages: usize) -> i32 { 0 }
    fn phys_to_virt(paddr: usize) -> usize { paddr }
    fn virt_to_phys(vaddr: usize) -> usize { vaddr }
}

pub static DEVICES: Once<DeviceManager> = Once::new();

pub fn init_devices() {
    DEVICES.call_once(DeviceManager::new::<HalImpl>);
}