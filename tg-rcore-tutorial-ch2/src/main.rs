//! Runs ch2 game: incremental tangram picture of OS
#![no_std]
#![no_main]
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code))]

#[macro_use]
extern crate tg_console;

use impls::{Console, SyscallContext};
use riscv::register::*;
use tg_console::log;
use tg_kernel_context::LocalContext;
use tg_sbi::{self, console_getchar};
use tg_syscall::{Caller, SyscallId};

use spin::Once;
use buddy_system_allocator::LockedHeap;
use tg_driver::{DeviceManager};
use virtio_drivers::{Hal};

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::empty();

const HEAP_SIZE: usize = 0x20000;
#[repr(align(4096))]
struct HeapSpace([u8; HEAP_SIZE]);
static mut HEAP_SPACE: HeapSpace = HeapSpace([0; HEAP_SIZE]);

fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP_SPACE.0) as usize, HEAP_SIZE);
    }
}

struct HalImpl;
impl Hal for HalImpl {
    fn dma_alloc(pages: usize) -> usize {
        #[repr(align(4096))]
        #[allow(dead_code)]
        struct DmaBuffer([u8; 1024 * 1024 * 2]);
        #[allow(dead_code)]
        static mut DMA_BUF: DmaBuffer = DmaBuffer([0; 1024 * 1024 * 2]);
        static mut OFFSET: usize = 0;
        unsafe {
            let base = core::ptr::addr_of_mut!(DMA_BUF) as usize;
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

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!(env!("APP_ASM")));

#[cfg(target_arch = "riscv64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
unsafe extern "C" fn _start() -> ! {
    const STACK_SIZE: usize = 8 * 4096;
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

extern "C" fn rust_main() -> ! {
    unsafe { tg_linker::KernelLayout::locate().zero_bss() };

    unsafe extern "C"  { fn __end(); }
    
    tg_console::init_console(&Console);
    tg_console::set_log_level(option_env!("LOG"));
    tg_console::test_log();
    
    println!("End of kernel is {:#x}", __end as *const () as usize);

    init_heap();
    
    DEVICES.call_once(DeviceManager::new::<HalImpl>);

    tg_syscall::init_io(&SyscallContext);
    tg_syscall::init_process(&SyscallContext);
    tg_syscall::init_memory(&SyscallContext);

    for (i, app) in tg_linker::AppMeta::locate().iter().enumerate() {
        let app_base = app.as_ptr() as usize;
        log::info!("load app{i} to {app_base:#x}");

        let mut ctx = LocalContext::user(app_base);

        let mut user_stack: core::mem::MaybeUninit<[usize; 512]> =
            core::mem::MaybeUninit::uninit();
        let user_stack_ptr = user_stack.as_mut_ptr() as *mut usize;
        *ctx.sp_mut() = unsafe { user_stack_ptr.add(512) } as usize;

        loop {
            unsafe { ctx.execute() };

            use scause::{Exception, Trap};
            match scause::read().cause() {
                Trap::Exception(Exception::UserEnvCall) => {
                    use SyscallResult::*;
                    match handle_syscall(&mut ctx) {
                        Done => continue,
                        Exit(code) => {
                            log::info!("app{i} exit with code {code}");
                        }
                        Error(id) => {
                            log::error!("app{i} call an unsupported syscall {:?}", id)
                        }
                    }
                }
                trap => log::error!("app{i} was killed because of {trap:?}"),
            }
            unsafe { core::arch::asm!("fence.i") };
            break;
        }
        let _ = core::hint::black_box(&user_stack);
        println!();
    }

    println!("Press any key to exit...");
    console_getchar();

    tg_sbi::shutdown(false)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    tg_sbi::shutdown(true)
}

enum SyscallResult {
    Done,
    Exit(usize),
    Error(SyscallId),
}

fn handle_syscall(ctx: &mut LocalContext) -> SyscallResult {
    use tg_syscall::{SyscallId as Id, SyscallResult as Ret};

    let id = ctx.a(7).into();
    let args = [ctx.a(0), ctx.a(1), ctx.a(2), ctx.a(3), ctx.a(4), ctx.a(5)];

    match tg_syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
        Ret::Done(ret) => match id {
            Id::EXIT => SyscallResult::Exit(ctx.a(0)),
            _ => {
                *ctx.a_mut(0) = ret as _;
                ctx.move_next();
                SyscallResult::Done
            }
        },
        Ret::Unsupported(id) => SyscallResult::Error(id),
    }
}

mod impls {
    use tg_syscall::{STDDEBUG, STDOUT};

    pub struct Console;

    impl tg_console::Console for Console {
        #[inline]
        fn put_char(&self, c: u8) {
            tg_sbi::console_putchar(c);
        }
    }

    pub struct SyscallContext;

    impl tg_syscall::IO for SyscallContext {
        fn write(
            &self,
            _caller: tg_syscall::Caller,
            fd: usize,
            buf: usize,
            count: usize,
        ) -> isize {
            match fd {
                STDOUT | STDDEBUG => {
                    print!("{}", unsafe {
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                            buf as *const u8,
                            count,
                        ))
                    });
                    count as _
                }
                _ => {
                    tg_console::log::error!("unsupported fd: {fd}");
                    -1
                }
            }
        }

        fn open(&self, _caller: tg_syscall::Caller, _path: usize, _flags: usize) -> isize {
            3
        }

        fn ioctl(&self, _caller: tg_syscall::Caller, _fd: usize, request: usize, _argp: usize) -> isize {
            if request == 1 { // FB_FLUSH
                if let Some(gpu) = crate::DEVICES.get().unwrap().get_gpu() {
                    gpu.flush();
                }
            }
            0
        }
    }

    impl tg_syscall::Memory for SyscallContext {
        fn mmap(
            &self,
            _caller: tg_syscall::Caller,
            _addr: usize,
            _length: usize,
            _prot: i32,
            _flags: i32,
            _fd: i32,
            _offset: usize,
        ) -> isize {
            if let Some(gpu) = crate::DEVICES.get().unwrap().get_gpu() {
                gpu.get_framebuffer().as_mut_ptr() as isize
            } else {
                -1
            }
        }
        
        fn munmap(&self, _caller: tg_syscall::Caller, _addr: usize, _length: usize) -> isize {
            0
        }
    }

    impl tg_syscall::Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: tg_syscall::Caller, _status: usize) -> isize {
            0
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
mod stub {
    #[unsafe(no_mangle)]
    pub extern "C" fn main() -> i32 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn __libc_start_main() -> i32 {
        0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn rust_eh_personality() {}
}
