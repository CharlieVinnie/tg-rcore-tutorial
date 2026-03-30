#![no_std]
#![no_main]

fn syscall(id: usize, args: [usize; 3]) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") args[0] => ret,
            in("a1") args[1],
            in("a2") args[2],
            in("a7") id,
        );
    }
    ret
}

fn sys_write(fd: usize, buf: usize, len: usize) -> isize {
    syscall(64, [fd, buf, len])
}

fn sys_exit(code: i32) -> ! {
    syscall(93, [code as usize, 0, 0]);
    unreachable!()
}

fn print(s: &str) {
    sys_write(1, s.as_ptr() as usize, s.len());
}

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    print("BOOM!");
    print("");
    0
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    sys_exit(-1)
}
