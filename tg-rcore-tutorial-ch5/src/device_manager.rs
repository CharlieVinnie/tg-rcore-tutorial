use core::{alloc::Layout, ptr::NonNull};

use alloc::alloc::{alloc_zeroed, dealloc};
use spin::Once;

use tg_driver::{DeviceManager, Hal};
use tg_kernel_vm::page_table::{MmuMeta, Sv39, VAddr, VmFlags};

use crate::{build_flags, kernel_space::KERNEL_SPACE};

struct VirtioHal;

impl Hal for VirtioHal {
    /// 分配 DMA 内存
    fn dma_alloc(pages: usize) -> usize {
        unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                pages << Sv39::PAGE_BITS,
                1 << Sv39::PAGE_BITS,
            )) as _
        }
    }

    /// 释放 DMA 内存
    fn dma_dealloc(paddr: usize, pages: usize) -> i32 {
        unsafe {
            dealloc(
                paddr as _,
                Layout::from_size_align_unchecked(pages << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS),
            )
        }
        0
    }

    /// 物理地址转虚拟地址（恒等映射）
    fn phys_to_virt(paddr: usize) -> usize {
        paddr
    }

    /// 虚拟地址转物理地址
    fn virt_to_phys(vaddr: usize) -> usize {
        const VALID: VmFlags<Sv39> = build_flags("__V");
        let ptr: NonNull<u8> = KERNEL_SPACE
            .get()
            .translate(VAddr::new(vaddr), VALID)
            .unwrap();
        ptr.as_ptr() as usize
    }
}

pub static DEVICES: Once<DeviceManager> = Once::new();

pub fn init_devices() {
    DEVICES.call_once(DeviceManager::new::<VirtioHal>);
}
