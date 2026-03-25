## Task 1

Task 1 主要由 AI 编写完成小实验，但 AI 编写的代码存在以下问题：

+ ch3 中 AI 忽略了已经封装好的 `Trace` 接口，转而在其它部分实现，破坏封装
+ ch5 中 AI 将 `spawn` 函数直接实现成了 `fork`+`exec`，额外拷贝了一次原有进程的地址空间
+ ch6 中 `mmap` 没有正确检测分配的区间是否重叠

由此可以看出，AI 写的程序能跑是不够的，要仔细看看都写了些什么。

## Task 2

在 Task 2 中将 tg-rcore-tutoral 的 ch1 和 ch4 进行扩展，实现了 kernel 的多核运行。

最终效果：可以在 qemu 中使用 `-smp 4` 参数运行 kernel，不同 hart 可以通过互斥锁独立 `println!` 和 `log` 输出执行信息，每个 hart 可以独立运行用户程序，用户程序可以在不同 hart 之间转移。

kernel 运行过程：在 `m_entry` 中挂起除 hart 0 以外的其它 hart，先由 hart 0 进行 kernel 的初始化工作，然后 hart 0 通过 sbi 唤醒其它 hart，随后所有 hart 竞相从进程队列中取出进程运行。

创建了 `tg-framework` crate，该 crate 提供了 `_start` 部分和栈分配等流程，并通过 `AtominUsize` 来控制各个 hart 同步进出各个阶段。具体来说，主 kernel 代码只需提供 `rust_main_prelude` `rust_main_execute(hartid)` 和 `rust_main_epilouge` 的实现，framework 会保证只有 hart 0 执行 `rust_main_prelude`，所有 hart 共同执行 `rust_main_execute`，最终只有 hart 0 执行 `rust_main_epilouge`。在 framework 代码中，hart 0 执行 `prelude` 结束后会通过 sbi 唤醒其它 hart，共同开始执行 `execute`。

### 遇到过的问题

+ 不同 hart 同时调用 sbi 的 `console_putchar` 和 `console_getchar` 会导致 race condition，使得读入/写入出错。解决方案：给 sbi 的这两个函数加锁。
+ 不同 hart 的输出会交织，难以调试。解决方案：给内核态的 `println!` 和 `log` 加锁，并在 `log` 中标明是哪个内核发起的。
+ 忘记将 allocator 改成支持多线程的，导致各种错误。解决方案：加锁。
+ AI 在编写代码时将 boot stack 链接到了 `.bss.uninit` 段而非 `.boot.stack` 段，导致在清空 bss 的时候损坏栈帧。
+ MSIE 没有设置上导致 hart 0 没能成功唤醒其它进程。
+ customizable_buddy 存在 bug，使其返回的内存不一定符合对齐要求，导致在映射跳板页的时候计算 `addr >> 12` 产生非预期的偏移，映射到了错误的物理地址。

## Task 3

实现了 ch1~ch8 中除了 ch7 以外的所有用户态游戏。

在所有游戏中，增加了以下功能以支持游戏运行：

+ 使用 `mmap` 来分配映射到 screen framebuffer 的内存。在未开启页表的时直接返回 framebuffer 物理地址，开启页表后则由内核从堆中动态分配一块内存并映射到 framebuffer 上。
+ 使用 `ioctl` 来获取屏幕分辨率以及刷新屏幕。
+ 使用 `open("/dev/fb0")` 和 `open("/dev/input0")` 来获取 screen 和 keyboard 的 file descriptor，screen 的 fd 可用于传入 `mmap` 和 `ioctl`。
+ 使用 `read` 传入 keyboard 的 fd 以读取键盘事件。
+ 使用 `mmap(..., MAP_SHARED, ...)` 以使用共享内存在父进程和子进程之间通信。这需要修改 `AddressSpace` 以支持 private 和 shared 两种不同的 memory 段，在 `fork()` 的时候只有 private memory 段需要重新分配物理内存。

创建了 `tg-driver` crate，该 crate 会扫描所有 virtio 地址以发现设备，封装屏幕、鼠标、磁盘设备，设置 PLIC 以响应外部中断。内核在收到 `ExternalInterrupt` 中断的时候会将委托给 `tg-driver` 进行处理。

### 实现亮点

+ ch1-tangram：从头重写 kernel，使其支持执行单个用户态应用。

+ ch5-pingpong：由主进程 fork 出来两个子进程，两个子进程负责监测键盘事件以更新球拍的运动状态（正在上移/下移），主进程负责计算球和球拍位置以及检测碰撞。

+ ch6-breakout：实现一个 `File` trait，将磁盘文件的 `FileHandle` 封装为 `DiskFile`，将 GPU 和 keyboard 封装为 `GpuFile` 和 `InputFile` 以统一处理，贯彻“Everything is a file”思想。

+ ch8-doom：使用 picolibc 来为用户程序提供库支持，将 Rust 的系统调用作为 picolibc 的 stub。

### 遇到过的问题

#### ch1

+ 应使用 `-sdl` 来启动 qemu 的图形界面模式
+ 内核开了较大的 framebuffer，占用空间较多，若用户程序仍链接在 0x80400000 则会导致内核与用户程序地址重叠
+ 内核栈开得过小，导致内核代码/数据被覆写，产生随机错误，症状为添加 `println!` 调试语句会导致发生 `IllegalInstruction` 的位置不一样
+ `__alltraps` 需要明确标记放在 `.text` 段
+ 用户程序被修改后内核不一定会重新被编译，原因是 `APP_ASM` 的值未改变（只是 `.incbin` 所包含的二进制文件变了），`cargo` 检测到内核代码没有任何改变，故不重新编译。解决方案：在 `APP_ASM` 开头用注释标出用户程序的哈希值

#### ch2

+ 即使使用了 guard byte 以检测栈溢出，仍可能发生栈溢出过于严重以至于 guard byte 未被覆写的情况

#### ch3

+ 每次重新绘制整个屏幕会导致游戏极其卡顿，FPS 只有 5~6，可改为每次只重新绘制有变化的像素

#### ch4

+ 需要增加 `VirtIO` 地址段的虚存映射

#### ch5

+ （AI 重写的）user shell 没有给输入的程序名的末尾加上 `\0`，而 kernel 错误地使用 `\0` 来判断字符串的结尾。后来学到 `\0` 在 Rust 内核中属于 anti-pattern


#### ch8

+ 一般情况下编译用户程序搜得到的 `data` 段和 `bss` 段的页面可能有重叠，原有 `from_elf` 无法处理该情况。解决方案：让 AI 重写 `from_elf`，让相交的页面单独成为一个 `map_area`，权限取并。
+ 需要明确提供自己的 `crt0.S` 启动脚本，否则 picolibc 会使用自己的 `crt0.S` 启动脚本。
+ picolibc 使用 `tp` 寄存器作为 TLS（thread local storage）的起始地址，而系统调用出错的 `errno` 是被储存在 TLS 中的，当前 kernel 没有初始化 `tp` 寄存器，会导致在地址 0 附近的 `StorePageFault`。
+ 跳转到 S 态时需要先对 `MPP` 进行 `csrc` 再 `csrs`。

## 发现的 tg-rcore-tutorial 的问题：

+ kernel-alloc 和 user crate 中在 customizable-buddy 外包了一层作为 global allocator，但 customizable-buddy 的 `allocate_layout` 函数实现有误，导致分配的内存不一定满足 `align` 的对齐要求，从而会导致内核和用户态程序的 bug。

+ 原本的实现中，`open` `read` `write` 等函数没有检测试图读取或写入的内存是否是用户可见的（`translate` 的 flag 没有传入 `U` 标记），存在用户程序读取、覆写内核跳板代码的风险。

+ ch2 的内核中先执行 `fence.i` 再拷贝用户程序，应改为后执行 `fence.i`。
