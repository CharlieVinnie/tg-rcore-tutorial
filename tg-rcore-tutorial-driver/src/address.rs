pub const VIRT_PLIC: usize = 0xC00_0000;

pub const VIRTIO_START: usize = 0x1000_1000;
pub const VIRTIO_END: usize = 0x1000_9000;
pub const VIRTIO_STEP: usize = 0x1000;

pub fn visit_virtio_ranges<F: FnMut(usize, usize)>(mut visitor: F) {
    visitor(VIRTIO_START, VIRTIO_END);
    visitor(VIRT_PLIC, VIRT_PLIC + 0x100_0000);
}