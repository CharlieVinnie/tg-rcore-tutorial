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

#![no_std]
#![no_main]

// RISC-V64 架构下启用严格警告和文档检查
#![cfg_attr(target_arch = "riscv64", deny(warnings, missing_docs))]
// 非 RISC-V64 架构允许死代码（用于 cargo publish --dry-run 在主机上通过编译）
#![cfg_attr(not(target_arch = "riscv64"), allow(dead_code))]

extern crate tg_rcore_tutorial_framework;

use tg_sbi::console_putchar;

/// 初始化系统的核心（仅仅对应 hart 0）
#[unsafe(no_mangle)]
pub extern "C" fn rust_main_prelude() {
    // 只有 hart 0 会执行这里进行 BSS 清屏或设备初始化（当前为空，未来由 console_init 负责）
}


/// 真正的业务逻辑处理模块
#[unsafe(no_mangle)]
pub extern "C" fn rust_main_execute(hartid: usize) {
    // 所有 hart 都会打印启动信息（目前使用 SBI 直接输出）
    for c in b"Hello, world! from hart " {
        console_putchar(*c);
    }
    console_putchar(b'0' + hartid as u8);
    console_putchar(b'\n');
}

/// 关机清理
#[unsafe(no_mangle)]
pub extern "C" fn rust_main_epilogue() -> ! {
    tg_sbi::shutdown(false);
}
