# Chapter 4: Execution Flow Trace

```text
[Boot] _start (Assembly)
|-- State: Initializes boot stack pointer (`sp` = `STACK + STACK_SIZE`).
|-- JUMP to: The memory address of `rust_main`.
    |-- [Init] rust_main() (Rust)
    |   |-- [Init] KernelLayout::zero_bss()
    |   |   |-- State: Zeroes out memory in the BSS segment.
    |   |   |-- RET: Returns to `rust_main`.
    |   |-- [Init] tg_console::init_console()
    |   |   |-- State: Initializes the console output system.
    |   |   |-- RET: Returns to `rust_main`.
    |   |-- [Init] tg_console::set_log_level()
    |   |   |-- State: Sets the global logging level.
    |   |   |-- RET: Returns to `rust_main`.
    |   |-- [Init] tg_kernel_alloc::init()
    |   |   |-- State: Initializes the kernel heap allocator starting at the kernel image end.
    |   |   |-- RET: Returns to `rust_main`.
    |   |-- [Init] tg_kernel_alloc::transfer()
    |   |   |-- State: Transfers available physical memory to the allocator.
    |   |   |-- RET: Returns to `rust_main`.
    |   |-- [Init] kernel_space()
    |   |   |-- [Init] AddressSpace::new()
    |   |   |   |-- State: Allocates and initializes an empty Sv39 page table.
    |   |   |   |-- RET: Returns to `kernel_space`.
    |   |   |-- State: Maps kernel text, rodata, data, boot, heap, and portal to physical memory.
    |   |   |-- [Init] satp::set()
    |   |   |   |-- State: Writes the kernel root page table PPN to the `satp` CSR, activating Sv39 paging.
    |   |   |   |-- RET: Returns to `kernel_space`.
    |   |   |-- RET: Returns the initialized kernel `AddressSpace` to `rust_main`.
    |   |-- [Init] Process::new() (Called in a loop for each ELF application)
    |   |   |-- State: Creates an empty `AddressSpace` for the user program.
    |   |   |-- State: Parses the ELF file and maps its LOAD segments into the user `AddressSpace`.
    |   |   |-- State: Allocates user stack pages and maps them to a high virtual address.
    |   |   |-- State: Initializes a user `LocalContext` (sets `sepc` to program entry, configures `sp`).
    |   |   |-- State: Constructs the user `satp` value.
    |   |   |-- RET: Returns the initialized user `Process` to `rust_main`.
    |   |-- State: Appends the transit portal kernel mapping to the user's `AddressSpace` and pushes `Process` to `PROCESSES`.
    |   |-- State: Allocates and maps the `schedule` thread's stack to high virtual memory in the kernel space.
    |   |-- [Init] LocalContext::thread()
    |   |   |-- State: Initializes a kernel scheduling `LocalContext` (sets Supervisor privileged mode, disables interrupts, sets target `sepc` to `schedule()`).
    |   |   |-- RET: Returns the initialized `LocalContext` to `rust_main`.
    |   |-- [Syscall] LocalContext::execute() (For the scheduling thread)
    |       |-- State: Calculates `sstatus` indicating Supervisor mode.
    |       |-- [Syscall] execute_naked (Assembly)
    |           |-- State: Saves caller (rust_main) general registers to stack.
    |           |-- State: Swaps `sp` and `sscratch` (pointing to the scheduler's `LocalContext`).
    |           |-- State: Loads general registers and `sp` from the scheduler's `LocalContext`.
    |           |-- SRET: Drops to the address in `sepc` (landing in `schedule`).

[Task] schedule() (Rust)
|-- [Init] MultislotPortal::init_transit()
|   |-- State: Configures portal layout in the transit cache.
|   |-- RET: Returns the initialized `ForeignPortal` implementation.
|-- [Init] tg_syscall::init_*() (io, process, scheduling, clock, trace, memory)
|   |-- State: Mounts syscall handlers.
|   |-- RET: Returns to `schedule`.
|-- [Task] ForeignContext::execute() (Called in a loop over `PROCESSES`)
    |-- State: Modifies thread state to Supervisor + Interrupt Disabled.
    |-- State: Initializes `PortalCache` (sets target `satp`, `sepc`, `a0` and `sstatus`).
    |-- State: Replaces thread `sepc` with the public portal transit entry mapping. 
    |-- [Trap] LocalContext::execute()
        |-- State: Generates new `sstatus`.
        |-- [Syscall] execute_naked (Assembly)
            |-- State: Saves scheduler's general registers to stack.
            |-- State: Sets `stvec` to `1f` (the trap catcher within `execute_naked`).
            |-- State: Swaps `sp` and `sscratch` (pointing to `ForeignContext::context`).
            |-- State: Loads generic registers and user `sp` from the thread's `LocalContext`.
            |-- SRET: Drops to the address in `sepc` (landing in `foreign_execute` portal entry).
            |-- [Trap] foreign_execute (Assembly)
                |-- State: Swaps address space by writing `satp` to the user's root table.
                |-- State: Loads thread's `sstatus` and `sepc` from `PortalCache`.
                |-- State: Swaps `stvec` to `1f` (trap catcher within `foreign_execute`) and saves the old `stvec` to `PortalCache`.
                |-- State: Loads generic registers from `PortalCache`.
                |-- SRET: Drops privilege level to User mode. Returns to the user application entry address stored in the `sepc` register.

... (User Application executes, triggers a syscall or exception, and traps) ...

[Trap] foreign_execute (Assembly) (At internal label `1f` via `stvec`)
|-- State: Saves the trap entry `a0`, swaps `sscratch` to acquire the `PortalCache` pointer.
|-- State: Restores kernel address space by writing `satp` from the `PortalCache`.
|-- State: Restores `stvec` from `PortalCache`.
|-- JUMP to: The address held in `a0` (landing at label `1f` in `execute_naked`).
    |-- [Trap] execute_naked (Assembly) (At internal label `1f`)
        |-- State: Swaps `sp` with `sscratch`, restoring the thread's `LocalContext` pointer to `sscratch`.
        |-- State: Saves the trapped user context (all general registers) into the `LocalContext`.
        |-- State: Restores scheduler's `sp` from the stack.
        |-- State: Loads scheduler's general registers from stack.
        |-- RET: Returns to `LocalContext::execute()` inline assembly.
|-- [Task] LocalContext::execute() (Inline Assembly Continuation)
    |-- State: Restores the scheduler's original `sscratch` value.
    |-- State: Retrieves the trapped thread's `sepc` and `sstatus` CSRs.
    |-- RET: Returns trapped `sstatus` to `ForeignContext::execute()`.
|-- [Task] ForeignContext::execute()
    |-- State: Restores thread's original `supervisor` and `interrupt` options.
    |-- State: Reads the syscall return value/parameters from `PortalCache` into `LocalContext`.
    |-- RET: Returns the user `sstatus` to `schedule()`.
|-- State: Reads the trap cause from `scause`.
|-- [Task] tg_syscall::handle() (If Trap is purely UserEnvCall)
    |-- State: Processes the specific syscall. Changes program states (e.g. adjusts memory break or modifies fd).
    |-- State: For continuing tasks, advances `sepc` by 4 and updates return value.
    |-- RET: Returns execution control back to `schedule` loop, leading to the next `ForeignContext::execute()`.
```


## Context Switch Register Transitions (`sp` and `sscratch`)

```text
=== SP & SSCRATCH Transition Graph ===

        [sp]                                           [sscratch]
         |                                                 |
(1. Enter `execute()`)                                     |
Kernel Stack (Scheduler)                           Old Kernel `sscratch`
         |                                                 |
         | <--- csrrw old, sscratch, ctx                   v
         |                                       Target `LocalContext`
         |                                                 |
(3. Save Scheduler Context)                                |
Kernel Stack (Scheduler's TrapFrame)                       |
         |                                                 |
         | <--- sd sp, (t0); mv sp, t0                     |
         v                                                 |
Target `LocalContext`                                      |
         |                                                 |
         | <--- ld sp, 16(sp)                              |
         v                                                 |
Target Thread's Stack (User Stack)                         |
         |                                                 |
(6. `sret` to Target Thread)                               |
         |                                                 |
(7. Trap returns to `1f`)                                  |
         |                                                 |
         | <--- csrrw sp, sscratch, sp ------------------> |
         v                                                 v
Target `LocalContext`                              Target Thread's Stack
         |                                                 |
         | <--- csrrw t0, sscratch, sp ------------------> |
         |      sd t0, 16(sp)                              v
         |                                       Target `LocalContext`
         |                                                 |
         | <--- ld sp, (sp)                                |
         v                                                 |
Kernel Stack (Scheduler's TrapFrame)                       |
         |                                                 |
(11. Restore Scheduler Context)                            |
Kernel Stack (Scheduler)                                   |
         |                                                 |
         | <--- csrw sscratch, old_ss                      v
         |                                         Old Kernel `sscratch`
         v                                                 v
(12. Return to Rust)
```
