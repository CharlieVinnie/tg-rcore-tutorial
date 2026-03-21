//! VirtIO 块设备驱动模块
//!
//! 本模块实现了 VirtIO 块设备驱动，连接 QEMU 的虚拟块设备与 easy-fs 文件系统。
//!
//! ## 架构
//!
//! ```text
//! easy-fs 文件系统
//!       │
//!       ▼
//! BlockDevice trait（read_block / write_block）
//!       │
//!       ▼
//! VirtIOBlock（本模块实现）
//!       │
//!       ▼
//! virtio-drivers 库（VirtIOBlk）
//!       │
//!       ▼
//! QEMU VirtIO MMIO 设备（0x10001000）
//!       │
//!       ▼
//! fs.img 磁盘镜像文件
//! ```
//!
//! ## VirtioHal
//!
//! `virtio-drivers` 库需要一个 `Hal` 实现来处理 DMA 内存分配和地址转换。
//! 由于内核使用恒等映射，物理地址 == 虚拟地址，因此转换非常简单。
//!
//! 教程阅读建议：
//!
//! - 先看 `BLOCK_DEVICE`：理解驱动实例如何被文件系统全局复用；
//! - 再看 `BlockDevice` trait 实现：理解文件系统读写如何下沉到块设备；
//! - 最后看 `VirtioHal`：理解 DMA 分配与地址转换为何能“近似直通”。

use alloc::sync::Arc;
use spin::Lazy;
use tg_easy_fs::BlockDevice;

/// 全局块设备实例（延迟初始化）
///
/// 从 DeviceManager 中获取由 driver crate 发现的 BlockDevice。
pub static BLOCK_DEVICE: Lazy<Arc<dyn BlockDevice>> = Lazy::new(|| {
    let block = crate::device::DEVICES
        .get()
        .expect("DEVICES must be initialized")
        .get_block()
        .expect("Block device not found");
    Arc::new(TgBlockDevice(block))
});

/// easy-fs BlockDevice 接口到 tg_driver::BlockDevice 的适配层
struct TgBlockDevice(Arc<dyn tg_driver::BlockDevice>);

impl BlockDevice for TgBlockDevice {
    /// 读取一个磁盘块（512 字节）
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        self.0.read_block(block_id, buf);
    }
    /// 写入一个磁盘块（512 字节）
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        self.0.write_block(block_id, buf);
    }
}


