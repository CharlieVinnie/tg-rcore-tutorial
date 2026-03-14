// Patches tg_syscall

pub use tg_syscall::*;

#[inline]
pub fn mmap(start: usize, len: usize, prot: usize, flags: usize, fd: usize, offset: usize) -> isize {
    unsafe { native::syscall6(SyscallId::MMAP, start, len, prot, flags, fd, offset) }
}

#[inline]
pub fn ioctl(fd: usize, request: usize, arg: usize) -> isize {
    unsafe { native::syscall3(SyscallId::IOCTL, fd, request, arg) }
}