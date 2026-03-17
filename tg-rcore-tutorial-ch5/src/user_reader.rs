use tg_kernel_vm::page_table::{Sv, VAddr, VmFlags};
use tg_kernel_vm::{AddressSpace, PageManager};

pub trait Int {
    fn non_zero(&self) -> bool;
}

impl Int for u8 {
    fn non_zero(&self) -> bool {
        *self != 0
    }
}
impl Int for u16 {
    fn non_zero(&self) -> bool {
        *self != 0
    }
}
impl Int for u32 {
    fn non_zero(&self) -> bool {
        *self != 0
    }
}
impl Int for u64 {
    fn non_zero(&self) -> bool {
        *self != 0
    }
}
impl Int for usize {
    fn non_zero(&self) -> bool {
        *self != 0
    }
}

pub fn read_zero_ended_list<T: Int, const N: usize, M: PageManager<Sv<N>>>(
    address_space: &AddressSpace<Sv<N>, M>,
    path: usize,
) -> Result<&[T], ()> {
    let readable: VmFlags<Sv<N>> = VmFlags::build_from_str("RV");
    if let Some(ptr) = address_space.translate::<T>(VAddr::new(path), readable) {
        let mut count = 0;
        while unsafe { ptr.add(count).as_ref().non_zero() } {
            count += 1;
        }
        unsafe { Ok(core::slice::from_raw_parts(ptr.as_ptr(), count)) }
    } else {
        Err(())
    }
}
