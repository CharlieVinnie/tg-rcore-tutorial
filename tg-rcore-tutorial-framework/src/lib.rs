#![no_std]

use core::sync::atomic::{AtomicUsize, Ordering};
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

// 栈大小：每个 hart 64 KiB
pub const STACK_SIZE_PER_HART: usize = 16 * 4096;
pub const STACK_SIZE: usize = STACK_SIZE_PER_HART * MAX_HARTS;

#[unsafe(link_section = ".boot.stack")]
#[used]
static mut STACK: [u8; STACK_SIZE] = [0u8; STACK_SIZE];

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() {
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

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn secondary_hart_start() {
    core::arch::naked_asm!(
        "mv sp, a1",
        "j {main_secondary}",
        main_secondary = sym rust_main_secondary,
    )
}

unsafe extern "C" {
    fn rust_main_prelude();
    fn rust_main_execute(hartid: usize);
    fn rust_main_epilogue() -> !;
}

pub static READY_BARRIER: AtomicUsize = AtomicUsize::new(0);
pub static FINISH_BARRIER: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
extern "C" fn rust_main(hartid: usize) -> ! {
    // 只有主核心（Hart 0）进入此函数
    unsafe { rust_main_prelude(); }

    // 唤醒其余从属核心
    for target_hart in 1..MAX_HARTS {
        let stack_top = unsafe { (core::ptr::addr_of!(STACK) as *const u8).add(STACK_SIZE - target_hart * STACK_SIZE_PER_HART) as usize };
        tg_sbi::sbi_hart_start(target_hart, secondary_hart_start as *const () as usize, stack_top);
    }

    READY_BARRIER.fetch_add(1, Ordering::SeqCst);
    while READY_BARRIER.load(Ordering::SeqCst) < MAX_HARTS {
        core::hint::spin_loop();
    }

    unsafe { rust_main_execute(hartid); }

    FINISH_BARRIER.fetch_add(1, Ordering::SeqCst);
    while FINISH_BARRIER.load(Ordering::SeqCst) < MAX_HARTS {
        core::hint::spin_loop();
    }
    
    unsafe { rust_main_epilogue(); }
}

#[unsafe(no_mangle)]
extern "C" fn rust_main_secondary(hartid: usize) -> ! {
    READY_BARRIER.fetch_add(1, Ordering::SeqCst);
    while READY_BARRIER.load(Ordering::SeqCst) < MAX_HARTS {
        core::hint::spin_loop();
    }

    unsafe { rust_main_execute(hartid); }

    FINISH_BARRIER.fetch_add(1, Ordering::SeqCst);
    loop {
        #[cfg(target_arch = "riscv64")]
        unsafe { core::arch::asm!("wfi") };
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    shutdown(true)
}
