# 第三章 sys_trace 系统调用重构实验报告

## 实验背景与初始实现

本章节的核心编程任务为实现 `sys_trace` 系统调用（调用号 410），旨在追踪和统计用户程序的系统调用历史记录。其中，`trace_request` 为 2 的功能要求返回特定系统调用编号的已调用次数。

在初始的实现方案中：
1. 于 `TaskControlBlock` 结构体中新增了 `syscall_counts` 数组，用于记录各系统调用的触发次数。
2. 在处理 `trace_request` 为 0（读取）和 1（写入）时，利用裸指针直接访问用户内存完成了功能。
3. 针对 `trace_request` 为 2 的计数查询功能，由于遭遇了架构层面的限制，采取了非标准的拦截式实现。

## 遇到的技术阻碍与架构缺陷

在标准架构设计中，`sys_trace` 的核心处理逻辑应当被完全封装于 `src/main.rs` 内的 `impl Trace for SyscallContext` 代码块中。然而，系统调用执行环境 (`SyscallContext`) 并不具备直接访问所属任务控制块 (`TaskControlBlock`) 内部数据的权限。

为了规避上述变量作用域及借用关系问题，初始方案采用了一种破坏模块封装的临时手段：
1. 在 `impl Trace for SyscallContext` 中，对于 `trace_request == 2` 的调用，仅返回数值 `0` 作为占位符。
2. 在系统的调度事件分发循环（即 `src/task.rs` 中的 `TaskControlBlock::handle_syscall` 函数内部），加入特判逻辑。若侦测到当前请求为 `sys_trace` 且参数对应计数查询，则直接越级读取内部 `syscall_counts` 数组，并在系统调用返回前，强行覆盖写入内核向用户态返回的寄存器 `a0`。

此方案虽顺利通过了自动化测试，但严重违背了系统调用处理过程与底层分发逻辑的隔离原则，导致业务逻辑发生无序外溢。

## 用户指导与代码重构

在此过程中，用户及时发现了该设计缺陷，并明确下达指令，要求将 `sys_trace` 的处理逻辑完全移回其正规归属地 `impl Trace for SyscallContext` 中。在用户的指导下，对原代码逻辑进行了彻底的重构。

修复方案如下：
1. **解除耦合与数据前置传递**：启用系统调用封装中预留的 `Caller` 结构特性。当 `TaskControlBlock::handle_syscall` 触发 `tg_syscall::handle` 请求时，将任务结构体内存放的具体 `syscall_counts` 数组地址，通过强转转换为裸指针再转换为 `usize`，装载至 `Caller.entity` 字段向下层传递。
2. **指针还原机制**：在 `src/main.rs`的 `sys_trace` 实现主体内部，通过利用传导而来的 `_caller.entity` 参数，辅以 `core::slice::from_raw_parts` 将无类型内存重新构造为具备 `500` 容量的无类型数组切片，自此合法且高效地读取对应系统调用的计数值。
3. **清理越权代码**：将 `TaskControlBlock::handle_syscall` 内的所有针对 `sys_trace` 拦截与覆盖赋值的判定语句彻底删除。

## 结论

本次重构工作在用户明确纠正指引下完成。新方案妥善解决了内部模块间的访问限制限制，同时还原了 `tg-rcore-tutorial` 所要求的干净执行上下文。核心的逻辑模块最终得以在它应在的位置被实现，后续 `./test.sh all` 所执行的全量回归测试结果亦充分证明了本次修改的正确性及系统的整体稳定性。
