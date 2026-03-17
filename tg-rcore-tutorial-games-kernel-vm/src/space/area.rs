use core::ops::Range;

use page_table::{VPN, VmMeta};

#[derive(Clone, Copy, PartialEq, Eq)]
/// 映射可见性。
pub enum MapVisibility {
    /// 私有映射，修改不会共享给子进程
    PRIVATE,
    /// 共享映射，修改会共享给子进程
    SHARED
}

pub struct MappedRange<Meta: VmMeta> {
    pub range: Range<VPN<Meta>>,
    pub visibility: MapVisibility,    
}

impl<Meta: VmMeta> MappedRange<Meta> {
    pub fn new(range: Range<VPN<Meta>>, visibility: MapVisibility) -> Self {
        Self {
            range,
            visibility,
        }
    }

    pub fn start(&self) -> VPN<Meta> {
        self.range.start
    }
    
    pub fn end(&self) -> VPN<Meta> {
        self.range.end
    }
}