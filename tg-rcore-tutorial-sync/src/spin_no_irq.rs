use core::ops::{Deref, DerefMut};
use riscv::register::sstatus;
use spin::Mutex;

/// 带中断屏蔽的自旋锁容器。
///
/// 在 SMP 环境中，当获取锁时主动关闭中断 `sstatus.SIE`，
/// 避免死锁（例如内核时钟中断尝试获取当前 Hart 已持有的锁）。
pub struct SpinNoIrq<T> {
    inner: Mutex<T>,
}

/// 关中断自旋锁的守卫，释放时会自动恢复中断状态。
pub struct SpinNoIrqGuard<'a, T> {
    guard: core::option::Option<spin::MutexGuard<'a, T>>,
    sie_before: bool,
}

impl<T> SpinNoIrq<T> {
    /// 构造一个新的关中断自旋锁。
    pub const fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(value),
        }
    }

    /// 获取锁并返回守卫，会自动关闭当前核的中断。
    pub fn lock(&self) -> SpinNoIrqGuard<'_, T> {
        let sie_before = sstatus::read().sie();
        // 关键：在申请锁之前，先关闭本核的中断！
        unsafe { sstatus::clear_sie() };
        SpinNoIrqGuard {
            guard: Some(self.inner.lock()),
            sie_before,
        }
    }
}

impl<'a, T> Deref for SpinNoIrqGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap().deref()
    }
}

impl<'a, T> DerefMut for SpinNoIrqGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().unwrap().deref_mut()
    }
}

impl<'a, T> Drop for SpinNoIrqGuard<'a, T> {
    fn drop(&mut self) {
        // 先释放 MutexGuard
        self.guard.take();
        // 如果进入临界区前开启了中断，则重开中断
        if self.sie_before {
            unsafe { sstatus::set_sie() };
        }
    }
}
