#![no_std]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tg_sbi::shutdown;

const fn parse_env_smp(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut num = 0;
    let mut i = 0;
    while i < bytes.len() {
        num = num * 10 + (bytes[i] - b'0') as usize;
        i += 1;
    }
    num
}
pub const MAX_HARTS: usize = match core::option_env!("SMP") {
    Some(s) => parse_env_smp(s),
    None => 1,
};

// 栈大小：每个 hart 4 KiB
pub const STACK_SIZE_PER_HART: usize = 4096;
pub const STACK_SIZE: usize = STACK_SIZE_PER_HART * MAX_HARTS;

#[unsafe(link_section = ".bss.uninit")]
#[used]
static mut STACK: [u8; STACK_SIZE] = [0u8; STACK_SIZE];

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "li t1, {max_harts}",
        "bge a0, t1, 1f",
        "li t1, {stack_size_per_hart}",
        "mul t0, a0, t1",
        "la sp, {stack} + {stack_size}",
        "sub sp, sp, t0",
        "j  {main}",
        "1: wfi",
        "j 1b",
        max_harts = const MAX_HARTS,
        stack_size_per_hart = const STACK_SIZE_PER_HART,
        stack_size = const STACK_SIZE,
        stack      =   sym STACK,
        main       =   sym rust_main,
    )
}

unsafe extern "C" {
    fn rust_main_prelude();
    fn rust_main_execute(hartid: usize);
    fn rust_main_epilogue() -> !;
}

static PRELUDE_FINISHED: AtomicBool = AtomicBool::new(false);
pub static READY_BARRIER: AtomicUsize = AtomicUsize::new(0);
pub static FINISH_BARRIER: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn rust_main(hartid: usize) -> ! {
    if hartid == 0 {
        unsafe { rust_main_prelude(); }
        PRELUDE_FINISHED.store(true, Ordering::Release);
    } else {
        while !PRELUDE_FINISHED.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
    }

    READY_BARRIER.fetch_add(1, Ordering::SeqCst);
    while READY_BARRIER.load(Ordering::SeqCst) < MAX_HARTS {
        core::hint::spin_loop();
    }

    unsafe { rust_main_execute(hartid); }

    FINISH_BARRIER.fetch_add(1, Ordering::SeqCst);

    if hartid == 0 {
        while FINISH_BARRIER.load(Ordering::SeqCst) < MAX_HARTS {
            core::hint::spin_loop();
        }
        unsafe { rust_main_epilogue(); }
    } else {
        loop {
            #[cfg(target_arch = "riscv64")]
            unsafe { core::arch::asm!("wfi") }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    shutdown(true)
}
