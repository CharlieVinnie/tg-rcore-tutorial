# 第三章 代码阅读指南：多道程序与分时多任务

本章在第二章批处理系统建立的特权级隔离基础之上，解除了用户程序必须串行执行的限制。系统演进为一个支持多道程序（Multiprogramming）协作运转，并利用时钟中断（Timer Interrupt）机制实现抢占式分时调度的操作系统。阅读本章时，请重点关注任务执行状态在内存中的存储机制，以及触发时间片轮转的硬件机制。

## 1. 任务控制块（TCB）与并发布局

为了实现多任务并发执行，内核无法继续使用单一的全局变量来维护运行状态。每个任务必须在内存中分配独立的数据结构，用以在发生特权级切换时记录和保存其寄存器上下文与执行状态。

### 1.1 `tg-rcore-tutorial-ch3/src/task.rs`
**阅读范围：** `TaskControlBlock` 结构体定义及 `init` 方法。
此文件引入了操作系统核心的数据结构：任务控制块（TCB）。阅读时请聚焦以下定义：
```rust
pub struct TaskControlBlock {
    ctx: LocalContext,
    pub finish: bool,
    stack: [usize; 1024],
}
```
该数据结构表明，记录各通用寄存器的状态快照 `LocalContext` 已被整合入任务实例中。它与大小为 8 KiB 的内核端执行栈 `stack`，以及标识任务生命周期的布尔值 `finish` 组成了完备的任务描述单元。
请查阅同文件内的 `init` 方法。内核在装载阶段遍历每个应用程序时，会调用 `LocalContext::user(entry)` 创建初始执行上下文，并将相应的私有栈 `stack` 的最高内存地址赋予该上下文的栈指针记录 `sp_mut()`，从而确立栈区的基地址。

## 2. 环境初始化与特征注入

具备了上述数据结构的支撑后，内核启动流程需要注册额外的底层能力，并配置硬件级中断，以允许异步硬件事件强行夺取用户态程序的执行权。

### 2.1 `tg-rcore-tutorial-ch3/src/main.rs` (第 1 处)
**阅读范围：** `rust_main` 函数的开头至“开启 S 特权级时钟中断”注释处。
除延续前述章节的接口配置外，需关注第 3 步代码：
```rust
    tg_syscall::init_scheduling(&SyscallContext);
    tg_syscall::init_clock(&SyscallContext);
    tg_syscall::init_trace(&SyscallContext);
```
**结构体新增实现与控制反转（IoC）：**
如同第二章所述，内核通过传递静态引用 `&SyscallContext` 至抽象分发库 `tg_syscall`，完成了底层能力的依赖注入。本章中，`SyscallContext` 额外实现了 `tg_syscall::Scheduling`、`tg_syscall::Clock` 及 `tg_syscall::Trace` 等特征。例如，在 `Clock` 特征的实现中，模块读取机器层的 RISC-V `time` 寄存器值，将其转换为符合 POSIX 语义的纳秒格式并返回。这使得全局系统调用分发机制能够在接收到硬件无关的调用号后，映射并执行平台强相关的具体代码。

随后需关注第 5 步代码：
```rust
    unsafe { sie::set_stimer() };
```
**Rust 与体系结构解释：**
`sie`（Supervisor Interrupt Enable）是 RISC-V 架构中 S 态的中断使能控制寄存器。执行此接口函数将在硬件级别激活 S 态定时器中断许可（STIE 位）。其直接导致的结果是，当硬件定时器触发时，CPU 必须无条件中断当前正在低特权级执行的用户指令流，强制其跳转至系统预设的异常处理入口（即第二章所述的由 `stvec` 指向的 `m_trap_vector`）。此机制是实现抢占式调度的必然前提。

## 3. 分时循环与控制权让渡

配置完毕后，内核主线程进入轮询结构（Round-Robin 调度模型），依赖时间片对所有未执行完毕的任务进行周期性调度。

### 3.1 `tg-rcore-tutorial-ch3/src/main.rs` (第 2 处)
**阅读范围：** `while remain > 0` 结构起至 `unsafe { tcb.execute() };` 指令前的准备阶段。
代码通过运算逻辑 `i = (i + 1) % index_mod` 实现了一个循环队列的遍历：
```rust
                tg_sbi::set_timer(time::read64() + 12500);
```
**控制流追踪：**
1. **时间片配额下发：** 在将 CPU 控制权交予某个状态未决任务（`!tcb.finish`）前，内核调用底层 SBI 接口，向 M 态系统请求设定下一次时钟中断的触发阈值，该阈值被典型设定为相对当前时间 12500 个时钟周期后。
2. **入场执行：** 随后，内核通过语句 `unsafe { tcb.execute() }` 执行与第二章相同的汇编上下文切换序列：将通用寄存器状态载入当前引用的 `tcb.ctx`，继而降低当前特权级别至 U 态。硬件最终恢复执行轮转队列中索引为 `i` 的用户态应用程序。

## 4. 抢占与协作：异常返回时的状态判定

用户态应用程序交出现有控制权的方式分为两种：一是由于执行系统调用指令而主动返回；二是由于前述时钟发生器达到阈值而被硬件异常强制中断。当系统的控制流随硬件异常路由（Trap）回卷至 S 态内核时，系统必须对任务状态进行判定。

### 4.1 `tg-rcore-tutorial-ch3/src/main.rs` (第 3 处)
**阅读范围：** 异常处理块 `match scause::read().cause()` 及其内部逻辑。
在指令执行完毕并从 `tcb.execute()` 方法返回至 Rust 控制流时，系统即刻读取底层被硬件置位的 `scause` 寄存器。目前可被处理的合法控制流分支存在两种：

**路径一：强行抢占（时钟中断到达）**
```rust
    Trap::Interrupt(Interrupt::SupervisorTimer) => {
```
若模式匹配结果为 `SupervisorTimer`，意味着在步骤 3.1 预设的 12500 时钟周期已被耗尽。硬件控制流被强制重定向至 S 态处理向量（`m_trap_vector` -> `execute_naked`）。
内核据此判定当前执行程序的执行配额已耗尽。系统在输出日志 `app{i} timeout` 后，维持当前任务的运行时标志为 `finish = false`，调用 `break` 语句退出当前的子事件循环。控制流转移回外层，调度器将选取轮转队列中的下一合法成员执行。

**路径二：协作请求（用户主动调用系统服务）**
```rust
    Trap::Exception(Exception::UserEnvCall) => {
```
若用户程序主动触发 `ecall` 指令，执行权返回内核后将进入 `tcb.handle_syscall()` 的解析逻辑。其解析结果以枚举类型 `SchedulingEvent` 的形式返回并受到主控循环逻辑的二次匹配：
- 若返回 `Event::None`（表征为未涉及状态变更的系统调用，例如获取时间或数据输出），则分支逻辑执行 `continue`。系统的当前子循环不被中断，当前进程将被立刻压回 `tcb.execute()` 重新获得执行权，由于中断定时器并未重置，原有的配额时钟周期将在此程序中继续流失。
- 若返回 `Event::Yield`（表征为进程主动出让 CPU 使用权），表示应用程序主动放弃剩余的可用时间片配额。程序分支直接调用 `break`，将控制权强行返回至外层调度主循环。

### 4.2 `tg-rcore-tutorial-ch3/src/task.rs` (第 2 处)
**阅读范围：** `handle_syscall` 方法与 `SchedulingEvent` 枚举。
为彻底理解路径二中的逻辑枚举转换，需回到 `TaskControlBlock` 的系统调用分发实现：
```rust
    pub fn handle_syscall(&mut self) -> SchedulingEvent {
```
在此方法内，模块提取上下文中存放的寄存器参数（`a7` 为调用号，`a0-a5` 为传参），进而转交底层的 `tg_syscall::handle`。调用结束后：
- 系统首先通过 `self.ctx.move_next()` 令虚拟程序计数器（`sepc`）增加 4 字节，从而跳出异常引发点（`ecall` 指令本身）。
- 系统依据 `tg_syscall` 的抽象行为识别调用号。若属于正常服务流转则包装为 `SchedulingEvent::None`。若系统调用号为特殊的 `SCHED_YIELD`，则封装为 `SchedulingEvent::Yield` 向上返回，交由 4.1 节的主循环完成实际的上下文剥离操作。

本系统通过硬件定时器的强行抢夺及接口约定的协作规程两项核心机制，建立并维持了多任务调度的稳定运转。
