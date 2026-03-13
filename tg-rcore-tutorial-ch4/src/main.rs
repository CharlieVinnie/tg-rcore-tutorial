//! # 第四章：地址空间
//!
//! 本章在第三章"多道程序与分时多任务"的基础上，引入了 **RISC-V Sv39 虚拟内存机制**，
//! 为每个用户进程提供独立的地址空间，实现进程间的内存隔离。
//!
//! ## 核心概念
//!
//! - **虚拟内存**：通过 Sv39 三级页表将虚拟地址映射到物理地址
//! - **地址空间隔离**：每个进程拥有独立的页表，无法访问其他进程的内存
//! - **异界传送门（MultislotPortal）**：解决跨地址空间的上下文切换问题
//! - **ELF 加载**：解析 ELF 格式的用户程序并映射到独立地址空间
//! - **内核堆分配器**：支持动态内存分配（`alloc` crate）
//! - **地址翻译**：系统调用中需要将用户虚拟地址翻译为物理地址
//!
//! 教程阅读建议：
//!
//! - 先看 `kernel_space`：建立“内核恒等映射 + 传送门映射”的总体视图；
//! - 再看 `schedule`：理解跨地址空间执行与 trap 返回路径；
//! - 最后看 `impls`：重点掌握 `translate()` 如何保证用户指针访问安全。

// 不使用标准库，裸机环境没有操作系统提供系统调用支持
#![no_std]
// 不使用标准入口，裸机环境没有 C runtime 进行初始化
#![no_main]
// RISC-V64 架构下启用严格警告和文档检查
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
// 非 RISC-V64 架构允许死代码和未使用导入（用于 cargo publish --dry-run）
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code, unused_imports))]

// 进程管理模块：定义 Process 结构体，包含地址空间和上下文
mod process;
mod device_manager;
mod kernel_space;
mod user_reader;

// 引入控制台输出宏（print! / println!），由 tg_console 库提供
#[macro_use]
extern crate tg_console;

// 启用 alloc crate，提供堆分配能力（Vec、Box 等）
extern crate alloc;

// ========== 导入 ==========

use crate::{
    device_manager::{DEVICES, init_devices}, impls::SyscallContext, kernel_space::{KERNEL_SPACE, Sv39Manager}, process::Process
};
use alloc::{alloc::alloc, vec::Vec};
use tg_driver::{visit_virtio_ranges};
use core::{alloc::Layout, cell::UnsafeCell};
use impls::Console;
use riscv::register::*;
// 非 RISC-V64 使用占位 Sv39 类型
#[cfg(not(target_arch = "riscv64"))]
use stub::Sv39;
use tg_console::log;
// 异界传送门：解决跨地址空间上下文切换的核心组件
use tg_kernel_context::{foreign::MultislotPortal, LocalContext};
// RISC-V64 使用真正的 Sv39 类型
#[cfg(target_arch = "riscv64")]
use tg_kernel_vm::page_table::Sv39;
use tg_kernel_vm::{
    AddressSpace, page_table::{MmuMeta, PPN, VAddr, VPN, VmFlags, VmMeta}
};
use tg_sbi::{self, console_getchar};
use tg_syscall::Caller;
use xmas_elf::ElfFile;

// ========== 辅助函数 ==========

/// 从字符串构建页表项标志位（编译期常量）。
///
/// 字符串格式如 `"U_WRV"` 表示 User + Write + Read + Valid。
#[cfg(target_arch = "riscv64")]
const fn build_flags(s: &str) -> VmFlags<Sv39> {
    VmFlags::build_from_str(s)
}

/// 从字符串解析页表项标志位（运行期）。
#[cfg(target_arch = "riscv64")]
fn parse_flags(s: &str) -> Result<VmFlags<Sv39>, ()> {
    s.parse()
}

// 非 RISC-V64 架构使用占位实现
#[cfg(not(target_arch = "riscv64"))]
use stub::{build_flags, parse_flags};

// ========== 启动相关 ==========

// 将用户程序的二进制数据内联到内核镜像中
#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!(env!("APP_ASM")));

// 定义内核入口点：分配 24 KiB 内核栈。
//
// 这里不再调用 tg_linker::boot0! 宏，避免外部已发布版本与 Rust 2024
// 在属性语义上的兼容差异影响本 crate 的发布校验。
#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 16 * 4096;
    #[unsafe(link_section = ".boot.stack")]
    static mut STACK: [u8; STACK_SIZE] = [0u8; STACK_SIZE];

    core::arch::naked_asm!(
        "la sp, {stack} + {stack_size}",
        "j  {main}",
        stack = sym STACK,
        stack_size = const STACK_SIZE,
        main = sym rust_main,
    )
}

// 物理内存容量 = 24 MiB（QEMU virt 平台的 RAM 大小）
const MEMORY: usize = 24 << 20;

// 异界传送门所在虚页：虚拟地址空间的最高页
// 传送门同时映射到内核和所有用户地址空间的相同虚拟地址，
// 使得切换 satp（地址空间）后代码仍然可以执行
const PROTAL_TRANSIT: VPN<Sv39> = VPN::MAX;

// ========== 进程列表 ==========

/// 全局进程列表（用 UnsafeCell 包装以允许内部可变性）。
struct ProcessList(UnsafeCell<Vec<Process>>);

unsafe impl Sync for ProcessList {}

impl ProcessList {
    const fn new() -> Self {
        Self(UnsafeCell::new(Vec::new()))
    }

    unsafe fn get_mut(&self) -> &mut Vec<Process> {
        unsafe { &mut *self.0.get() }
    }
}

/// 全局进程列表实例。
static PROCESSES: ProcessList = ProcessList::new();

// ========== 内核主函数 ==========

/// 内核主函数：初始化各子系统，建立内核地址空间，加载用户进程。
///
/// 与前几章不同，本章需要：
/// 1. 初始化内核堆（支持动态分配）
/// 2. 建立异界传送门（跨地址空间切换）
/// 3. 建立内核地址空间（Sv39 页表）
/// 4. 为每个用户程序解析 ELF 并创建独立地址空间
/// 5. 建立调度线程执行用户进程
extern "C" fn rust_main() -> ! {
    let layout = tg_linker::KernelLayout::locate();
    // 第一步：清零 BSS 段
    unsafe { layout.zero_bss() };
    // 第二步：初始化控制台
    tg_console::init_console(&Console);
    tg_console::set_log_level(option_env!("LOG"));
    tg_console::test_log();
    // 第三步：初始化内核堆分配器
    // 堆的起始地址为内核镜像起始处，可用内存为内核镜像之后到物理内存末尾
    tg_kernel_alloc::init(layout.start() as _);
    unsafe {
        tg_kernel_alloc::transfer(core::slice::from_raw_parts_mut(
            layout.end() as _,
            MEMORY - layout.len(),
        ))
    };
    // 第四步：分配异界传送门的物理页面
    // 传送门大小需要适配 1 个 slot（对应 1 个并发切换）
    let portal_size = MultislotPortal::calculate_size(1);
    let portal_layout = Layout::from_size_align(portal_size, 1 << Sv39::PAGE_BITS).unwrap();
    let portal_ptr = unsafe { alloc(portal_layout) };
    assert!(portal_layout.size() < 1 << Sv39::PAGE_BITS);
    // 第五步：建立内核地址空间（恒等映射 + 传送门映射）
    let mut ks = kernel_space(layout, MEMORY, portal_ptr as _);
    let portal_idx = PROTAL_TRANSIT.index_in(Sv39::MAX_LEVEL);
    // 第六步：加载用户程序
    // 解析每个 ELF 文件，创建独立地址空间，映射传送门
    for (i, elf) in tg_linker::AppMeta::locate().iter().enumerate() {
        let base = elf.as_ptr() as usize;
        log::info!("detect app[{i}]: {base:#x}..{:#x}", base + elf.len());
        if let Some(process) = Process::new(ElfFile::new(elf).unwrap()) {
            // 将内核传送门页表项共享到用户地址空间
            // 这样传送门在两个地址空间的虚拟地址相同
            process.address_space.root()[portal_idx] = ks.root()[portal_idx];
            unsafe { PROCESSES.get_mut().push(process) };
        }
    }

    // 第七步：建立调度栈（映射到内核地址空间的高地址区域）
    const PAGE: Layout =
        unsafe { Layout::from_size_align_unchecked(8 << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS) };
    let pages = 8;
    let stack = unsafe { alloc(PAGE) };
    ks.map_extern(
        VPN::new((1 << 26) - pages)..VPN::new(1 << 26),
        PPN::new(stack as usize >> Sv39::PAGE_BITS),
        build_flags("_WRV"),
    );
    // Remember to map VirtIO address!
    visit_virtio_ranges(
        |start, end| {
            assert!(start.trailing_zeros() >= Sv39::PAGE_BITS as _);
            assert!(end.trailing_zeros() >= Sv39::PAGE_BITS as _);
            ks.map_extern(
                VPN::new(start >> Sv39::PAGE_BITS)..VPN::new(end >> Sv39::PAGE_BITS),
                PPN::new(start >> Sv39::PAGE_BITS),
                build_flags("_WRV"),
            );
        }
    );
    KERNEL_SPACE.init(ks);
    init_devices();
    // 第八步：建立调度线程
    // 调度线程在独立的异常域运行，内核异常不会导致整个系统崩溃
    let mut scheduling = LocalContext::thread(schedule as *const () as _, false);
    *scheduling.sp_mut() = 1 << 38;
    unsafe { scheduling.execute() };
    // 如果从 execute() 返回，说明调度线程发生了异常
    log::error!("stval = {:#x}", stval::read());
    panic!("trap from scheduling thread: {:?}", scause::read().cause());
}

// ========== 调度函数 ==========

/// 调度函数：在异界传送门中循环执行所有用户进程。
///
/// 工作流程：
/// 1. 初始化传送门和系统调用
/// 2. 取出第一个进程，通过传送门切换到其地址空间并执行
/// 3. Trap 返回后处理系统调用或异常
/// 4. 进程退出后从列表中移除，继续下一个
extern "C" fn schedule() -> ! {
    // 初始化异界传送门（设置传送门页面的虚拟地址和 slot 数量）
    let portal = unsafe { MultislotPortal::init_transit(PROTAL_TRANSIT.base().val(), 1) };
    // 初始化系统调用处理
    // 比第三章多了 memory（mmap/munmap/sbrk）
    tg_syscall::init_io(&SyscallContext);
    tg_syscall::init_process(&SyscallContext);
    tg_syscall::init_scheduling(&SyscallContext);
    tg_syscall::init_clock(&SyscallContext);
    tg_syscall::init_trace(&SyscallContext);
    tg_syscall::init_memory(&SyscallContext);

    // 调度循环：持续执行直到所有进程完成
    while !unsafe { PROCESSES.get_mut().is_empty() } {
        let ctx = unsafe { &mut PROCESSES.get_mut()[0].context };
        // 通过传送门执行用户进程：
        // 1. 跳转到传送门页面
        // 2. 在传送门内切换 satp 到用户地址空间
        // 3. 恢复用户寄存器，执行 sret 进入 U-mode
        // 4. 用户触发 Trap 后，传送门切换回内核地址空间
        unsafe { ctx.execute(portal, ()) };

        // 处理 Trap
        match scause::read().cause() {
            // ─── 系统调用 ───
            scause::Trap::Exception(scause::Exception::UserEnvCall) => {
                use tg_syscall::{SyscallId as Id, SyscallResult as Ret};

                let ctx = &mut ctx.context;
                let id: Id = ctx.a(7).into();
                let args = [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)];

                // 统计系统调用次数
                let id_usize = ctx.a(7);
                if id_usize < 500 {
                    unsafe { PROCESSES.get_mut()[0].syscall_counts[id_usize] += 1 };
                }

                match tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
                    Ret::Done(ret) => match id {
                        // exit：移除进程
                        Id::EXIT => unsafe {
                            PROCESSES.get_mut().remove(0);
                        },
                        // 其他系统调用：写回返回值，sepc += 4
                        _ => {
                            *ctx.a_mut(0) = ret as _;
                            ctx.move_next();
                        }
                    },
                    // 不支持的系统调用：杀死进程
                    Ret::Unsupported(_) => {
                        log::info!("id = {id:?}");
                        unsafe { PROCESSES.get_mut().remove(0) };
                    }
                }
            }
            scause::Trap::Interrupt(scause::Interrupt::SupervisorExternal) => {
                DEVICES.get().unwrap().handle_external_interrupt();
            }
            // ─── 其他异常/中断：杀死进程 ───
            e => {
                log::error!(
                    "unsupported trap: {e:?}, stval = {:#x}, sepc = {:#x}",
                    stval::read(),
                    ctx.context.pc()
                );
                unsafe { PROCESSES.get_mut().remove(0) };
            }
        }
    }
    // 所有进程执行完毕，关机
    println!("Press any key to shutdown...");
    console_getchar();
    println!("");
    tg_sbi::shutdown(false)
}

// ========== panic 处理 ==========

/// panic 处理函数：打印错误信息后以异常状态关机。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log::error!("{info}");
    tg_sbi::shutdown(true)
}

// ========== 内核地址空间构建 ==========

/// 建立内核地址空间。
///
/// 包含以下映射：
/// - **恒等映射**（Identity Mapping）：内核代码段、数据段、堆区域
///   虚拟地址 == 物理地址，方便内核直接访问物理内存
/// - **传送门映射**：将传送门物理页映射到虚拟地址空间最高页
fn kernel_space(
    layout: tg_linker::KernelLayout,
    memory: usize,
    portal: usize,
) -> AddressSpace<Sv39, Sv39Manager> {
    let mut space = AddressSpace::<Sv39, Sv39Manager>::new();
    // 映射内核各段（恒等映射：VPN == PPN）
    for region in layout.iter() {
        log::info!("{region}");
        use tg_linker::KernelRegionTitle::*;
        let flags = match region.title {
            Text => "X_RV",    // 代码段：可执行、可读
            Rodata => "__RV",  // 只读数据段：只读
            Data | Boot => "_WRV", // 数据段/启动段：可读写
        };
        let s = VAddr::<Sv39>::new(region.range.start);
        let e = VAddr::<Sv39>::new(region.range.end);
        space.map_extern(
            s.floor()..e.ceil(),
            PPN::new(s.floor().val()),
            build_flags(flags),
        )
    }
    // 映射内核堆区域（恒等映射）
    log::info!(
        "(heap) ---> {:#10x}..{:#10x}",
        layout.end(),
        layout.start() + memory
    );
    let s = VAddr::<Sv39>::new(layout.end());
    let e = VAddr::<Sv39>::new(layout.start() + memory);
    space.map_extern(
        s.floor()..e.ceil(),
        PPN::new(s.floor().val()),
        build_flags("_WRV"),
    );
    // 映射异界传送门到虚拟地址空间最高页
    // 标志位 "__G_XWRV" 表示全局、可执行、可读写、有效
    space.map_extern(
        PROTAL_TRANSIT..PROTAL_TRANSIT + 1,
        PPN::new(portal >> Sv39::PAGE_BITS),
        build_flags("__G_XWRV"),
    );
    println!();
    // 激活内核地址空间：写入 satp 寄存器，开启 Sv39 分页模式
    unsafe { satp::set(satp::Mode::Sv39, 0, space.root_ppn().val()) };
    space
}

// ========== 接口实现 ==========

/// 各依赖库所需接口的具体实现。
///
/// 与前几章不同，本章的系统调用实现需要进行**地址翻译**：
/// 用户传入的指针是虚拟地址，内核需要通过页表将其翻译为物理地址才能访问。
mod impls {
    use crate::{PROCESSES, Sv39, build_flags, device_manager::DEVICES, kernel_space::Sv39Manager, user_reader::read_zero_ended_list};
    use tg_console::log;
    use tg_driver::InputEvent;
    use tg_kernel_vm::{
        AddressSpace, page_table::{MmuMeta, PPN, VAddr, VmFlags}
    };
    use tg_syscall::*;

    /// 控制台实现：通过 SBI 逐字符输出
    pub struct Console;

    impl tg_console::Console for Console {
        #[inline]
        fn put_char(&self, c: u8) {
            tg_sbi::console_putchar(c);
        }
    }

    /// 系统调用上下文实现
    pub struct SyscallContext;
    
    const GPU_FD: usize = 3;
    const KEYBOARD_FD: usize = 4;
    const FB_FLUSH: usize = 1;
    const FB_GET_RESOLUTION: usize = 2;

    const READABLE: VmFlags<Sv39> = build_flags("URV");
    const WRITABLE: VmFlags<Sv39> = build_flags("UWRV");

    fn addr_space_of(caller: Caller) -> &'static mut AddressSpace<Sv39, Sv39Manager> {
        unsafe { &mut PROCESSES.get_mut().get_mut(caller.entity).unwrap().address_space }
    }

    /// IO 系统调用实现
    ///
    /// **与前几章的关键区别**：用户传入的 `buf` 是虚拟地址，
    /// 需要通过 `address_space.translate()` 翻译为物理地址才能访问。
    impl IO for SyscallContext {
        fn write(&self, caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            match fd {
                STDOUT | STDDEBUG => {
                    if let Some(ptr) = addr_space_of(caller).translate::<u8>(VAddr::new(buf), READABLE)
                    {
                        print!("{}", unsafe {
                            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                                ptr.as_ptr(),
                                count,
                            ))
                        });
                        count as _
                    } else {
                        log::error!("ptr not readable");
                        -1
                    }
                }
                _ => {
                    log::error!("unsupported fd: {fd}");
                    -1
                }
            }
        }

        fn read(&self, caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            match fd {
                KEYBOARD_FD => {
                    let Some(mut ptr) = addr_space_of(caller).translate::<InputEvent>(VAddr::new(buf), WRITABLE)
                    else {
                        log::error!("ptr not writable");
                        return -1;
                    };
                    let Some(keyboard) = DEVICES.get().unwrap().get_keyboard()
                    else {
                        tg_console::log::error!("Keyboard not initialized");
                        return -1;
                    };
                    let mut read_count = 0;
                    while read_count + core::mem::size_of::<InputEvent>() <= count {
                        let Some(event) = keyboard.read_event() else { break; };
                        let event_ptr = ptr.as_ptr();
                        unsafe { event_ptr.write_volatile(event); }
                        ptr = unsafe { ptr.add(core::mem::size_of::<InputEvent>()) };
                        read_count += core::mem::size_of::<InputEvent>();
                    }
                    read_count as isize
                }
                _ => {
                    tg_console::log::error!("unsupported fd: {fd}");
                    -1
                }
            }
        }

        fn open(&self, caller: tg_syscall::Caller, path: usize, _flags: usize) -> isize {
            let Ok(path_slice) = read_zero_ended_list(addr_space_of(caller), path)
            else {
                log::error!("path not readable");
                return -1;
            };
            let path_str = unsafe { core::str::from_utf8_unchecked(path_slice) };
            // hardcoded paths
            if path_str == "/dev/fb0" {
                GPU_FD as isize
            } else if path_str == "/dev/input0" {
                KEYBOARD_FD as isize
            } else {
                -1
            }
        }

        fn ioctl(&self, caller: tg_syscall::Caller, fd: usize, request: usize, argp: usize) -> isize {
            match fd {
                GPU_FD => {
                    let Some(gpu) = DEVICES.get().unwrap().get_gpu() else {
                        tg_console::log::error!("GPU not initialized");
                        return -1;
                    };
                    match request {
                        FB_FLUSH => {
                            gpu.flush().unwrap();
                            0
                        }
                        FB_GET_RESOLUTION => {
                            let (w, h) = gpu.resolution().unwrap();
                            let Some(res_ptr) = addr_space_of(caller).translate::<u32>(VAddr::new(argp), WRITABLE)
                            else {
                                tg_console::log::error!("argp not writable");
                                return -1;
                            };
                            unsafe {
                                res_ptr.write_volatile(w as u32);
                                res_ptr.add(1).write_volatile(h as u32);
                            }
                            0
                        }
                        _ => {
                            tg_console::log::error!("unsupported request: {request}");
                            -1
                        }
                    }
                }
                _ => {
                    tg_console::log::error!("unsupported fd: {fd}");
                    -1
                }
            }
        }
    }

    /// Process 系统调用实现
    impl Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: Caller, _status: usize) -> isize {
            0
        }

        /// sbrk：调整进程堆空间大小
        ///
        /// 这是本章新增的系统调用，允许用户程序动态扩展/收缩堆内存。
        /// 返回旧的 break 地址，失败返回 -1。
        fn sbrk(&self, caller: Caller, size: i32) -> isize {
            if let Some(process) = unsafe { PROCESSES.get_mut() }.get_mut(caller.entity) {
                if let Some(old_brk) = process.change_program_brk(size as isize) {
                    old_brk as isize
                } else {
                    -1
                }
            } else {
                -1
            }
        }
    }

    /// Scheduling 系统调用实现
    impl Scheduling for SyscallContext {
        #[inline]
        fn sched_yield(&self, _caller: Caller) -> isize {
            0
        }
    }

    /// Clock 系统调用实现
    ///
    /// 与前章不同：需要通过 translate() 将用户传入的 TimeSpec 指针
    /// 翻译为内核可访问的物理地址，然后写入时间数据。
    impl Clock for SyscallContext {
        #[inline]
        fn clock_gettime(&self, caller: Caller, clock_id: ClockId, tp: usize) -> isize {
            match clock_id {
                ClockId::CLOCK_MONOTONIC => {
                    if let Some(mut ptr) = unsafe { PROCESSES.get_mut() }
                        .get_mut(caller.entity)
                        .unwrap()
                        .address_space
                        .translate::<TimeSpec>(VAddr::new(tp), WRITABLE)
                    {
                        let time = riscv::register::time::read() * 10000 / 125;
                        *unsafe { ptr.as_mut() } = TimeSpec {
                            tv_sec: time / 1_000_000_000,
                            tv_nsec: time % 1_000_000_000,
                        };
                        0
                    } else {
                        log::error!("ptr not readable");
                        -1
                    }
                }
                _ => -1,
            }
        }
    }

    /// Trace 系统调用实现（练习题需要完成的部分）
    ///
    /// 引入虚存机制后，原来的 trace 实现无效了，需要：
    /// - 读取时检查用户地址是否可见且可读
    /// - 写入时检查用户地址是否可见且可写
    /// - 使用 translate() 方法进行地址翻译和权限检查
    impl Trace for SyscallContext {
        #[inline]
        fn trace(
            &self,
            caller: Caller,
            trace_request: usize,
            id: usize,
            data: usize,
        ) -> isize {
            match trace_request {
                0 => {
                    const READABLE: VmFlags<Sv39> = build_flags("U_RV");
                    if let Some(ptr) = unsafe { PROCESSES.get_mut() }
                        .get_mut(caller.entity)
                        .unwrap()
                        .address_space
                        .translate::<u8>(VAddr::new(id), READABLE) {
                        unsafe { ptr.read_volatile() as isize }
                    } else {
                        -1
                    }
                }
                1 => {
                    const WRITABLE: VmFlags<Sv39> = build_flags("U_WRV");
                    if let Some(ptr) = unsafe { PROCESSES.get_mut() }
                        .get_mut(caller.entity)
                        .unwrap()
                        .address_space
                        .translate::<u8>(VAddr::new(id), WRITABLE) {
                        unsafe { ptr.write_volatile((data & 0xff) as u8) };
                        0
                    } else {
                        -1
                    }
                }
                2 => {
                    if id < 500 {
                        unsafe { PROCESSES.get_mut()[caller.entity].syscall_counts[id] as isize }
                    } else {
                        0
                    }
                }
                _ => -1,
            }
        }
    }

    /// Memory 系统调用实现（练习题需要完成的部分）
    ///
    /// - `mmap`：将物理内存映射到用户虚拟地址空间
    /// - `munmap`：取消虚拟内存映射
    impl Memory for SyscallContext {
        fn mmap(
            &self,
            caller: Caller,
            addr: usize,
            len: usize,
            prot: i32,
            _flags: i32,
            fd: i32,
            _offset: usize,
        ) -> isize {
            if addr % 4096 != 0 {
                return -1;
            }
            if prot & !0b111 != 0 || prot == 0 {
                return -1;
            }
            if len == 0 {
                return -1;
            }
            
            let mut flags_bytes = [b'U', b'_', b'_', b'_', b'V'];
            if prot & 1 != 0 {
                flags_bytes[3] = b'R';
            }
            if prot & 2 != 0 {
                flags_bytes[2] = b'W';
            }
            if prot & 4 != 0 {
                flags_bytes[1] = b'X';
            }
            let flags_str = unsafe { core::str::from_utf8_unchecked(&flags_bytes) };
            let flags = build_flags(flags_str);
            
            let process = unsafe { PROCESSES.get_mut() }.get_mut(caller.entity).unwrap();

            let start;
            let end;

            if addr != 0 {
                start = VAddr::<Sv39>::new(addr).floor();
                end = VAddr::<Sv39>::new(addr + len).ceil();
            } else {
                let brk = process.program_brk;
                start = VAddr::<Sv39>::new(brk).floor();
                end = VAddr::<Sv39>::new(brk + len).ceil();
                process.program_brk = end.base().val();
            }
            
            // 检查冲突
            let mut conflict = false;
            for area in &process.address_space.areas {
                if area.end > start && area.start < end {
                    conflict = true;
                    break;
                }
            }
            if conflict {
                return -1;
            }

            if fd == GPU_FD as _ {
                let gpu = DEVICES.get().unwrap().get_gpu().unwrap();
                // Check that len completely covers gpu framebuffer
                if len != gpu.resolution().map(|res| res.0 * res.1 * 4).unwrap() as _ {
                    log::error!("len does not completely cover gpu framebuffer");
                    return -1;
                }

                let fb = gpu.get_framebuffer().unwrap();
                let fb_ptr = fb.as_ptr() as usize;
                // check that physical framebuffer is page aligned
                assert!((fb_ptr & ((1 << Sv39::PAGE_BITS) - 1)) == 0);
                let fb_ppn = PPN::new(fb_ptr >> Sv39::PAGE_BITS);
                
                process.address_space.map_extern(start..end, fb_ppn, flags);
                start.base().val() as _
            } else {
                process.address_space.map(start..end, &[], 0, flags);
                start.base().val() as _
            }
        }

        fn munmap(&self, caller: Caller, addr: usize, len: usize) -> isize {
            if addr % 4096 != 0 {
                return -1;
            }

            let start = VAddr::<Sv39>::new(addr).floor();
            let end = VAddr::<Sv39>::new(addr + len).ceil();
            
            let process = unsafe { PROCESSES.get_mut() }.get_mut(caller.entity).unwrap();
            
            // 检查这个区域是否已经映射？必须完全位于某个已有的 area 内部吗？
            // 测试用例要求“尝试 unmap 没有映射的内存”返回 -1
            let mut mapped = false;
            for area in &process.address_space.areas {
                if start >= area.start && end <= area.end {
                    mapped = true;
                    break;
                }
            }

            if !mapped {
                return -1;
            }

            process.address_space.unmap(start..end);
            0
        }
    }
}

/// 非 RISC-V64 架构的占位模块。
///
/// 提供编译所需的符号和类型，使得 `cargo publish --dry-run` 在主机平台上能通过编译。
#[cfg(not(target_arch = "riscv64"))]
mod stub {
    use tg_kernel_vm::page_table::{MmuMeta, VmFlags};

    /// Sv39 占位类型：在主机平台上模拟 Sv39 的参数
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
    pub struct Sv39;

    impl MmuMeta for Sv39 {
        const P_ADDR_BITS: usize = 56;
        const PAGE_BITS: usize = 12;
        const LEVEL_BITS: &'static [usize] = &[9, 9, 9];
        const PPN_POS: usize = 10;

        #[inline]
        fn is_leaf(value: usize) -> bool {
            value & 0b1110 != 0
        }
    }

    /// 构建 VmFlags 占位
    pub const fn build_flags(_s: &str) -> VmFlags<Sv39> {
        unsafe { VmFlags::from_raw(0) }
    }

    /// 解析 VmFlags 占位
    pub fn parse_flags(_s: &str) -> Result<VmFlags<Sv39>, ()> {
        Ok(unsafe { VmFlags::from_raw(0) })
    }

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
