use core::ops::Range;

use rangemap::RangeMap;

#[derive(Clone)]
pub struct UserAllocator {
    range: Range<usize>,
    inner: RangeMap<usize, ()>,
}

impl UserAllocator {
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            inner: RangeMap::new()
        }
    }

    pub fn alloc_aligned(&mut self, size: usize) -> Option<usize> {
        // Assume standard 4KB (4096 bytes) pages for Sv39
        // If your MmuMeta trait defines this, you can use Sv39::PAGE_SIZE instead.
        let align = 4096; 
        
        // 1. Align the requested size UP to the nearest page boundary.
        // If Doom asks for 4097 bytes, we must reserve 8192 bytes (2 pages) 
        // to prevent overlapping with future allocations.
        let aligned_size = (size + align - 1) & !(align - 1);

        // We use this to store our target coordinates before mutating the map
        let mut target_range = None;

        // 2. Search the gaps
        for gap in self.inner.gaps(&self.range) {
            // Align the START of the gap UP to the nearest page boundary.
            // (e.g., if the gap starts at 0x1005, we push it to 0x2000)
            let aligned_start = (gap.start + align - 1) & !(align - 1);
            
            // Calculate where this allocation would end
            let alloc_end = aligned_start.saturating_add(aligned_size);
            
            // 3. Does it fit in this gap?
            if alloc_end <= gap.end {
                target_range = Some(aligned_start..alloc_end);
                break;
            }
        }

        // 4. If we found a spot, insert it and return the starting address
        if let Some(r) = target_range {
            // We insert () because we stripped out the VmaFlags for this simplified struct
            self.inner.insert(r.clone(), ());
            Some(r.start)
        } else {
            None // Out of virtual memory in this range!
        }
    }
}