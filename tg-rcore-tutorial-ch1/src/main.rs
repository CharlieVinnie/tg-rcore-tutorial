//! # 第一章：应用程序与基本执行环境
//!
//! 本章实现了一个最简单的 RISC-V S 态裸机程序，展示操作系统的最小执行环境。
//!
//! ## 关键概念
//!
//! - `#![no_std]`：不使用 Rust 标准库，改用不依赖操作系统的核心库 `core`
//! - `#![no_main]`：不使用标准的 `main` 入口，自定义裸函数 `_start` 作为入口
//! - 裸函数（naked function）：不生成函数序言/尾声，可在无栈环境下执行
//! - SBI（Supervisor Binary Interface）：S 态软件向 M 态固件请求服务的标准接口
//!
//! 教程阅读建议：
//!
//! - 先看 `_start`：理解无运行时情况下的最小启动流程；
//! - 再看 `rust_main`：理解最小 I/O 路径（SBI 输出 + 关机）；
//! - 最后看 `panic_handler`：理解 no_std 程序的异常收口方式。

// 不使用标准库，因为裸机环境没有操作系统提供系统调用支持
#![no_std]
// 不使用标准入口，因为裸机环境没有 C runtime 进行初始化
#![no_main]
// RISC-V64 架构下启用严格警告和文档检查
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
// 非 RISC-V64 架构允许死代码（用于 cargo publish --dry-run 在主机上通过编译）
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code))]

// 引入 SBI 调用库，提供 console_putchar（输出字符）和 shutdown（关机）功能
// 启用 nobios 特性后，tg_sbi 内建了 M-mode 启动代码，无需外部 SBI 固件
use tg_sbi::{console_getchar, console_putchar, shutdown};
use spin::Once;

/// S 态程序入口点。
///
/// 这是一个裸函数（naked function），放置在 `.text.entry` 段，
/// 链接脚本将其安排在地址 `0x80200000`。
///
/// 裸函数不生成函数序言和尾声，因此可以在没有栈的情况下执行。
/// 它完成两件事：
/// 1. 设置栈指针 `sp`，指向栈顶（栈从高地址向低地址增长）
/// 2. 跳转到 Rust 主函数 `rust_main`
#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 16 * 4096;

    // 在 .bss.uninit 段中分配栈空间
    #[unsafe(link_section = ".boot.stack")]
    static mut STACK: [u8; STACK_SIZE] = [0xCC; STACK_SIZE];

    core::arch::naked_asm!(
        "la sp, {stack} + {stack_size}", // 将 sp 设置为栈顶地址
        "j  {main}",                      // 跳转到 rust_main
        stack_size = const STACK_SIZE,
        stack      =   sym STACK,
        main       =   sym rust_main,
    )
}

/// S 态主函数：打印 "Hello, world!" 并关机。
///
/// 通过 SBI 的 `console_putchar` 逐字节输出字符串，
/// 然后调用 `shutdown` 正常关机退出 QEMU。
use riscv::register::{sepc, sstatus, scause};
use core::arch::asm;
use tg_driver::{DeviceManager};
use virtio_drivers::{Hal};

core::arch::global_asm!(include_str!(env!("APP_ASM")));

use buddy_system_allocator::LockedHeap;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::empty();

const HEAP_SIZE: usize = 0x40000;

#[repr(align(4096))]
struct HeapSpace([u8; HEAP_SIZE]);

static mut HEAP_SPACE: HeapSpace = HeapSpace([0; HEAP_SIZE]);

fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP_SPACE.0) as usize, HEAP_SIZE);
    }
}

// Helper HAL for virtio-drivers initialization in Ch1 (No Paging)
struct HalImpl;
impl Hal for HalImpl {
    fn dma_alloc(pages: usize) -> usize {
        // A bump allocator from a static buffer for DMA
        #[repr(align(4096))]
        #[allow(dead_code)]
        struct DmaBuffer([u8; 1024 * 1024 * 2]);
        #[allow(dead_code)]
        static mut DMA_BUF: DmaBuffer = DmaBuffer([0; 1024 * 1024 * 2]); // 2MB is enough for a framebuffer
        static mut OFFSET: usize = 0;
        unsafe {
            let base = core::ptr::addr_of_mut!(DMA_BUF) as usize;
            // The buffer is aligned to 4096 thanks to #[repr(align(4096))],
            // so we can just add the offset.
            let paddr = base + OFFSET;
            OFFSET += pages * 4096;
            paddr
        }
    }
    
    fn dma_dealloc(_paddr: usize, _pages: usize) -> i32 { 0 }
    
    fn phys_to_virt(paddr: usize) -> usize { paddr }
    
    fn virt_to_phys(vaddr: usize) -> usize { vaddr }
}

static DEVICES: Once<DeviceManager> = Once::new();

use core::fmt::{self, Write};

struct Stdout;
impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.bytes() {
            console_putchar(c);
        }
        Ok(())
    }
}

/// 内部打印函数
pub fn print_fmt(args: fmt::Arguments) {
    Stdout.write_fmt(args).unwrap();
}

macro_rules! print {
    ($($arg:tt)*) => {
        print_fmt(format_args!($($arg)*));
    };
}

// We will just use a global memory region for TrapContext for this barebones demo.
static mut TRAP_CONTEXT: [usize; 32] = [0; 32];
static mut KERNEL_STACK: [u8; 4096] = [0xCC; 4096];
static USER_STACK: [usize; 256] = [0xCC; 256];

core::arch::global_asm!(
    "
    .section .text
    .align 2
    .global __alltraps
    __alltraps:
        csrw sscratch, sp
        la sp, {trap_context}
        
        sd x1, 1*8(sp)
        csrr x1, sscratch
        sd x1, 2*8(sp)
        sd x3, 3*8(sp)
        sd x4, 4*8(sp)
        sd x5, 5*8(sp)
        sd x6, 6*8(sp)
        sd x7, 7*8(sp)
        sd x8, 8*8(sp)
        sd x9, 9*8(sp)
        sd x10, 10*8(sp)
        sd x11, 11*8(sp)
        sd x12, 12*8(sp)
        sd x13, 13*8(sp)
        sd x14, 14*8(sp)
        sd x15, 15*8(sp)
        sd x16, 16*8(sp)
        sd x17, 17*8(sp)
        sd x18, 18*8(sp)
        sd x19, 19*8(sp)
        sd x20, 20*8(sp)
        sd x21, 21*8(sp)
        sd x22, 22*8(sp)
        sd x23, 23*8(sp)
        sd x24, 24*8(sp)
        sd x25, 25*8(sp)
        sd x26, 26*8(sp)
        sd x27, 27*8(sp)
        sd x28, 28*8(sp)
        sd x29, 29*8(sp)
        sd x30, 30*8(sp)
        sd x31, 31*8(sp)
        
        la sp, {kernel_stack}
        li t0, 4096
        add sp, sp, t0
        
        call trap_handler
        
        la sp, {trap_context}
        sd a0, 10*8(sp)
        
        csrr t1, sepc
        addi t1, t1, 4
        csrw sepc, t1
        
    .global __restore
    __restore:
        la sp, {trap_context}
        
        ld x1, 1*8(sp)
        ld x3, 3*8(sp)
        ld x4, 4*8(sp)
        ld x5, 5*8(sp)
        ld x6, 6*8(sp)
        ld x7, 7*8(sp)
        ld x8, 8*8(sp)
        ld x9, 9*8(sp)
        ld x10, 10*8(sp)
        ld x11, 11*8(sp)
        ld x12, 12*8(sp)
        ld x13, 13*8(sp)
        ld x14, 14*8(sp)
        ld x15, 15*8(sp)
        ld x16, 16*8(sp)
        ld x17, 17*8(sp)
        ld x18, 18*8(sp)
        ld x19, 19*8(sp)
        ld x20, 20*8(sp)
        ld x21, 21*8(sp)
        ld x22, 22*8(sp)
        ld x23, 23*8(sp)
        ld x24, 24*8(sp)
        ld x25, 25*8(sp)
        ld x26, 26*8(sp)
        ld x27, 27*8(sp)
        ld x28, 28*8(sp)
        ld x29, 29*8(sp)
        ld x30, 30*8(sp)
        ld x31, 31*8(sp)
        
        ld sp, 2*8(sp)
        
        sret
    ",
    trap_context = sym TRAP_CONTEXT,
    kernel_stack = sym KERNEL_STACK,
);

macro_rules! println {
    ($fmt:literal $(, $($arg: tt)+)?) => {
        print!(concat!($fmt, "\n") $(, $($arg)+)?);
    }
}

/// 早期异常处理函数
#[unsafe(no_mangle)]
pub unsafe extern "C" fn early_trap_handler() -> ! {
    let scause = scause::read().cause();
    let sepc = sepc::read();
    let stval = riscv::register::stval::read();
    println!("KERNEL PANIC: Early Trap! cause: {:?}, sepc: {:#x}, stval: {:#x}", scause, sepc, stval);
    shutdown(true);
}

fn clear_bss() {
    unsafe {
        unsafe extern "C" {
            fn sbss();
            fn ebss();
        }
        let mut ptr = sbss as *mut u8;
        let end = ebss as *mut u8;
        while ptr < end {
            ptr.write_volatile(0);
            ptr = ptr.add(1);
        }
    }
}

extern "C" fn rust_main() -> ! {
    unsafe {
        asm!(
            "la t0, early_trap_handler",
            "csrw stvec, t0",
        );
    }
    clear_bss();
    init_heap();
    println!("Hello to Tangram OS!");


    // 2. Load User App
    unsafe extern "C" {
        fn user_app_start();
        fn user_app_end();
    }

    println!("GPU Initialized!");

    let user_base = 0x80600000;

    let app_size = user_app_end as *const () as usize - user_app_start as *const () as usize;

    DEVICES.call_once(DeviceManager::new::<HalImpl>);

    unsafe {
        let src = core::slice::from_raw_parts(user_app_start as *const u8, app_size);
        let dst = core::slice::from_raw_parts_mut(user_base as *mut u8, 0x20_0000);
        dst.fill(0);
        dst[..app_size].copy_from_slice(src);
        
        asm!("fence.i");
    }

    // 3. Setup U-mode transition
    unsafe {
        sstatus::set_spp(sstatus::SPP::User);
        sepc::write(user_base);
        
        // Setup user stack
        let user_stack_ptr = USER_STACK.as_ptr() as *mut usize;
        let sp = user_stack_ptr.add(256) as usize;

        let mut sp_register = sp;
        
        asm!(
            "csrw sscratch, {sp_register}",
            "csrr {sp_register}, sscratch",
            sp_register = inout(reg) sp_register,
        );


        
        // We must place __alltraps in stvec so that the syscall works
        asm!(
            "la t0, __alltraps",
            "csrw stvec, t0",
            "mv sp, {sp}",
            "sret",
            sp = in(reg) sp_register,
            options(noreturn)
        );
    }
}

/// Ch1 U-mode syscall trap handler
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trap_handler(
    _a0: usize, a1: usize, _a2: usize, _a3: usize, _a4: usize, _a5: usize, _a6: usize, a7: usize, _epc: usize
) -> usize {
    let cause = scause::read().cause();
    match cause {
        scause::Trap::Exception(scause::Exception::UserEnvCall) => {
            // Syscall
            let id = a7;
            match id {
                56 => { // open
                    3 // return 3 based on user POSIX feedback
                }
                222 => { // mmap
                    let gpu = DEVICES.get().unwrap().get_gpu().unwrap();
                    gpu.get_framebuffer().unwrap().as_mut_ptr() as usize
                }
                29 => { // ioctl
                    let req = a1;
                    if req == 1 { // FB_FLUSH
                        let gpu = DEVICES.get().unwrap().get_gpu().unwrap();
                        gpu.flush().unwrap();
                    }
                    0
                }
                93 => { // exit
                    for c in b"Goodbye!\nPress any key to exit..." {
                        console_putchar(*c);
                    }
                    console_getchar();
                    console_putchar(b'\n');
                    shutdown(false);
                }
                _ => {
                    panic!("Unsupported syscall ID: {}", id);
                }
            }
        }
        _ => {
            let inst = unsafe { *(riscv::register::sepc::read() as *const u16) };
            println!("Instruction at sepc: {:#x}", inst);
            unsafe { println!("Stack pointer is: {:#x}", TRAP_CONTEXT[2]); }
            let current_sp: usize;
            unsafe { core::arch::asm!("mv {}, sp", out(reg) current_sp); }
            println!("Current sp is {:#x}", current_sp);
            panic!("Unexpected trap: {:?}, sepc: {:#x}, stval: {:#x}", cause, riscv::register::sepc::read(), riscv::register::stval::read());
        }
    }
}

/// panic 处理函数。
///
/// `#![no_std]` 环境下必须自行实现。发生 panic 时以异常状态关机。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("KERNEL PANIC: {}", info);
    shutdown(true) // true 表示异常关机
}

/// 非 RISC-V64 架构的占位模块。
///
/// 提供 `main` 等符号，使得在主机平台（如 x86_64）上也能通过编译，
/// 满足 `cargo publish --dry-run` 和 `cargo test` 的需求。
#[cfg(not(target_arch = "riscv64"))]
mod stub {
    /// 主机平台占位入口
    #[unsafe(no_mangle)]
    pub extern "C" fn main() -> i32 {
        0
    }

    /// C 运行时占位
    #[unsafe(no_mangle)]
    pub extern "C" fn __libc_start_main() -> i32 {
        0
    }

    /// Rust 异常处理人格占位
    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}
}
