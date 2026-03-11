# 第二章 代码阅读指南：批处理系统与特权级切换

本章在第一章最小化执行环境的基础上，引入了受保护的用户模式（U-mode）与内核特权模式（S-mode）隔离机制。系统的核心职责演变为：将多个独立编译的用户应用程序依次装载并执行，同时通过系统调用（System Call）为受限的用户程序提供输出与退出服务。本指南严格按照代码执行的时间顺序展开，请依序阅读。

## 1. 应用程序的编译与打包

在内核启动前，所有用户应用程序必须已被预先编译，并作为数据段无缝整合至内核镜像中。这是批处理系统能够找到目标代码的物理前提。

### 1.1 `tg-rcore-tutorial-ch2/build.rs`
**阅读范围：** `build_apps` 和 `write_app_asm` 函数。
构建脚本会在编译期自动寻找用户应用库，通过 Cargo 指令独立编译它们，接着使用 `rust-objcopy` 剥离调试信息，最终调用 `write_app_asm` 生成一个汇编片段：
```rust
fn write_app_asm(path: &PathBuf, base: u64, step: u64, bins: &[PathBuf]) {
```
此步骤产出的 `app.asm` 汇编文件将内联包含所有应用的可执行二进制数据，并构建一个名为 `apps` 的数组以便内核寻址。

### 1.2 `tg-rcore-tutorial-linker/src/app.rs`
**阅读范围：** 全文。
当且仅当理解了打包机制后，方可阅读此文件。它定义了内核如何在其内存空间内解析上一步生成的二进制打包数据：
```rust
pub struct AppMeta {
```
通过调用 `AppMeta::locate()` 和迭代器 `iter()`，内核能够精确提取出每个应用程序的首字节物理地址及其长度。当指定的 `base` 地址不为 0 时（如后续章节的固定虚拟地址），切片数据将被硬拷贝至对应内存再返回。但在本章，它仅仅是迭代内存中的静态数据供内核加载。

## 2. 内核初始化与系统调用挂载

内核的引导过程与第一章类似（通过 `_start` 汇编进入 S 态），但主函数的首要任务转为了环境子系统的注册与初始化。

### 2.1 `tg-rcore-tutorial-ch2/src/main.rs` (第 1 处)
**阅读范围：** `rust_main` 函数至第三步（初始化系统调用处理），并结合文件底部的 `mod impls` 模块。
```rust
extern "C" fn rust_main() -> ! {
```
内核首先调用 `tg_linker::KernelLayout::locate().zero_bss()` 进行内存清理。随后执行关键的接口映射：
```rust
    tg_console::init_console(&Console);
// ...
    tg_syscall::init_io(&SyscallContext);
    tg_syscall::init_process(&SyscallContext);
```
**结构体实现解释：`Console` 与 `SyscallContext` 及 `tg_syscall` 的控制反转**
在初始化代码中传入的 `&Console` 和 `&SyscallContext` 均为定义在同文件底部 `impls` 模块内的空结构体（Zero-Sized Types）。
- `Console` 结构体实现了 `tg_console::Console` 特征（Trait），其内部包裹了第一章用到的 `tg_sbi::console_putchar`，使底层 SBI 打印能力对接到更高层的宏（如 `print!`）。
- `SyscallContext` 结构体则同时实现了 `tg_syscall::IO` 与 `tg_syscall::Process` 特征。在特征方法 `write` 中，解析文件描述符并调用 `print!` 宏输出字符串缓存；在 `exit` 方法中执行退出逻辑。

`tg_syscall` 是一个平台无关的系统调用分发库。它仅定义了各子系统（IO、Process 等）的抽象特征（Trait）以及对应的系统调用号，其内部本身不包含针对具体硬件的操作代码（例如，它并不知道如何向串口发送字符，或如何进行硬件关机）。通过将 `&Console` 与 `&SyscallContext` 等实现了硬件相关操作的实例，作为参数传递给 `tg_syscall::init_*` 函数，内核完成了典型的控制反转（IoC）。这相当于内核主动把“具体的执行方法”装配至“抽象的分发器”中。当用户程序随后从 U 态触发 ecall 指令时，系统级的分发器便可以直接使用这些组装好的具体能力来满足各类系统调用请求。

### 2.2 `tg-rcore-tutorial-syscall/src/kernel/mod.rs`
**阅读范围：** `handle` 函数。
在 2.1 节中，内核仅仅“注册”了各种具体能力。但当后续运行的用户程序真正发起系统调用例外时，内核需要一个总控模块，来将硬件捕获到的“系统调用号”翻译并转发给对应的具体模块。`handle` 函数正是扮演这个“总机房”角色的系统调用分发机制本身。因此，在理解了能力的注册后，应当立刻查看分发器是如何工作的：
```rust
pub fn handle(caller: Caller, id: SyscallId, args: [usize; 6]) -> SyscallResult {
```
该函数使用 `match id` 语句接收请求的类别。当匹配到有效的 `id`（系统调用号）时，利用事先挂载的特征钩子将此号映射至对应子系统（如 `IO` 或 `PROCESS`）的具体闭包回调中，最终调用之前由于 IoC 而注入的本章主内核真实实现。如果在阅读时觉得这部分目前与运行主线脱钩，可将其视为一次预先了解，在阅读第 5 节时再回顾此处的最终触发行为。

## 3. 用户特权态上下文构建与切换

准备就绪后，内核进入主循环，开始批处理装载程序。

### 3.1 `tg-rcore-tutorial-ch2/src/main.rs` (第 2 处)
**阅读范围：** `rust_main` 的第四步循环块（`for (i, app) in ...`），至 `unsafe { ctx.execute() }` 处暂停。
```rust
        let mut ctx = LocalContext::user(app_base);
```
**结构体实现解释：`LocalContext`**
`LocalContext` 是本章最重要的数据结构之一，代表了一个线程（本章中即用户程序）的完整硬件执行上下文。由于操作系统在执行任务切换或处理例外时，必须暂时剥夺当前程序的 CPU 控制权，`LocalContext` 就充当了该程序的“状态快照存档点”。它在内存中严格按照预定格式排列，内部包含了全部 31 个通用寄存器的存放数组 `x: [usize; 31]`、用于保存内核栈指针的 `sctx`、记录中断断点的 `sepc`，以及标识特权级和中断状态的布尔值。

在此处，内核为该应用开辟了一段 4 KiB 的用户栈区，并使用静态方法 `LocalContext::user(app_base)` 初始化这个执行环境。该方法除了将 `sepc`（即程序的起始指令地址）指向应用的物理基址 `app_base` 外，还硬性规定了即将弹出的 `sstatus.SPP` 必须设定为降级的 User 模式，确保应用不能越权。随后调用 `ctx.execute()` 将控制权正式移交。

### 3.2 `tg-rcore-tutorial-kernel-context/src/lib.rs`
**阅读范围：** `LocalContext::execute` 方法及 `execute_naked` 汇编块。
这是本章最核心、最复杂的特权级硬件级切换区域。
**控制流与栈操作追踪：**
1. **CSR 准备（S 态）：** `execute` 内计算目标 `sstatus`，随后发起核心寄存器换位：`csrrw {old_ss}, sscratch, {ctx}` 将承载 S 态局部状态的 `LocalContext` 结构体物理地址存入 `sscratch` 寄存器。随后 `call {execute_naked}` 将 S 态栈指针保存到裸汇编函数。
2. **内核现场保存（S 态）：** `execute_naked` 汇编中，执行 `SAVE_ALL`，将 S 态（当前内核处理流）的全部主要寄存器堆叠压入当前的内核栈中。
3. **设置陷入入口（S 态）：** `la t0, 1f` 与 `csrw stvec, t0` 配合，将本段汇编代码下方局部标号 `1:` 的物理地址强制写入 `stvec` 寄存器。这是建立自陷机制的关键一环：它硬性规定了当用户应用程序未来试图通过 `ecall` 请求系统服务，或发生任何诸如非法指令的异常时，CPU 将立刻放弃用户态执行序，无条件跳转至 `1:` 标号处开始运行。
4. **栈帧反转（S 态）：** `csrr t0, sscratch` 取回结构体指针。通过 `sd sp, (t0)` 将刚才保存好 S 态所有寄存器的内核栈指针 `sp` 长期稳固存放在 `LocalContext.sctx` 子段落中。紧接着 `mv sp, t0`，将当前工作栈强行切变为 `LocalContext` 内部数组区域。
5. **用户现场恢复（S 态）：** 使用 `LOAD_ALL` 宏，把存放在 `LocalContext.x` 内的 31 个用户寄存器悉数重载到物理 CPU 的通用寄存器中。
6. **降级生效：** 汇编中的最后一跳为 `sret`。硬件自动验证 `sstatus` 判断切回 U 态，使用 `sepc` 中的值重载 CPU 的 PC，开始执行毫无察觉的用户应用程序第一条指令。

## 4. 用户模式发起系统调用

身处 U 态的用户代码无权直接硬件输出机制，必须通过系统调用协议求助内核。

### 4.1 `tg-rcore-tutorial-syscall/src/user.rs`
**阅读范围：** `native::syscall3` 和高层级封装如 `write` 函数。
当用户态的某个应用程序希望输出字符串时，会调用同目录下的功能函数最终执行这一下降到底层的宏块：
```rust
    pub unsafe fn syscall3(id: SyscallId, a0: usize, a1: usize, a2: usize) -> isize {
```
**控制流与栈操作追踪：**
在 `syscall3` 执行时：
1. **ABI 制约：** 内联汇编强制要求编译器把系统调用号注入 `a7` 寄存器，而对应的参数依据顺位装填至 `a0`、`a1`、`a2` 中。
2. **触发例外：** 汇编执行 `ecall` 机器指令。
3. **硬件级反应：** CPU 在 U 态侦测到陷阱请求，由于之前并未设定该例外的委托，特权级强制上升至 S 态。PC 值从应用控制流中被抽离并重定向至 `stvec` 寄存器里所存的地址（这是之前 `execute_naked` 设置的标号 `1f` 的物理内存入口）。

## 5. 陷阱捕捉与处理返回

处理完毕系统调用后，内核需将状态再还给用户空间。

### 5.1 `tg-rcore-tutorial-kernel-context/src/lib.rs` (第二次)
**阅读范围：** `execute_naked` 的标号 `1` 代码以下部分，以及返回 `execute` 宏后的扫尾动作。
```assembly
        "1: csrrw sp, sscratch, sp",
```
**汇编逆向恢复操作追踪：**
当 PC 因自陷机制到达 `1:` 标号处时：
1. **栈交换：** 此时被中断抛弃的 `sp` 仍然指向用户态私有栈。通过强制 `csrrw` 交换，再次将存有 `LocalContext` 地址的 `sscratch` 放回 `sp` 用作安全的数据结构落脚点，同时 `sscratch` 保存被污染的不可信用户栈指针。
2. **保护用户现场：** 再次调用 `SAVE_ALL`，将当前发生系统调用停滞瞬间的所有 CPU 通用寄存器，完好无损地覆盖写入 `LocalContext.x` 的用户数组存档区中。同时将 `sscratch` 里存好的用户栈指针归为原位写入存档。
3. **提取 S 态现场：** 向前寻址 `ld sp, (sp)` 取出最开始存储的指向有效内核栈空间的 `sctx` 地址，完全置换回起初在 S 态等待响应的原始内核栈。
4. **栈帧提取：** `LOAD_ALL` 把压入内核栈中的内核所有执行状态悉数从栈中弹出。内核恢复如初。
5. **重回 Rust：** 汇编的最后单条 `ret` 指令使得控制流跳出了 `call {execute_naked}` 的禁锢区间，返回至 `LocalContext::execute` 的内联汇编后半段。
6. **上下文信息拾取（S 态）：** 回到 `execute` 宏内部执行余下收尾工作：
```rust
                    ld    ra, (sp)
                    addi  sp, sp,  8
                    csrw  sscratch, {old_ss}
                    csrr  {sepc}   , sepc
                    csrr  {sstatus}, sstatus
```
首先还原被 `call` 修改的返回地址寄存器 `ra`。然后将 `sscratch` 复原为进入前的状态（通常为 0）。最后，内核通过 `csrr` 指令，把硬件刚刚为了响应系统调用最新写入到 `sepc`（引起例外的那条用户指令的物理地址）和 `sstatus` 寄存器的状态值提取到 Rust 局部变量中。
7. **落盘更新：** 在脱离 `asm!` 内联汇编块后，执行 `(*ctx_ptr).sepc = sepc;`，将最新的断点物理地址正式更新到 `LocalContext` 结构体内，并向上层返回最新的特权级状态值。至此，一次单程的特权级漫游彻底结束。

### 5.2 `tg-rcore-tutorial-ch2/src/main.rs` (第 3 处)
**阅读范围：** 返回循环块的 `match scause::read().cause()` 部分以及 `handle_syscall` 实现。
```rust
                Trap::Exception(Exception::UserEnvCall) => {
```
调用完 `execute()`，系统查询由于 ecall 引发的 CPU 异常状态，将解析工作移交 `handle_syscall`，其最终对接前言中初始化的特征进行真实的打印工作或者退出判定。如果继续运行，则再次执行 `execute` 开启一轮新循环。
