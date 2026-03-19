use tg_kernel_vm::page_table::{Sv, VAddr, VmFlags};
use tg_kernel_vm::{AddressSpace, PageManager};

pub fn read_list<T, const N: usize, M: PageManager<Sv<N>>>(
    address_space: &AddressSpace<Sv<N>, M>,
    path: usize,
    count: usize,
) -> Result<&[T], ()> {
    let readable: VmFlags<Sv<N>> = VmFlags::build_from_str("RV");
    if let Some(ptr) = address_space.translate::<T>(VAddr::new(path), readable) {
        unsafe { Ok(core::slice::from_raw_parts(ptr.as_ptr(), count)) }
    } else {
        Err(())
    }
}
