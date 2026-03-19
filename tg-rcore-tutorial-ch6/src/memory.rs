use core::{alloc::Layout, ptr::NonNull};

use alloc::alloc::alloc_zeroed;
use spin::Once;
use tg_kernel_vm::{
    AddressSpace, MapVisibility, PageManager, page_table::{MmuMeta, PPN, Pte, Sv39, VAddr, VPN, VmFlags}
};

pub struct KernelSpace {
    inner: Once<AddressSpace<Sv39, Sv39Manager>>,
}

impl KernelSpace {
    pub fn init(&self, ks: AddressSpace<Sv39, Sv39Manager>) {
        self.inner.call_once(|| ks);
    }
    pub fn get(&self) -> &AddressSpace<Sv39, Sv39Manager> {
        self.inner.get().unwrap()
    }
}

unsafe impl Sync for KernelSpace {}
unsafe impl Send for KernelSpace {}

pub static KERNEL_SPACE: KernelSpace = KernelSpace { inner: Once::new() };

/// Sv39 页表管理器：负责物理页的分配和映射。
#[repr(transparent)]
pub struct Sv39Manager(NonNull<Pte<Sv39>>);

impl Sv39Manager {
    /// 自定义标志位：标记该页面由内核分配（用于 deallocate 时判断）
    const OWNED: VmFlags<Sv39> = unsafe { VmFlags::from_raw(1 << 8) };

    /// 分配物理页面并清零
    #[inline]
    fn page_alloc<T>(count: usize) -> *mut T {
        unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                count << Sv39::PAGE_BITS,
                1 << Sv39::PAGE_BITS,
            ))
        }
        .cast()
    }
}

/// 实现 PageManager trait：为地址空间提供页表操作能力
impl PageManager<Sv39> for Sv39Manager {
    /// 创建新的根页表（分配一个物理页）
    #[inline]
    fn new_root() -> Self {
        Self(NonNull::new(Self::page_alloc(1)).unwrap())
    }

    /// 获取根页表的物理页号
    #[inline]
    fn root_ppn(&self) -> PPN<Sv39> {
        PPN::new(self.0.as_ptr() as usize >> Sv39::PAGE_BITS)
    }

    /// 获取根页表的指针
    #[inline]
    fn root_ptr(&self) -> NonNull<Pte<Sv39>> {
        self.0
    }

    /// 物理页号 → 虚拟地址指针（恒等映射下 PPN == VPN）
    #[inline]
    fn p_to_v<T>(&self, ppn: PPN<Sv39>) -> NonNull<T> {
        unsafe { NonNull::new_unchecked(VPN::<Sv39>::new(ppn.val()).base().as_mut_ptr()) }
    }

    /// 虚拟地址指针 → 物理页号
    #[inline]
    fn v_to_p<T>(&self, ptr: NonNull<T>) -> PPN<Sv39> {
        PPN::new(VAddr::<Sv39>::new(ptr.as_ptr() as _).floor().val())
    }

    /// 检查页表项是否由内核分配
    #[inline]
    fn check_owned(&self, pte: Pte<Sv39>) -> bool {
        pte.flags().contains(Self::OWNED)
    }

    /// 分配物理页面：清零并标记为内核拥有
    #[inline]
    fn allocate(&mut self, len: usize, flags: &mut VmFlags<Sv39>) -> NonNull<u8> {
        *flags |= Self::OWNED;
        NonNull::new(Self::page_alloc(len)).unwrap()
    }

    fn deallocate(&mut self, _pte: Pte<Sv39>, _len: usize) -> usize {
        todo!()
    }

    fn drop_root(&mut self) {
        todo!()
    }
}

pub trait VmMapper {
    #[allow(unused)]
    fn empty(&self) -> bool; 

    fn map_to(&mut self, paddr: usize) -> Result<(), ()>;

    // Returns: The Physical Address of the allocated frame.
    #[allow(unused)]
    fn map_a_frame(&mut self) -> Result<usize, ()>;
}

pub struct VmMapperSv39<'a> {
    start: VPN<Sv39>,
    end: VPN<Sv39>,
    flags: VmFlags<Sv39>,
    visibility: MapVisibility,
    address_space: &'a mut AddressSpace<Sv39, Sv39Manager>,
}

impl<'a> VmMapperSv39<'a> {
    pub fn new(start: VPN<Sv39>, end: VPN<Sv39>, flags: VmFlags<Sv39>, visibility: MapVisibility, address_space: &'a mut AddressSpace<Sv39, Sv39Manager>) -> Self {
        Self { start, end, flags, visibility, address_space }
    }
}

impl<'a> VmMapper for VmMapperSv39<'a> {
    fn empty(&self) -> bool {
        self.start == self.end
    }

    fn map_to(&mut self, paddr: usize) -> Result<(), ()> {
        self.address_space.map_extern(self.start..self.end, PPN::new(paddr >> Sv39::PAGE_BITS), self.flags, self.visibility);
        Ok(())
    }

    fn map_a_frame(&mut self) -> Result<usize, ()> {
        todo!()
        // assert!(self.start != self.end);
        // let paddr = self.address_space.map(self.start..self.start+1, &[], 0, self.flags);
        // self.start += 1;
        // paddr
    }
}