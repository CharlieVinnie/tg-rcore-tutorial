//! # 第六章：文件系统
//!
//! 本章在第五章"进程管理"的基础上，引入了 **文件系统** 支持。
//! 用户程序不再嵌入内核镜像，而是存放在 **磁盘镜像**（fs.img）中，
//! 内核通过 **VirtIO 块设备驱动** 和 **easy-fs 文件系统** 按名称加载和执行程序。
//!
//! ## 核心概念
//!
//! - **文件系统（easy-fs）**：简单的类 UNIX inode 文件系统，支持单级目录
//! - **块设备驱动（VirtIO-blk）**：通过 MMIO 访问虚拟块设备
//! - **文件描述符表**：每个进程维护 fd_table，统一管理标准 I/O 和普通文件
//! - **文件操作系统调用**：open、close、read、write
//!
//! ## 与第五章的区别
//!
//! | 特性 | 第五章 | 第六章 |
//! |------|--------|--------|
//! | 程序存储 | 嵌入内核镜像（APP_ASM） | 磁盘镜像（fs.img） |
//! | 程序加载 | 按名称查内存表 | 通过文件系统 open + read |
//! | I/O 方式 | 仅 SBI 控制台 | 文件描述符表 + 文件句柄 |
//! | 块设备 | 无 | VirtIO-blk 驱动 |
//! | QEMU 参数 | 无磁盘 | 挂载 fs.img 块设备 |
//!
//! 教程阅读建议：
//!
//! - 先看 `rust_main`：掌握“内核初始化 -> 文件系统启动 -> initproc 加载”的主线；
//! - 再看 `kernel_space`：理解 MMIO 与普通内存映射的差异；
//! - 最后看 `impls`：理解系统调用如何经由 fd_table 访问文件系统。

// 不使用标准库，裸机环境没有操作系统提供系统调用支持
#![no_std]
// 不使用默认的 main 函数入口，裸机环境需要自定义入口点
#![no_main]
// 在 RISC-V 架构上启用严格的编译警告和文档要求
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
// 在非 RISC-V 架构上允许未使用的代码（用于 IDE 开发体验）
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code, unused_imports))]

mod device;
mod file;
/// 文件系统模块：easy-fs 文件系统管理器
mod fs;
mod memory;
/// 进程模块：定义 Process 结构体（含文件描述符表）
mod process;
/// 处理器模块：定义 PROCESSOR 全局变量和进程管理器
mod processor;
mod user_reader;
/// VirtIO 块设备驱动模块
mod virtio_block;

#[macro_use]
extern crate tg_console;

#[macro_use]
extern crate alloc;

use crate::{
    device::{init_devices, DEVICES},
    fs::{read_all, FS},
    impls::{Console, SyscallContext},
    memory::{Sv39Manager, KERNEL_SPACE},
    process::Process,
    processor::{ProcManager, PROCESSOR},
};
use alloc::alloc::alloc;
use core::alloc::Layout;
use riscv::register::*;
#[cfg(not(target_arch = "riscv64"))]
use stub::Sv39;
use tg_console::log;
use tg_driver::visit_virtio_ranges;
use tg_easy_fs::{FSManager, OpenFlags};
use tg_kernel_context::foreign::MultislotPortal;
#[cfg(target_arch = "riscv64")]
pub use tg_kernel_vm::page_table::Sv39;
use tg_kernel_vm::{
    page_table::{MmuMeta, VAddr, VmFlags, VmMeta, PPN, VPN},
    AddressSpace, MapVisibility,
};
use tg_sbi;
use tg_syscall::Caller;
use tg_task_manage::{PManager, ProcId};
use xmas_elf::ElfFile;

/// 构建 VmFlags（虚拟内存标志位）。
#[cfg(target_arch = "riscv64")]
pub const fn build_flags(s: &str) -> VmFlags<Sv39> {
    VmFlags::build_from_str(s)
}

/// 运行时解析 VmFlags 字符串。
#[cfg(target_arch = "riscv64")]
fn parse_flags(s: &str) -> Result<VmFlags<Sv39>, ()> {
    s.parse()
}

#[cfg(not(target_arch = "riscv64"))]
pub use stub::{build_flags, parse_flags};

// 定义内核入口点，设置启动栈大小为 32 页 = 128 KiB。
#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 32 * 4096;
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

/// 物理内存容量 = 48 MiB
const MEMORY: usize = 48 << 20;

/// 异界传送门所在虚页（虚拟地址空间最高页）
const PROTAL_TRANSIT: VPN<Sv39> = VPN::MAX;

/// 内核主函数——系统初始化和启动入口
///
/// 执行流程：
/// 1. 清零 BSS 段
/// 2. 初始化控制台和日志系统
/// 3. 初始化内核堆分配器
/// 4. 分配并创建异界传送门
/// 5. 建立内核地址空间（恒等映射 + MMIO 映射 + 传送门映射），激活 Sv39 分页
/// 6. 初始化外设和异界传送门
/// 7. 初始化系统调用处理器
/// 8. 从文件系统加载初始进程 `initproc`，进入调度循环
extern "C" fn rust_main() -> ! {
    let layout = tg_linker::KernelLayout::locate();
    // 步骤 1：清零 BSS 段
    unsafe { layout.zero_bss() };
    // 步骤 2：初始化控制台输出和日志系统
    tg_console::init_console(&Console);
    tg_console::set_log_level(option_env!("LOG"));
    tg_console::test_log();
    // 步骤 3：初始化内核堆分配器
    tg_kernel_alloc::init(layout.start() as _);
    unsafe {
        tg_kernel_alloc::transfer(core::slice::from_raw_parts_mut(
            layout.end() as _,
            MEMORY - layout.len(),
        ))
    };
    // 步骤 4：分配异界传送门所需的物理页面
    let portal_size = MultislotPortal::calculate_size(1);
    let portal_layout = Layout::from_size_align(portal_size, 1 << Sv39::PAGE_BITS).unwrap();
    let portal_ptr = unsafe { alloc(portal_layout) };
    assert!(portal_layout.size() < 1 << Sv39::PAGE_BITS);
    // 步骤 5：建立内核地址空间并激活 Sv39 分页（包含 MMIO 映射）
    kernel_space(layout, MEMORY, portal_ptr as _);
    // 初始化外围设备（GPU，Keyboard）
    init_devices();
    // 步骤 6：初始化异界传送门
    let portal = unsafe { MultislotPortal::init_transit(PROTAL_TRANSIT.base().val(), 1) };
    // 步骤 7：初始化系统调用处理器
    tg_syscall::init_io(&SyscallContext);
    tg_syscall::init_process(&SyscallContext);
    tg_syscall::init_scheduling(&SyscallContext);
    tg_syscall::init_clock(&SyscallContext);
    tg_syscall::init_memory(&SyscallContext);
    
    // 步骤 8：从文件系统加载初始进程 initproc
    const INITPROC: &str = env!("INITPROC");
    let initproc = read_all(
        FS.open(INITPROC, OpenFlags::RDONLY)
            .expect(alloc::format!("INITPROC {INITPROC} is not found").as_str()),
    );
    if let Some(process) = Process::from_elf(ElfFile::new(initproc.as_slice()).unwrap()) {
        PROCESSOR.get_mut().set_manager(ProcManager::new());
        PROCESSOR
            .get_mut()
            .add(process.pid, process, ProcId::from_usize(usize::MAX));
    }

    // ─── 主调度循环 ───
    loop {
        let processor: *mut PManager<Process, ProcManager> = PROCESSOR.get_mut() as *mut _;
        if let Some(task) = unsafe { (*processor).find_next() } {
            // 通过异界传送门切换到用户地址空间执行用户程序
            unsafe { task.context.execute(portal, ()) };

            // ─── Trap 返回后处理 ───
            match scause::read().cause() {
                // ─── 系统调用（ecall 指令触发） ───
                scause::Trap::Exception(scause::Exception::UserEnvCall) => {
                    use tg_syscall::{SyscallId as Id, SyscallResult as Ret};
                    let ctx = &mut task.context.context;
                    ctx.move_next();
                    let id: Id = ctx.a(7).into();
                    let args = [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)];
                    match tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
                        Ret::Done(ret) => match id {
                            Id::EXIT => unsafe { (*processor).make_current_exited(ret) },
                            _ => {
                                let ctx = &mut task.context.context;
                                *ctx.a_mut(0) = ret as _;
                                unsafe { (*processor).make_current_suspend() };
                            }
                        },
                        Ret::Unsupported(_) => {
                            log::info!("id = {id:?}");
                            unsafe { (*processor).make_current_exited(-2) };
                        }
                    }
                }
                scause::Trap::Interrupt(scause::Interrupt::SupervisorExternal) => {
                    DEVICES.get().unwrap().handle_external_interrupt();
                    unsafe { (*processor).make_current_suspend() };
                }
                // ─── 其他异常/中断：杀死进程 ───
                e => {
                    log::error!("unsupported trap: {e:?}");
                    unsafe { (*processor).make_current_exited(-3) };
                }
            }
        } else {
            println!("no task");
            break;
        }
    }

    tg_sbi::shutdown(false)
}

/// Rust panic 处理函数，打印错误信息并以异常方式关机
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    tg_sbi::shutdown(true)
}

/// 建立内核地址空间
///
/// 内核使用**恒等映射**（Identity Mapping）：虚拟地址 == 物理地址。
fn kernel_space(layout: tg_linker::KernelLayout, memory: usize, portal: usize) {
    let mut space = AddressSpace::new();
    // 映射内核各段（恒等映射：VPN == PPN）
    for region in layout.iter() {
        log::info!("{region}");
        use tg_linker::KernelRegionTitle::*;
        let flags = match region.title {
            Text => "X_RV",       // 代码段：可执行、可读
            Rodata => "__RV",     // 只读数据：可读
            Data | Boot => "_WRV", // 数据段：可写、可读
        };
        let s = VAddr::<Sv39>::new(region.range.start);
        let e = VAddr::<Sv39>::new(region.range.end);
        space.map_extern(
            s.floor()..e.ceil(),
            PPN::new(s.floor().val()),
            build_flags(flags),
            MapVisibility::PRIVATE,
        )
    }
    // 映射堆区域
    let s = VAddr::<Sv39>::new(layout.end());
    let e = VAddr::<Sv39>::new(layout.start() + memory);
    log::info!("(heap) ---> {:#10x}..{:#10x}", s.val(), e.val());
    space.map_extern(
        s.floor()..e.ceil(),
        PPN::new(s.floor().val()),
        build_flags("_WRV"),
        MapVisibility::PRIVATE,
    );
    // 映射异界传送门页面
    space.map_extern(
        PROTAL_TRANSIT..PROTAL_TRANSIT + 1,
        PPN::new(portal >> Sv39::PAGE_BITS),
        build_flags("__G_XWRV"),
        MapVisibility::PRIVATE,
    );
    println!();
    
    // 映射所有由设备树扫描出的 VirtIO 设备
    visit_virtio_ranges(|start, end| {
        assert!(start.trailing_zeros() >= Sv39::PAGE_BITS as _);
        assert!(end.trailing_zeros() >= Sv39::PAGE_BITS as _);
        space.map_extern(
            VPN::new(start >> Sv39::PAGE_BITS)..VPN::new(end >> Sv39::PAGE_BITS),
            PPN::new(start >> Sv39::PAGE_BITS),
            build_flags("_WRV"),
            MapVisibility::PRIVATE,
        );
    });

    // 激活 Sv39 分页模式
    unsafe { satp::set(satp::Mode::Sv39, 0, space.root_ppn().val()) };
    // 保存内核地址空间到全局变量
    KERNEL_SPACE.init(space);
}

/// 将内核地址空间中的异界传送门页表项复制到用户地址空间
fn map_portal(space: &AddressSpace<Sv39, Sv39Manager>) {
    let portal_idx = PROTAL_TRANSIT.index_in(Sv39::MAX_LEVEL);
    space.root()[portal_idx] = KERNEL_SPACE.get().root()[portal_idx];
}

/// 各种接口库的实现
mod impls {
    use crate::{
        Sv39, build_flags, device::DEVICES, file::{DiskFile, File, GPUFile, InputDevFile}, fs::{FS, read_all}, memory::{Sv39Manager, VmMapperSv39}, process::Process as ProcStruct, processor::{PROCESSOR, ProcManager}, user_reader::read_list
    };
    use alloc::{sync::Arc, vec::Vec};
    use core::ptr::NonNull;
    use spin::Mutex;
    use tg_console::log;
    use tg_easy_fs::{FSManager, OpenFlags, UserBuffer};
    use tg_kernel_vm::{
        page_table::{VAddr, VmFlags},
        AddressSpace, MapVisibility,
    };
    use tg_syscall::*;
    use tg_task_manage::{PManager, ProcId};
    use xmas_elf::ElfFile;

    /// 控制台输出实现，通过 SBI 接口逐字符输出
    pub struct Console;

    impl tg_console::Console for Console {
        #[inline]
        fn put_char(&self, c: u8) {
            tg_sbi::console_putchar(c);
        }
    }

    /// 系统调用上下文
    pub struct SyscallContext;

    const READABLE: VmFlags<Sv39> = build_flags("URV");
    const WRITABLE: VmFlags<Sv39> = build_flags("UWRV");

    fn current_address_space() -> &'static mut AddressSpace<Sv39, Sv39Manager> {
        &mut PROCESSOR.get_mut().current().unwrap().address_space
    }

    fn translate_current<T>(addr: usize, flags: VmFlags<Sv39>) -> Option<NonNull<T>> {
        current_address_space().translate::<T>(VAddr::new(addr), flags)
    }

    /// IO 系统调用实现：read、write、open、close 等
    impl IO for SyscallContext {
        fn write(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            if let Some(ptr) = translate_current::<u8>(buf, READABLE) {
                if fd == STDOUT || fd == STDDEBUG {
                    // 标准输出：直接打印到控制台
                    print!("{}", unsafe {
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                            ptr.as_ptr(),
                            count,
                        ))
                    });
                    count as _
                } else if let Some(file) = &PROCESSOR.get_mut().current().unwrap().fd_table[fd] {
                    // 统一分发：不管底层是什么文件，统统调用 write!
                    let file = file.lock();
                    let mut v: Vec<&'static mut [u8]> = Vec::new();
                    unsafe { v.push(core::slice::from_raw_parts_mut(ptr.as_ptr(), count)) };
                    file.write(UserBuffer::new(v)) as _
                } else {
                    log::error!("unsupported fd: {fd}");
                    -1
                }
            } else {
                log::error!("ptr not readable");
                -1
            }
        }

        fn read(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            if let Some(mut ptr) = translate_current::<u8>(buf, WRITABLE) {
                if fd == STDIN {
                    // 标准输入：通过 SBI 逐字符读取
                    let mut ptr = unsafe { ptr.as_mut() } as *mut u8;
                    for _ in 0..count {
                        unsafe {
                            *ptr = tg_sbi::console_getchar() as u8;
                            ptr = ptr.add(1);
                        }
                    }
                    count as _
                } else if let Some(file) = &PROCESSOR.get_mut().current().unwrap().fd_table[fd] {
                    // 统一分发：键盘输入和磁盘读取现在的代码路径完全一致！
                    let file = file.lock();
                    let mut v: Vec<&'static mut [u8]> = Vec::new();
                    unsafe { v.push(core::slice::from_raw_parts_mut(ptr.as_ptr(), count)) };
                    file.read(UserBuffer::new(v)) as _
                } else {
                    log::error!("unsupported fd: {fd}");
                    -1
                }
            } else {
                log::error!("ptr not writeable");
                -1
            }
        }

        fn open(&self, _caller: Caller, path: usize, count: usize, flags: usize) -> isize {
            let Ok(path_slice) = read_list(current_address_space(), path, count) else {
                log::error!("path not readable");
                return -1;
            };
            let path_str = unsafe { core::str::from_utf8_unchecked(path_slice) };

            let current = PROCESSOR.get_mut().current().unwrap();
            let new_fd = current.fd_table.len();

            let file: Arc<dyn File>;

            if path_str == "/dev/fb0" {
                file = Arc::new(GPUFile::new(DEVICES.get().unwrap().get_gpu().unwrap()));
            } else if path_str == "/dev/input0" {
                file = Arc::new(InputDevFile::new(DEVICES.get().unwrap().get_keyboard().unwrap()));
            } else if let Some(file_handle) = FS.open(path_str, OpenFlags::from_bits(flags as u32).unwrap()) {
                file = Arc::new(DiskFile::new(file_handle));
            } else {
                return -1;
            }

            // 存入文件描述符表
            current.fd_table.push(Some(Mutex::new(file)));
            new_fd as isize
        }

        #[inline]
        fn close(&self, _caller: Caller, fd: usize) -> isize {
            let current = PROCESSOR.get_mut().current().unwrap();
            if fd >= current.fd_table.len() || current.fd_table[fd].is_none() {
                return -1;
            }
            current.fd_table[fd].take();
            0
        }

        fn ioctl(&self, _caller: tg_syscall::Caller, fd: usize, request: usize, argp: usize) -> isize {
            let current = PROCESSOR.get_mut().current().unwrap();
            if fd >= current.fd_table.len() {
                return -1;
            }
            
            if let Some(file) = &current.fd_table[fd] {
                let file = file.lock();
                file.ioctl(request, argp)
            } else {
                log::error!("unsupported fd: {fd}");
                -1
            }
        }
    }

    /// 进程管理系统调用实现
    impl Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: Caller, exit_code: usize) -> isize {
            exit_code as isize
        }

        fn fork(&self, _caller: Caller) -> isize {
            let processor: *mut PManager<ProcStruct, ProcManager> = PROCESSOR.get_mut() as *mut _;
            let current = unsafe { (*processor).current().unwrap() };
            let parent_pid = current.pid;
            let mut child_proc = current.fork().unwrap();
            let pid = child_proc.pid;
            let context = &mut child_proc.context.context;
            *context.a_mut(0) = 0 as _;
            unsafe {
                (*processor).add(pid, child_proc, parent_pid);
            }
            pid.get_usize() as isize
        }

        fn exec(&self, _caller: Caller, path: usize, count: usize) -> isize {
            let Ok(path_slice) = read_list(current_address_space(), path, count) else {
                return -1;
            };
            let name = unsafe { core::str::from_utf8_unchecked(path_slice) };
            if let Some(fd) = FS.open(name, OpenFlags::RDONLY) {
                let current = PROCESSOR.get_mut().current().unwrap();
                current.exec(ElfFile::new(&read_all(fd)).unwrap());
                0
            } else {
                log::error!("unknown app, select one in the list: ");
                FS.readdir("").unwrap().into_iter().for_each(|app| println!("{app}"));
                println!();
                -1
            }
        }

        fn wait(&self, _caller: Caller, pid: isize, exit_code_ptr: usize) -> isize {
            let processor: *mut PManager<ProcStruct, ProcManager> = PROCESSOR.get_mut() as *mut _;
            let current = unsafe { (*processor).current().unwrap() };
            if let Some((dead_pid, exit_code)) =
                unsafe { (*processor).wait(ProcId::from_usize(pid as usize)) }
            {
                if let Some(mut ptr) = current
                    .address_space
                    .translate::<i32>(VAddr::new(exit_code_ptr), WRITABLE)
                {
                    unsafe { *ptr.as_mut() = exit_code as i32 };
                }
                return dead_pid.get_usize() as isize;
            } else {
                return -1;
            }
        }

        fn getpid(&self, _caller: Caller) -> isize {
            let current = PROCESSOR.get_mut().current().unwrap();
            current.pid.get_usize() as _
        }

        fn spawn(&self, _caller: Caller, path: usize, count: usize) -> isize {
            let processor: *mut PManager<ProcStruct, ProcManager> = PROCESSOR.get_mut() as *mut _;
            let current = unsafe { (*processor).current().unwrap() };
            
            if let Ok(path_slice) = read_list(&current.address_space, path, count) {
                let name = unsafe { core::str::from_utf8_unchecked(path_slice) };
                if let Some(fd) = FS.open(name, OpenFlags::RDONLY) {
                    if let Ok(elf) = ElfFile::new(&read_all(fd)) {
                        if let Some(child_proc) = ProcStruct::from_elf(elf) {
                            let pid = child_proc.pid;
                            let parent_pid = current.pid;
                            unsafe { (*processor).add(pid, child_proc, parent_pid) };
                            return pid.get_usize() as isize;
                        }
                    }
                }
            }
            -1
        }

        fn sbrk(&self, _caller: Caller, size: i32) -> isize {
            let current = PROCESSOR.get_mut().current().unwrap();
            if let Some(old_brk) = current.change_program_brk(size as isize) {
                old_brk as isize
            } else {
                -1
            }
        }
    }

    /// 调度系统调用实现
    impl Scheduling for SyscallContext {
        #[inline]
        fn sched_yield(&self, _caller: Caller) -> isize {
            0
        }

        fn set_priority(&self, _caller: Caller, prio: isize) -> isize {
            if prio < 2 {
                return -1;
            }
            let current = PROCESSOR.get_mut().current().unwrap();
            current.priority = prio as usize;
            prio
        }
    }

    /// 时钟系统调用实现
    impl Clock for SyscallContext {
        #[inline]
        fn clock_gettime(&self, _caller: Caller, clock_id: ClockId, tp: usize) -> isize {
            match clock_id {
                ClockId::CLOCK_MONOTONIC => {
                    if let Some(mut ptr) = translate_current::<TimeSpec>(tp, WRITABLE) {
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

    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;
    const MAP_PRIVATE: i32 = 0x1;
    const MAP_SHARED: i32 = 0x2;
    const MAP_ANONYMOUS: i32 = 0x20;

    /// 内存管理系统调用实现
    impl Memory for SyscallContext {
        fn mmap(
            &self,
            _caller: Caller,
            addr: usize,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            _offset: usize,
        ) -> isize {
            if addr % 4096 != 0 {
                return -1;
            }
            if len == 0 {
                return -1;
            }

            let visibility = match flags & (MAP_PRIVATE | MAP_SHARED) {
                MAP_PRIVATE => MapVisibility::PRIVATE,
                MAP_SHARED => MapVisibility::SHARED,
                _ => return -1,
            };
            let mut prot_bytes = [b'U', b'_', b'_', b'_', b'V'];
            if prot & PROT_READ != 0 {
                prot_bytes[3] = b'R';
            }
            if prot & PROT_WRITE != 0 {
                prot_bytes[2] = b'W';
            }
            if prot & PROT_EXEC != 0 {
                prot_bytes[1] = b'X';
            }
            let prot_str = unsafe { core::str::from_utf8_unchecked(&prot_bytes) };
            let prot_flags = build_flags(prot_str);
            
            let process = PROCESSOR.get_mut().current().unwrap();

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
                if area.end() > start && area.start() < end {
                    conflict = true;
                    break;
                }
            }
            if conflict {
                return -1;
            }

            if (flags & MAP_ANONYMOUS) == MAP_ANONYMOUS {
                process.address_space.map(start..end, &[], 0, prot_flags, visibility);
                start.base().val() as _
            } else {
                let file = process.fd_table[fd as usize].as_ref().unwrap().lock();
                let mut mapper = VmMapperSv39::new(start, end, prot_flags, visibility, &mut process.address_space);
                file.mmap(&mut mapper).map(|_| start.base().val() as _).unwrap_or(-1)
            }
        }

        fn munmap(&self, _caller: Caller, addr: usize, len: usize) -> isize {
            if addr % 4096 != 0 {
                return -1;
            }

            let start = VAddr::<Sv39>::new(addr).floor();
            let end = VAddr::<Sv39>::new(addr + len).ceil();
            
            let process = PROCESSOR.get_mut().current().unwrap();
            
            let mut mapped = false;
            for area in &process.address_space.areas {
                if start >= area.start() && end <= area.end() {
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

/// 非 RISC-V64 架构的占位实现
#[cfg(not(target_arch = "riscv64"))]
mod stub {
    use tg_kernel_vm::page_table::{MmuMeta, VmFlags};

    /// Sv39 占位类型
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

    /// 构建 VmFlags 占位实现
    pub const fn build_flags(_s: &str) -> VmFlags<Sv39> {
        unsafe { VmFlags::from_raw(0) }
    }

    /// 解析 VmFlags 占位实现
    pub fn parse_flags(_s: &str) -> Result<VmFlags<Sv39>, ()> {
        Ok(unsafe { VmFlags::from_raw(0) })
    }

    /// 主机平台占位入口
    #[unsafe(no_mangle)]
    pub extern "C" fn main() -> i32 {
        0
    }

    /// libc 启动占位
    #[unsafe(no_mangle)]
    pub extern "C" fn __libc_start_main() -> i32 {
        0
    }

    /// 异常处理占位
    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}
}