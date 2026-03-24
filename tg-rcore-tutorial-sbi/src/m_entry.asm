# `-bios none` 场景下的 M 态入口代码
# 当 QEMU 使用 `-bios none` 启动时，该代码在 M 态最先执行（常见入口 0x80000000）

    .section .text.m_entry
    .globl _m_start
_m_start:
    # 1) 检查 mhartid，并初始化 M 态栈
    csrr t0, mhartid
    li t1, {max_harts}
    bge t0, t1, _m_start_die # 超过支持的最大核数，直接挂起

    li t1, {m_stack_size}
    mul t0, t0, t1
    la sp, m_stack_top
    sub sp, sp, t0

    # 将 M 态栈顶保存到 mscratch，后续陷阱处理时用于切换栈
    csrw mscratch, sp

    # 2) 配置 mstatus：MPP=01（返回到 S 态），MPIE=1
    li t0, (1 << 11) | (1 << 7)
    csrw mstatus, t0

    # 3) 设置 mepc 为 S 态入口（由章节内核提供的 _start）
    la t0, _start
    csrw mepc, t0

    # 4) 设置 M 态陷阱向量
    la t0, m_trap_vector
    csrw mtvec, t0

    # 5) 中断委托给 S 态，但保留 MSIP(3) 和 MTIP(7) 在 M 态处理
    li t0, 0xffff
    li t1, (1 << 3) | (1 << 7)
    not t1, t1
    and t0, t0, t1
    csrw mideleg, t0

    li t0, 0xffff
    li t1, (1 << 9)     # 异常号 9：Environment call from S-mode
    not t1, t1
    and t0, t0, t1
    csrw medeleg, t0

    # 6) 配置 PMP：允许 S 态访问全部物理地址空间（教学简化）
    li t0, -1
    csrw pmpaddr0, t0
    li t0, 0x0f         # TOR 模式 + RWX
    csrw pmpcfg0, t0

    # 7) 允许 S 态读取计数器（如 time）
    li t0, -1
    csrw mcounteren, t0

    # 8) s-mode 参数：a0 设置为 hartid
    csrr a0, mhartid

    # 取消：直接 mret
    # 9) 仅 Hart 0 直接启动 S 态内核，其余核进入等待中断状态
    bnez a0, _park_hart
    mret

_park_hart:
    # 1. 允许接收 M 态软件中断 (MSIE = 第 3 位)
    li t0, (1 << 3)
    csrs mie, t0

    # 2. 开启全局 M 态中断允许 (MIE = 第 3 位)
    li t0, (1 << 3)
    csrs mstatus, t0

1:  wfi
    j 1b
    
_m_start_die:
    wfi
    j _m_start_die

    .section .text.m_trap
    .globl m_trap_vector
    .align 4
m_trap_vector:
    # 最小 M 态陷阱入口：主要处理来自 S 态的 ecall（SBI 调用）
    # 先切换到 M 态专用栈，避免污染 S 态栈
    csrrw sp, mscratch, sp
    addi sp, sp, -128

    # 保存将被修改的通用寄存器（构造 MachineTrapFrame）
    sd ra, 0(sp)
    sd t0, 8(sp)
    sd t1, 16(sp)
    sd t2, 24(sp)
    sd a0, 32(sp)
    sd a1, 40(sp)
    sd a2, 48(sp)
    sd a3, 56(sp)
    sd a4, 64(sp)
    sd a5, 72(sp)
    sd a6, 80(sp)
    sd a7, 88(sp)

    # 保存 mepc，供 Rust 进行修改
    csrr t0, mepc
    sd t0, 96(sp)

    # a0 传入当前栈指针，作为 &mut MachineTrapFrame 参数
    mv a0, sp

    # 调用 Rust 侧分发函数（msbi.rs::m_trap_handler）
    call m_trap_handler

    # 恢复 mepc（可能已被 Rust 代码修改）
    ld t0, 96(sp)
    csrw mepc, t0

    # 恢复所有寄存器（包含被 Rust 修改的返回值 a0, a1）
    ld ra, 0(sp)
    ld t0, 8(sp)
    ld t1, 16(sp)
    ld t2, 24(sp)
    ld a0, 32(sp)
    ld a1, 40(sp)
    ld a2, 48(sp)
    ld a3, 56(sp)
    ld a4, 64(sp)
    ld a5, 72(sp)
    ld a6, 80(sp)
    ld a7, 88(sp)

    addi sp, sp, 128
    # 切回原先（S 态侧）栈指针
    csrrw sp, mscratch, sp
    # 返回到触发 ecall 的 S 态上下文
    mret

    .section .bss.m_stack
    .globl m_stack_lower_bound
m_stack_lower_bound:
    # M 态专用栈，每个核分配 m_stack_size
    .space {m_stack_size} * {max_harts}
    .globl m_stack_top
m_stack_top:

    .section .bss.m_data
    # 预留少量 M 态数据区（当前实现未显式使用，便于后续扩展）
    .space 64
