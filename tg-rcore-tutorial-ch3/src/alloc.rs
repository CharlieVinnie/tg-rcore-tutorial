use buddy_system_allocator::LockedHeap;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::empty();

const HEAP_SIZE: usize = 0x20000;
#[repr(align(4096))]
struct HeapSpace([u8; HEAP_SIZE]);
static mut HEAP_SPACE: HeapSpace = HeapSpace([0; HEAP_SIZE]);

pub fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP_SPACE.0) as usize, HEAP_SIZE);
    }
}