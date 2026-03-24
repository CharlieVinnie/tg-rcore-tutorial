//! 用于 `-bios none` 启动的最小 M-Mode SBI 实现。
//!
//! 本模块在没有外部引导程序（如 RustSBI）的情况下提供基本的 SBI 服务。
//! 它处理来自 S-mode 的 ecall 并提供：
//! - 控制台 I/O（UART）
//! - 定时器管理
//! - 系统重置
//!
//! 设计定位：这是“够用即可”的教学最小实现，
//! 只覆盖 ch1~ch8 需要的 SBI 功能，不追求完整 SBI 规范实现。

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

const UART_BASE: usize = 0x1000_0000;
// 说明：该地址是 QEMU virt 机器常用 UART MMIO 基址。
// 本实现假定运行环境与教学配置一致（单核 + QEMU virt）。

/// UART 操作（16550 兼容）。
mod uart {
    use super::UART_BASE;

    const THR: usize = UART_BASE; // 发送保持寄存器（与接收寄存器共享偏移 0）
    const LSR: usize = UART_BASE + 5; // 线路状态寄存器

    /// 检查 UART 是否准备好发送。
    #[inline]
    fn is_tx_ready() -> bool {
        // SAFETY: 从 QEMU virt 机器的已知 MMIO 地址读取 UART LSR 寄存器。
        // 这是只读操作，用于检查 THRE（发送保持寄存器空）位。
        unsafe {
            let lsr = (LSR as *const u8).read_volatile();
            (lsr & 0x20) != 0 // THRE bit
        }
    }

    /// 向 UART 写入一个字节。
    pub fn putchar(c: u8) {
        while !is_tx_ready() {}
        // SAFETY: 向 QEMU virt 机器的已知 MMIO 地址写入 UART THR 寄存器。
        // 我们已通过 is_tx_ready() 验证 UART 已准备好。
        unsafe {
            (THR as *mut u8).write_volatile(c);
        }
    }

    /// 从 UART 读取一个字节（非阻塞）。
    pub fn getchar() -> Option<u8> {
        // SAFETY: 从 QEMU virt 机器的已知 MMIO 地址读取 UART LSR 寄存器，
        // 检查数据就绪状态。
        let lsr = unsafe { (LSR as *const u8).read_volatile() };
        if lsr & 1 != 0 {
            // SAFETY: 读取偏移 0 的数据寄存器（16550 语义下即接收缓冲区 RBR）。
            // 代码中沿用 THR 常量名，是因为读写共用同一偏移地址。
            Some(unsafe { (THR as *const u8).read_volatile() })
        } else {
            None
        }
    }
}

/// SBI 扩展 ID。
mod eid {
    pub const CONSOLE_PUTCHAR: usize = 0x01;
    pub const CONSOLE_GETCHAR: usize = 0x02;
    pub const SHUTDOWN: usize = 0x08;
    pub const BASE: usize = 0x10;
    pub const SRST: usize = 0x53525354;
    pub const TIMER: usize = 0x54494D45;
    pub const HSM: usize = 0x48534D;
}

/// SBI 功能 ID。
mod fid {
    pub const BASE_GET_SBI_VERSION: usize = 0;
    pub const BASE_GET_IMPL_ID: usize = 1;
    pub const BASE_GET_IMPL_VERSION: usize = 2;
    pub const BASE_PROBE_EXTENSION: usize = 3;
    pub const BASE_GET_MVENDORID: usize = 4;
    pub const BASE_GET_MARCHID: usize = 5;
    pub const BASE_GET_MIMPID: usize = 6;

    pub const SRST_SHUTDOWN: usize = 0;
    #[allow(dead_code)]
    pub const SRST_COLD_REBOOT: usize = 1;
    #[allow(dead_code)]
    pub const SRST_WARM_REBOOT: usize = 2;

    // HSM FIDs
    pub const HSM_HART_START: usize = 0;
}

/// SBI 错误码。
mod error {
    pub const SUCCESS: isize = 0;
    pub const ERR_NOT_SUPPORTED: isize = -2;
    pub const ERR_INVALID_PARAM: isize = -3;
    pub const ERR_ALREADY_AVAILABLE: isize = -6;
}

/// SBI 返回值。
#[repr(C)]
pub struct SbiRet {
    /// 错误码。
    pub error: isize,
    /// 返回值。
    pub value: usize,
}

impl SbiRet {
    fn success(value: usize) -> Self {
        SbiRet {
            error: error::SUCCESS,
            value,
        }
    }

    fn not_supported() -> Self {
        SbiRet {
            error: error::ERR_NOT_SUPPORTED,
            value: 0,
        }
    }
}

static CONSOLE_LOCK: AtomicBool = AtomicBool::new(false);

/// 处理 Legacy 控制台 putchar（EID 0x01）。
fn handle_console_putchar(c: usize) -> SbiRet {
    while CONSOLE_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    uart::putchar(c as u8);
    CONSOLE_LOCK.store(false, Ordering::Release);
    SbiRet::success(0)
}

/// 处理 Legacy 控制台 getchar（EID 0x02）。
fn handle_console_getchar() -> SbiRet {
    // 简化实现：忙等直到收到字符。
    // 在多核场景下，我们仅在读取 UART 寄存器时获取锁，避免长时间持有阻碍其他核输出。
    loop {
        while CONSOLE_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        let res = uart::getchar();
        CONSOLE_LOCK.store(false, Ordering::Release);
        
        if let Some(c) = res {
            return SbiRet::success(c as usize);
        } else {
            core::hint::spin_loop();
        }
    }
}

/// 处理 Timer 扩展（EID 0x54494D45）。
fn handle_timer(time: u64) -> SbiRet {
    const CLINT_MTIMECMP: usize = 0x200_4000;
    // SAFETY: 向 QEMU virt 机器的已知 MMIO 地址写入 CLINT mtimecmp 寄存器。
    // 这将设置下一次定时器中断的触发时间。
    unsafe {
        (CLINT_MTIMECMP as *mut u64).write_volatile(time);
    }
    // 清除挂起的 S-mode 定时器中断（STIP），避免“旧中断状态”干扰下一次调度。
    // SAFETY: 修改 mip CSR 以清除 STIP 位是有效的 M-mode 操作。
    // 这是确认定时器中断所必需的。
    unsafe {
        asm!(
            "csrc mip, {}",
            in(reg) (1 << 5), // Clear STIP
        );
    }
    SbiRet::success(0)
}

const CLINT_MSIP_BASE: usize = 0x200_0000;
struct HartContext {
    start_addr: usize,
    opaque: usize,
    started: bool,
}
static mut HART_CONTEXTS: [HartContext; 8] = [
    HartContext { start_addr: 0, opaque: 0, started: false },
    HartContext { start_addr: 0, opaque: 0, started: false },
    HartContext { start_addr: 0, opaque: 0, started: false },
    HartContext { start_addr: 0, opaque: 0, started: false },
    HartContext { start_addr: 0, opaque: 0, started: false },
    HartContext { start_addr: 0, opaque: 0, started: false },
    HartContext { start_addr: 0, opaque: 0, started: false },
    HartContext { start_addr: 0, opaque: 0, started: false },
];

/// 处理 HSM 扩展：启动核心
fn handle_hsm(fid: usize, target_hart: usize, start_addr: usize, opaque: usize) -> SbiRet {
    if fid != fid::HSM_HART_START {
        return SbiRet::not_supported();
    }
    unsafe {
        if target_hart >= 8 {
            return SbiRet { error: error::ERR_INVALID_PARAM, value: 0 };
        }
        let ctx_ptr = core::ptr::addr_of_mut!(HART_CONTEXTS[target_hart]);
        if (*ctx_ptr).started {
            return SbiRet { error: error::ERR_ALREADY_AVAILABLE, value: 0 };
        }
        (*ctx_ptr).start_addr = start_addr;
        (*ctx_ptr).opaque = opaque;
        (*ctx_ptr).started = true;
        
        // 触发 IPI 唤醒 target_hart
        let msip_ptr = (CLINT_MSIP_BASE + target_hart * 4) as *mut u32;
        msip_ptr.write_volatile(1);
    }
    SbiRet::success(0)
}

/// 处理系统重置请求。
fn handle_system_reset(fid: usize) -> SbiRet {
    const VIRT_TEST: usize = 0x10_0000;
    const EXIT_SUCCESS: u32 = 0x5555;
    const EXIT_RESET: u32 = 0x3333;

    match fid {
        fid::SRST_SHUTDOWN => {
            // SAFETY: 向 QEMU virt test 设备的已知 MMIO 地址写入会触发系统关机。
            // 这是终止 QEMU virt 机器仿真的标准方式。
            unsafe {
                (VIRT_TEST as *mut u32).write_volatile(EXIT_SUCCESS);
            }
        }
        _ => {
            // SAFETY: 同上，但触发重置而非干净关机。
            unsafe {
                (VIRT_TEST as *mut u32).write_volatile(EXIT_RESET);
            }
        }
    }
    // 触发 reset/shutdown 后理论上不会返回，循环仅用于满足返回类型。
    loop {}
}

/// 处理 SBI Base 扩展调用。
fn handle_base(fid: usize) -> SbiRet {
    match fid {
        fid::BASE_GET_SBI_VERSION => SbiRet::success(0x01000000), // SBI v1.0.0
        fid::BASE_GET_IMPL_ID => SbiRet::success(0xFFFF),         // Custom implementation
        fid::BASE_GET_IMPL_VERSION => SbiRet::success(1),
        // 教学简化：统一返回 1，表示“支持该扩展”。
        // 在完整实现中应按 eid 逐项判断。
        fid::BASE_PROBE_EXTENSION => SbiRet::success(1),
        fid::BASE_GET_MVENDORID => SbiRet::success(0),
        fid::BASE_GET_MARCHID => SbiRet::success(0),
        fid::BASE_GET_MIMPID => SbiRet::success(0),
        _ => SbiRet::not_supported(),
    }
}

/// M-mode 陷阱发生时的上下文结构。
/// 用于保存和修改触发 M 态中断/异常时的寄存器状态。
#[repr(C)]
pub struct MachineTrapFrame {
    /// 返回地址 (x1)
    pub ra: usize,
    /// 临时寄存器 t0 (x5)
    pub t0: usize,
    /// 临时寄存器 t1 (x6)
    pub t1: usize,
    /// 临时寄存器 t2 (x7)
    pub t2: usize,
    /// 参数/返回值寄存器 a0 (x10)
    pub a0: usize,
    /// 参数/返回值寄存器 a1 (x11)
    pub a1: usize,
    /// 参数寄存器 a2 (x12)
    pub a2: usize,
    /// 参数寄存器 a3 (x13)
    pub a3: usize,
    /// 参数寄存器 a4 (x14)
    pub a4: usize,
    /// 参数寄存器 a5 (x15)
    pub a5: usize,
    /// 参数寄存器 a6 / fid (x16)
    pub a6: usize,
    /// 参数寄存器 a7 / eid (x17)
    pub a7: usize,
    /// 发生陷阱时的 PC 地址 (mepc)
    pub mepc: usize,
}

/// 从汇编调用的主 M-mode 陷阱处理程序。
#[unsafe(no_mangle)]
pub extern "C" fn m_trap_handler(frame: &mut MachineTrapFrame) {
    // 只处理 “S-mode ecall”：
    // - 这是 S 态内核调用 SBI 的标准入口
    // - 其余陷阱在本最小实现中统一视为不支持
    let mcause: usize;
    // SAFETY: 读取 mcause CSR 是有效的 M-mode 操作，它告诉我们陷阱的原因。
    // 我们需要此信息来验证这是一个 S-mode ecall。
    unsafe {
        core::arch::asm!("csrr {}, mcause", out(reg) mcause);
    }

    // 检查是否是机器级软件中断（即 MSIP 触发的 Wakeup IPI）
    let is_interrupt = (mcause as isize) < 0;
    let cause_code = mcause & !(1 << 63);
    if is_interrupt && cause_code == 3 {
        // 唤醒流程：
        // 1. 清除 IPI
        let hartid: usize;
        unsafe { asm!("csrr {}, mhartid", out(reg) hartid) };
        unsafe { ((CLINT_MSIP_BASE + hartid * 4) as *mut u32).write_volatile(0) };

        // 2. 配置 S 态寄存器为期望的 start_addr 和 opaque
        unsafe {
            let ctx_ptr = core::ptr::addr_of!(HART_CONTEXTS[hartid]);
            
            // 设置 S 态的起始地址
            frame.mepc = (*ctx_ptr).start_addr;
            
            // 设置 MPP = 01 (Supervisor) 以确保 mret 返回到 S 态
            // 必须先清空 11:12，然后再置为 01
            asm!(
                "csrc mstatus, {mask}",
                "csrs mstatus, {mpp}",
                mask = in(reg) (3 << 11),
                mpp = in(reg) (1 << 11),
            );

            // 让目标核心在唤醒时 a0 = hartid, a1 = opaque
            frame.a0 = hartid;
            frame.a1 = (*ctx_ptr).opaque;
        }
    }

    // 检查是否是 ecall 从 S 态发起
    if mcause == 9 {
        let eid = frame.a7;
        let fid = frame.a6;
        let a0_in = frame.a0;
        let a1_in = frame.a1;
        let a2_in = frame.a2;

        let ret = match eid {
            eid::CONSOLE_PUTCHAR => handle_console_putchar(a0_in),
            eid::CONSOLE_GETCHAR => handle_console_getchar(),
            eid::TIMER => handle_timer(a0_in as u64),
            eid::SHUTDOWN => handle_system_reset(fid::SRST_SHUTDOWN),
            eid::BASE => handle_base(fid),
            eid::SRST => handle_system_reset(fid),
            eid::HSM => handle_hsm(fid, a0_in, a1_in, a2_in),
            _ => SbiRet::not_supported(),
        };

        frame.a0 = ret.error as usize;
        frame.a1 = ret.value;
        frame.mepc += 4;
        return;
    }
}
