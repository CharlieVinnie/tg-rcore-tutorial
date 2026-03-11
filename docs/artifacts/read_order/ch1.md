# 第一章 代码阅读指南：最小化操作系统的执行环境

本章的核心目标是构建一个能够直接在 RISC-V 裸机硬件上运行的最小化独立程序。由于目标环境缺乏常规操作系统提供的隐式运行时（如 C 语言的 `crt0` 或 `exit`），所有的底层栈帧与执行状态必须手动构造。以下为本章代码阅读的严格时间顺序。请务必遵循此顺序进行阅读，以建立完整的指令流执行概念。

## 1. 编译与链接阶段的准备

在阅读任何高级语言代码前，必须首先理解程序的模块依赖关系及其在物理内存中的布局。本章未使用 Makefile，其编译逻辑由 Cargo 与构建脚本定义。

### 1.1 `tg-rcore-tutorial-ch1/Cargo.toml`
**阅读范围：** 全文。
请特别关注对 `tg-sbi` 依赖的声明：
```toml
[dependencies]
tg-sbi = { package = "tg-rcore-tutorial-sbi", path = "../tg-rcore-tutorial-sbi", version = "0.4.5", features = ["nobios"] }
```
此处激活了 `tg-sbi` 包的 `nobios` 特性。该属性的运用表明我们的系统不依赖于外部引导固件，而是使用自带的核心汇编代码作为机器加电后的第一条执行指令。

### 1.2 `tg-rcore-tutorial-ch1/build.rs`
**阅读范围：** 全文及其内嵌的 `LINKER_SCRIPT`。
该脚本在编译阶段生成链接脚本 `linker.ld`。请阅读常量 `LINKER_SCRIPT`：
```rust
const LINKER_SCRIPT: &[u8] = b"
OUTPUT_ARCH(riscv)
ENTRY(_m_start)
```
此处的链接脚本负责指示链接器将 M 态（Machine mode）代码段安置在物理地址 `0x80000000`，并将 S 态（Supervisor mode）代码段排列在 `0x80200000` 处。此布局是机器初始化时控制流转移的物理前提。

## 2. 系统加电与 M 态引导过程

由于配置为不附带外部固件（QEMU `-bios none`模式启动），机器加电后，CPU 强制处于最高特权级的 M 态，PC（程序计数器）将被置于物理地址 `0x80000000` 开始取指。这就要求离开当前主目录，进入依赖库的底层目录。

### 2.1 `tg-rcore-tutorial-sbi/src/m_entry.asm`
**阅读范围：** `_m_start` 汇编块全文。
```assembly
    .section .text.m_entry
    .globl _m_start
_m_start:
```
**控制流与栈操作追踪：**
当 CPU 进入 `_m_start` 时，栈尚未被设置，无法处理高级函数调用。
1. **栈初始化：** `la sp, m_stack_top` 为 M 态设立专用的执行栈，并将此栈顶寄存以防被随后的 S 态破坏。
2. **状态配置：** 代码改写 `mstatus` 寄存器定义将引发权限降级的目标特权级为 S 态。
3. **跳转准备：** 执行 `la t0, _start` 与 `csrw mepc, t0` 将 S 态代码入口地址存储至 `mepc`（机器异常程序计数器）。此时 CPU 未立即跳转。
4. **中断与保护设置：** 完成陷阱异常的快速委托与由于简化的 PMP（物理内存保护）全区域放行配置。
5. **权限降级：** 最终通过 `mret` 指令，CPU 切换至 S 态，并以 `mepc` 寄存器作为新 PC 飞跃至链接器安排的 `0x80200000` 所在处。

## 3. S 态入口与运行时建立

通过 `mret`，控制权正式降落在 S 态主内核代码的第一条执行指令上。

### 3.1 `tg-rcore-tutorial-ch1/src/main.rs` (第 1 处)
**阅读范围：** `_start` 函数块。
```rust
#[unsafe(naked)]
unsafe extern "C" fn _start() -> ! {
```
**Rust 特性解释：`#[unsafe(naked)]`**
在通常的 Rust 函数中，编译器自动注入汇编的“序言”和“尾声”用于保护调用者寄存器与构建栈空间。在此刻，S 态内尚不存在任何有效栈帧。裸函数（naked function）命令编译器放弃注入这些预置代码，以便直接执行开发者指定的汇编语句。

**控制流与栈操作追踪：**
这是由裸函数封装的环境搭建阶段：
1. **栈初始化：** 调用 `core::arch::naked_asm!` 内嵌宏，通过 `la sp, {stack} + {stack_size}` 令 SP（栈指针寄存器）指向保留在 `.bss.uninit` 区域的一段 4 KiB 静态内存块的最高端（栈帧向低地址方向生长）。
2. **长跳转运行：** 栈构造完毕后，使用无条件跳转 `j {main}` 将控制流单向移交予系统的主体逻辑函数 `rust_main`，完成初始化流程。

## 4. 主干逻辑与环境调用（SBI）

有了稳固的 S 态栈，操作系统在此进行最简单的控制台输出，继而请求关机。

### 4.1 `tg-rcore-tutorial-ch1/src/main.rs` (第 2 处)
**阅读范围：** `rust_main` 函数块与底部的 `panic_handler` 处理程序。
```rust
extern "C" fn rust_main() -> ! {
    for c in b"Hello, world!\n" {
```
在此函数内，迭代字节数组并最终通过外部接口 `console_putchar` 输出文本信息，随后以 `shutdown(false)` 请求系统正常停止。这两个调用的底层具体行为指向了下一阶段的代码剖析。

### 4.2 `tg-rcore-tutorial-sbi/src/lib.rs`
**阅读范围：** `sbi_call`（带有 `nobios` 属性的重载版本），`console_putchar` 和 `shutdown` 接口包装函数。
```rust
#[cfg(all(target_arch = "riscv64", feature = "nobios"))]
#[inline(always)]
fn sbi_call(eid: usize, fid: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
```
当诸如 `console_putchar` 的高级接口调用底层 `sbi_call` 时：
1. `sbi_call` 将预先准备必要的系统调用号放置于特定的寄存器内。
2. 内部嵌入执行汇编指令 `ecall`。这标志着对操作环境进行显式的异常陷入请求以获取 M 态的硬件交互支持。此指令引发同步自陷（Trap），导致 CPU 放弃当前 PC 与状态，被迫跳转转回高特权级陷阱向量入口。

### 4.3 `tg-rcore-tutorial-sbi/src/m_entry.asm` (第二次)
**阅读范围：** `.section .text.m_trap` 及其属下的 `m_trap_vector` 标签块。
```assembly
m_trap_vector:
    # 最小 M 态陷阱入口：主要处理来自 S 态的 ecall（SBI 调用）
```
**控制流与栈操作追踪：**
从 `ecall` 被触发的物理时刻起：
1. **栈切换与现场保护：** PC 来到 `m_trap_vector`。CPU 先将工作栈换回 M 态独占执行栈，执行 `sd` 操作依次把原有的 S 态所有通用寄存器压入栈中以防覆盖。
2. **委托处理：** 调用位于 `tg-rcore-tutorial-sbi/src/msbi.rs` 中的 `m_trap_handler` 并传递相应的异常环境信息，依靠 Rust 层处理该 `ecall` 对应的控制台文字输出或系统关机命令。
3. **消除自陷循环：** 返回到汇编后，执行 `addi t0, t0, 4` 主动将暂存在 `mepc` 里的返回地址推进 4 字节，从而略去引起此异常的 `ecall` 指令，否则返回后将陷入重复执行异常的无穷死循环。
4. **现场还原与返回：** 从栈中相继弹出暂存的通用寄存器还原，将指针切换回其原始状态。之后利用 `mret` 指令回落权限等级至 S 态，自原 `rust_main` 的异常引发点继续完成后续迭代计算或终止。
