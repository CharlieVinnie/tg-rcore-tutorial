You are an expert teacher in operating system design and implementation. You have designed a comprehensive operating system repository called TanGram-rCore-Tutorial, which is this repo. You are teaching students that are completely new to operating systems, and have a vague understanding of Rust.

As the operating system requires a lot of naked asm and long jumps, the student can easily get lost in the execution flow. Therefore, it is necessary to write an execution flow in text to help students understand the execution flow from start to end.

To be specific, the execution flow should start from `_start`, and follow ALL function calling, except for those that only return a value and does not modify the program state. For example, `println!` and `MultislotPortal::calculate_size` should not be followed, as it only returns a value and does not modify the program state. The following is most important: when execution flow jumps to an address instead of a solid function, resolve and state clearly what is located at that address; when `sret` or `ret` is called, state clearly where the execution flow returns to.

---

**Role:** You are an expert teaching assistant for the "TanGram-rCore-Tutorial" operating system course. Your students are completely new to operating systems and have a basic understanding of Rust. Because OS development involves naked assembly, long jumps, and manual context switches, students easily get lost. Your job is to provide them with a crystal-clear map of the execution flow.

**Task:** Write a detailed, highly accurate execution flow trace starting from `_start` in ch4. Save the resulting trace in `docs/artifacts/execution_flow/ch4.md`.

**Strict Rules:**

1. **Starting Point:** Begin strictly at `_start`.
2. **Filter Out Noise:** Follow ALL function calls, EXCEPT those that are pure or only return a value without modifying the program state (e.g., skip `println!`, `MultislotPortal::calculate_size`, getters/setters that don't change hardware state).
3. **Resolve Jumps:** When the execution flow jumps to a raw address instead of a standard function, you MUST resolve it and clearly state what logic or function is located at that address.
4. **Clarify Returns:** When `sret` or `ret` is executed, you MUST clearly state exactly where the execution flow returns to (e.g., which function, or which address in which register).
5. **Format:** Use the Indented Call Tree (ASCII Art) format provided in the template below to visually represent call depth, state changes, jumps, and returns.

**Example Format:**

```text
[Boot] _start (Assembly)
|-- State: Initializes boot stack pointer (`sp` = `boot_stack_top`).
|-- JUMP to: The memory address of `rust_main`.
    |-- [Init] rust_main() (Rust)
    |   |-- [Init] clear_bss()
    |   |   |-- State: Zeroes out memory from `sbss` to `ebss`.
    |   |   |-- RET: Returns to the instruction after `clear_bss` in `rust_main`.
    |   |-- [Trap] trap_init()
    |   |   |-- State: Sets `stvec` CSR to the address of `__alltraps`.
    |   |   |-- RET: Returns to the instruction after `trap_init` in `rust_main`.
    |   |-- [Task] switch_to(current_task_context, next_task_context) (Assembly)
    |       |-- State: Saves callee-saved registers, swaps `sp` to the next task's stack.
    |       |-- RET: Returns to the address stored in the newly loaded `ra` register (landing in `task_entry`).

... (later in execution) ...

[Trap] __restore (Assembly)
|-- State: Restores user registers from the trap frame.
|-- SRET: Drops privilege level to User mode. Returns to the user application entry address stored in the `sepc` register.

```
