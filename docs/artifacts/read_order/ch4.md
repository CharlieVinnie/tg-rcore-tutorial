# 第四章 代码阅读指南：地址空间与虚拟内存

本章在先验多任务处理机制的基础上，引入了受里斯克-V (RISC-V) Sv39 标准支持的虚拟内存架构。操作系统的核心职责由单纯的 CPU 时间片分配，扩展至对物理内存的多重映射与进程间地址空间的绝对隔离。

阅读本章时，必须严格遵循本文档提供的线性顺序。系统组件关联性剧增，跳跃阅读将导致对控制流挂起和恢复机制的理解产生断层。

## 1. 编译构建阶段：提取与链接

构建启用了地址隔离的操作系统，首要任务是在编译期分离内核与用户程序的物理布局引用。

### 1.1 `tg-rcore-tutorial-ch4/build.rs`
**阅读范围：** `build_apps` 函数与 `write_app_asm` 函数。
在内核主程序编译之前，`build.rs` 首先接管了用户侧应用程序的编译。
```rust
    let status = Command::new("rust-objcopy").args([...])
```
系统调度 `cargo build` 依次生成各用户程序的 ELF 文件，随后调用 `rust-objcopy` 将其剥离元数据并转换为纯二进制格式。由 `write_app_asm` 生成的汇编序列被输出至 `app.asm`。此文件随后在 `main.rs` 中由 `global_asm!` 宏整体摄入内核数据段。此举保证了后续内核在运行时能在自身的静态内存空间中寻址到所有用户程序的原始加载镜像。

## 2. 内存分配器宏观基础

具备解析与装载 ELF 宽泛结构的前提，是内核必须具备运行时动态分配变长物理页的能力，以构建进程专有的树状多级页表。

### 2.1 `tg-rcore-tutorial-ch4/src/main.rs` (第 1 处)
**阅读范围：** `rust_main` 中调用 `tg_kernel_alloc::init` 及 `transfer` 的部分。
```rust
    tg_kernel_alloc::init(layout.start() as _);
    unsafe { tg_kernel_alloc::transfer(...) };
```
内核初始化阶段，将未被静态链接占用的物理内存范围交托于分配器。

### 2.2 `tg-rcore-tutorial-kernel-alloc/src/lib.rs`
**阅读范围：** `init` 函数，及底层 `GlobalAlloc` 块实现。
请跟随上述调用进入外挂组件库。
```rust
#[global_allocator]
static GLOBAL: Global = Global;
```
1. 此文件实现了 Rust 核心库所定义的 `GlobalAlloc` 特型。在 Rust 的标准抽象中，当代码（如 `Box::new` 或 `Vec::push`）产生动态内存需求时，编译器会自动构造一个 `Layout` 结构体，其中包含所需内存的精确字节数与内存对齐要求（如要求地址按 8 或 4096 字节对齐）。此 `Layout` 实例随即被传递至已被 `#[global_allocator]` 宏标记的全局分配器的 `alloc` 方法中。
2. 此包通过 `Global` 结构体实现了上述 `GlobalAlloc` 接口。在 `alloc` 的具体实现内部，它将标准的 `Layout` 请求进一步下派至 `customizable_buddy` 库提供的 `BuddyAllocator`。
3. `BuddyAllocator` 采用伙伴算法管理内存。它将空闲内存池划分为不同以 2 的指数次幂为大小的内存块（伙伴块）。当接收到 `Layout` 传递的大小与对齐条件时，它向上取整至最近的 2 的次幂大小的空闲块并分配返回。
4. 内部采用了单处理器安全的 `StaticCell<T>` 规避了并发环境下的数据竞争与 Rust 编译器的 `static mut` 限制。由于使用了 `UnsafeCell::new` 获取底层原生指针执行变更，开发者能够合规地持有并维护伙伴分配器的变动状态。
离开此包后，内核主逻辑中的 `alloc` 宏及基于动态堆的容器（例如 `Vec<Process>`）才拥有了可执行的物质基础。

## 3. 地址空间与进程加载

动态分配器准备就绪后，内核进入逻辑拓扑建立阶段。

### 3.1 `tg-rcore-tutorial-ch4/src/main.rs` (第 2 处) 及 `tg-rcore-tutorial-linker/src/lib.rs`
**阅读范围：** `kernel_space` 函数及其上方分配 `PROTAL_TRANSIT` 的逻辑；横向比对 `tg-linker` 中的 `KernelLayout` 与 `KernelRegionIterator` 结构。

在解析内核地址空间之前，必须理解 `kernel_space` 函数接收的 `layout` 参数。
1. **`KernelLayout` 定位：** 当执行 `tg_linker::KernelLayout::locate()` 时，由 C 语言 ABI 导出的外部链接器符号（如 `__start`, `__rodata`, `__end` 等，它们定义在上一节提到的 `linker.ld` 脚本中）被 Rust 捕获。`KernelLayout` 结构体将这些底层裸指针安全地封装为 `usize` 数值边界。
2. **安全迭代器架构：** `kernel_space` 函数之所以能够优雅地书写 `for region in layout.iter()`，是因为 `tg-linker` 为 `KernelLayout` 实现了自定义的迭代器 `KernelRegionIterator`。该迭代器依照 `.text` -> `.rodata` -> `.data` -> `.boot` 的严格内存排布顺序，在每次调用 `next()` 时构造并计算出一个 `KernelRegion`（包含段名称语义 `title` 和闭区间界限 `range`）。这种设计将极度危险且易错的硬编码地址硬算，转化为符合 Rust 零成本抽象哲学的声明式区间迭代。

在 `kernel_space` 中，内核利用由迭代器吐出的每一个分区 `region`，为自身建立独立的 Sv39 三级页表：
```rust
        let s = VAddr::<Sv39>::new(region.range.start);
        let e = VAddr::<Sv39>::new(region.range.end);
        space.map_extern(s.floor()..e.ceil(), PPN::new(s.floor().val()), build_flags(flags))
```
函数内部遍历了由链接脚本暴露的静态区间组（包括 `.text`，`.data`，`.rodata` 等）。请仔细审视这段内存映射的代码行，它展示了 Rust 强类型封装在页表安全中的应用：
1. **`VAddr` 抽象：** `VAddr::<Sv39>` 是由内核专有组件 `tg-kernel-vm` 和底层硬件抽象库 `page_table` 联合提供的类型。它将裸指针或 `usize` 类型的地址数值包裹，通过类型系统强制赋予其“属于 Sv39 架构的一维虚拟内存地址”和对应位数校验的严格语义。
2. **`AddressSpace` 控制结构：** 声明的 `space` 为 `AddressSpace` 实例，它本质上是该地址空间根页表（存放于独占的一页物理内存中）和所有已分配 VPN 记录容器的集合代理。
3. **安全页对齐（`floor`, `ceil`）：** 针对传入的不按页边界整齐排列的原始地址，`VAddr` 必须强制降级或升级转换为页号（`VPN`）。左边界使用向下取整的 `floor()`，右边界使用向上取整的 `ceil()`，严谨地圈定连续的虚拟页范围。
最终，凭借 `AddressSpace::map_extern` 外部投射映射接口，在无需启动全自动物理块新分配的情境下，其将内核的合法代码静态区间与自身实施了硬件级别的恒等映射（Identity Mapping，即各级 VPN 与 `PPN` 在值上精确对等），并辅以最低权限门限（如只读 `__RV`、写执行等 `flags`）。

最后，它将上一步为主管跨段转移分配好的独立物理传送门页，强制挂载至系统虚拟地址空间的最高页记录内，落实了虚拟高域的固化配置。并以配置 `satp` 寄存器的写特权操作，正式启动硬件级别的虚拟地址防线。

### 3.2 `tg-rcore-tutorial-ch4/src/process.rs`
**阅读范围：** `Process::new` 方法。
伴随着 `main.rs` 中应用检测循环的递进，控制流转入针对单体用户程序的解析。`Process::new` 展示了操作系统从静态数据缔造隔离运行时实体的全貌：
1. **ELF 头部验签：** 首先，系统利用 `xmas_elf` 库的解析能力，提取并验证可执行文件的 ELF 头部魔法签名与内核机器码标识（`Machine::RISC_V`），确认其为合法的可执行镜像后获取程序的入口地址 `entry`。
2. **初始化虚拟地址基建：** 实例化一张挂载于全新物理页表根节点的 `AddressSpace` 实例，以及用于追踪堆分界线的计数器 `max_end_va`。
3. **程序段投射转移：**
```rust
        for program in elf.program_iter() {
```
通过上述迭代，代码滤除多余的调试与符号表数据，唯独筛选出带 `Load` 属性的程序段。依照指定的虚拟内存偏移，内核调用 `AddressSpace::map` 将二进制程序切片强行复制至新分配的物理页帧，并强制依据段定义的只读、读写、可执行等规则注入位权限。同时，为了沙盒的安全底线，必须为这些段强行打入表征用户资源隔离的 `U` (User) 特权旗标。

4. **动态堆基址锁定：** 根据步骤 3 中记录的应用程序最高可用虚拟地址，系统对齐出紧接其后的第一张页表始端作为进程栈与堆生长的起步红线 `heap_bottom`。
5. **构筑高位隔离运行栈：** 用户程序运行时不可借用内核栈帧，因此系统调用 `alloc_zeroed` 截取 2 页（8 KiB）全新且清零的物理内存作为私有用户栈。这块物理疆域被系统映射到了该用户虚存版图高居近 256 GiB 顶点的特定逻辑地址区间 `[(1 << 26) - 2, 1 << 26)`。系统安全得益于此，栈溢出将直接因为触碰到未映射区间而立刻诱发页故障异常（Page Fault），不会殃及系统低位数据。
6. **组装核心上下文句柄：**
```rust
        let mut context = LocalContext::user(entry);
        let satp = (8 << 60) | address_space.root_ppn().val();
        *context.sp_mut() = 1 << 38;
```
函数末尾构建的将是决定调度中轴命运的高级控制上下文 `ForeignContext`。除了将前述获得的入口函数指针（`entry`）和人为架设在 256 GiB 高位的虚假栈顶基址（`sp`）注入系统调用返回骨架外，至关重要的一点是系统依据全新的应用专属物理页表根索引拼合生成了代表着进入虚存时代唯一凭证的特征值寄存器映像——`satp`。没有此凭证的平稳置换，所有的内存隔离均属空谈。

## 4. 特权级穿梭机制：多槽传送门

在地址维度上彻底物理隔离内核与用户层后，特权切换便无法直接依赖连续的内存指令。因为一旦 `satp` 寄存器遭到改写，旧有指令流地址将在新页表中无法命中而立即触发极严重的取指异常（Instruction Page Fault）。

### 4.1 `tg-rcore-tutorial-kernel-context/src/foreign/multislot_portal.rs`
**阅读范围：** `MultislotPortal` 结构定义与其 `init_transit` 安全规约。
前往此组件包。
```rust
pub struct MultislotPortal {
```
此结构体在内存规划的末端充当两界物理通道。其利用预定义的独立纯汇编代码页（即传送门自身），被分别以相同的最高位虚拟地址挂载于内核空间与用户空间之中。

### 4.2 控制流非标准溯源 (`MultislotPortal` 执行路径)
回到 `ch4/src/main.rs` 第 236 行的 `schedule()` 主循环：
```rust
        unsafe { ctx.execute(portal, ()) };
```
对于此方法的调用偏离了常规函数压栈返回过程。明确其底层执行绪如下：
1. **CPU 指针定向：** 编译器将被调用的代码段跳转至处于高危虚拟分水岭上的 `portal` 页汇编块。
2. **状态覆盖与寄存器重配：** 当前 CPU 之工作根页表 `satp` 寄存器在汇编序列内部被强行替换为用户目标程序的页表指令基址。得益于步骤 4.1 所述之双端复用映射，此时 CPU 程序计数器（PC）依然有效。
3. **入界剥离：** 执行最后的特权级下降指令 `sret`，硬件强行截断当前内核流水线，跳转回用户态代码。
当用户态发生异常或系统调用（`ecall`），则执行流程逆向上述逻辑，由于异常处理器（`stvec`）均被指向了该双端传送门的入口，硬件直接陷入传送门，切换回内核 `satp` 并于后续恢复原始 Rust 栈指针向下执行。

## 5. 指针越界保护与逆向重寻

### 5.1 `tg-rcore-tutorial-ch4/src/main.rs` (第 3 处)
**阅读范围：** `mod impls` 模块下 `SyscallContext::write` 等具体系统调用接口实现。
进程发起的访问参数具有极强隐患。例如 `write` 调用传来的缓冲首地址系用户态视图提供的虚拟地址序列。对于居于内核态恒等映射架构中的特权执行流，该虚拟地址不可信且不可直接访问。
```rust
    PROCESSOR.get_mut().current().unwrap().address_space.translate::<u8>(VAddr::new(buf), READABLE)
```
所有具备数据交互诉求的 `SyscallContext` 成员必须通过上述调用实现地址软翻译。借由直接查阅请求方所控制的多级树状页表索引，内核通过 `translate` 一方面检验该页框授权 `READABLE` 或 `WRITEABLE` 是否合法，另一方面将寻得的底层物理页基址挂配原偏移量组成新的安全访问型指针予以数据处理。未通过该检测的访问即刻予以致命阻断并返回错误码。
