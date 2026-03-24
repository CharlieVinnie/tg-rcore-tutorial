//! 内存分配。
//!
//! 教程阅读建议：
//!
//! - 先看 `init` 与 `transfer`：理解“先初始化，再把可用内存交给分配器”；
//! - 再看 `HEAP` / `GlobalAlloc`：理解 Rust `alloc` 如何落到内核堆实现。

#![no_std]
#![deny(missing_docs)]

extern crate alloc;

use alloc::alloc::handle_alloc_error;
use core::{
    alloc::{GlobalAlloc, Layout}, num::NonZero, ptr::NonNull
};
use customizable_buddy::{BuddyAllocator, LinkedListBuddy, UsizeBuddy};

// 假设 SpinNoIrq 和 SpinNoIrqGuard 定义在 crate::sync 或者当前文件中
use tg_sync::SpinNoIrq; 

/// 初始化内存分配。
///
/// 参数 `base_address` 表示动态内存区域的起始位置。
#[inline]
pub fn init(base_address: usize) {
    // 获取关中断自旋锁后初始化 buddy 分配器
    HEAP.lock().init(
        core::mem::size_of::<usize>().trailing_zeros() as _,
        NonNull::new(base_address as *mut u8).unwrap(),
    );
}

/// 将一个内存块托管到内存分配器。
#[inline]
pub unsafe fn transfer(region: &'static mut [u8]) {
    let ptr = NonNull::new(region.as_mut_ptr()).unwrap();
    // 获取关中断自旋锁后转移内存
    unsafe { HEAP.lock().transfer(ptr, region.len()); }
}

/// 堆分配器。
///
/// 使用 SpinNoIrq 包装，确保在 SMP 环境下并发安全，
/// 同时避免内核中断处理程序带来的死锁问题。
static HEAP: SpinNoIrq<BuddyAllocator<21, UsizeBuddy, LinkedListBuddy>> =
    SpinNoIrq::new(BuddyAllocator::new());

struct Global;

#[global_allocator]
static GLOBAL: Global = Global;

// SAFETY: GlobalAlloc 的实现必须是 unsafe 的。
// 借助 SpinNoIrq，此实现现在是 SMP 和中断安全的。
unsafe impl GlobalAlloc for Global {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // 自动关中断并加锁，离开作用域自动解锁并恢复中断状态
        if let Ok((ptr, _)) = HEAP.lock().allocate::<u8>(layout.align(), NonZero::new(layout.size()).unwrap()) {
            ptr.as_ptr()
        } else {
            handle_alloc_error(layout)
        }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let layout = layout.pad_to_align();
        // 自动关中断并加锁，离开作用域自动解锁并恢复中断状态
        unsafe { HEAP.lock().deallocate_layout(NonNull::new(ptr).unwrap(), layout) }
    }
}