# Chapter 5: Execution Flow Trace (Process Management)

*Note: Steps identical to Chapter 4 (e.g., `_start`, detailed `kernel_space` setup, and the naked assembly of `execute` / `execute_naked` / `foreign_execute`) are abbreviated. For detail on the context switch mechanics, refer to `docs/artifacts/execution_flow/ch4.md`.*

```text
[Boot] _start (Assembly)
|-- State: Initializes boot stack pointer (`sp` = `STACK + STACK_SIZE`).
|-- JUMP to: The memory address of `rust_main`.
    |-- [Init] rust_main() (Rust)
    |   |-- (Abbreviated: Zero BSS, init console, init alloc, calculate portal, `kernel_space`, and init `MultislotPortal`)
    |   |-- [Init] tg_syscall::init_*() (io, process, scheduling, clock, memory)
    |   |   |-- State: Mounts syscall handlers (now handles `fork`, `exec`, `wait`, `mmap`, etc.).
    |   |   |-- RET: Returns to `rust_main`.
    |   |-- [Init] Process::from_elf() (For `initproc`)
    |   |   |-- State: Creates an empty `AddressSpace`.
    |   |   |-- State: Parses `initproc`'s ELF and maps LOAD segments.
    |   |   |-- State: Allocates and maps user stack to high virtual address.
    |   |   |-- State: Appends transit portal to address space.
    |   |   |-- State: Initializes `LocalContext` & `satp`.
    |   |   |-- State: Allocates a unique `PID` for `initproc`.
    |   |   |-- RET: Returns the initial `Process` object.
    |   |-- [Init] ProcManager::new()
    |   |   |-- State: Initializes `PROCESSOR` with a new `ProcManager`.
    |   |   |-- State: Adds the `initproc` `Process` into `PROCESSOR` (parent PID = MAX).
    |   |-- [Task] Loop over `PROCESSOR.get_mut().find_next()`
            |-- State: Retrieves the next ready task from the `ProcManager` scheduling queue (e.g., via Stride/FIFO scheduling).
            |-- [Task] ForeignContext::execute()
                |-- (Abbreviated: Context switch into User Space via `execute_naked` and `foreign_execute`)
             
            ... (User application executes and traps) ...
            
            |-- (Abbreviated: Context switches back to kernel via `foreign_execute` -> `execute_naked` -> Rust)
            |-- State: Returns back to `rust_main` loop.
            |-- State: Reads trap cause from `scause`.
            |-- [Trap] tg_syscall::handle()
                |-- State: Evaluates the syscall and advances `sepc` by 4.
                |-- State: Differentiates based on syscall id:
                    |-- If EXIT:
                        |-- State: Calls `(*processor).make_current_exited(ret)`. Marks the entity as a zombie to be collected by the parent. Wait queue is notified.
                    |-- For all other continuing syscalls (e.g., FORK, EXEC, YIELD):
                        |-- State: Updates return value in `a0`.
                        |-- State: Calls `(*processor).make_current_suspend()`. 
                            |-- State: Yields CPU, moving the process to the back of the ready queue (or re-evaluating its stride/priority).
            |-- Loop iterates back to `find_next()` to select the next process.
```

## System Call Deep Dives

### `fork`
```text
[Syscall] fork()
|-- State: Acquires `current` Process.
|-- [Init] Process::fork()
|   |-- State: Allocates new `PID`.
|   |-- State: Deep clones `AddressSpace` (all page tables + physical data).
|   |-- State: Maps portal into duplicate `AddressSpace`.
|   |-- State: Clones general registers (`context.clone()`).
|   |-- State: Creates new `ForeignContext` with identical registers but independent `satp`.
|   |-- RET: Returns the cloned child `Process`.
|-- State: Forces child `a0` to 0 (return value in the child's perspective).
|-- State: Appends child to `PROCESSOR` with `parent_pid = current.pid`.
|-- RET: Returns the child's `PID` to the caller.
```

### `exec`
```text
[Syscall] exec()
|-- State: Acquires the executable path from user memory using `translate()`.
|-- State: Finds the ELF binary data matching the path name in the `APPS` registry.
|-- [Init] Process::from_elf()
|   |-- State: Initializes a fresh `Process` with a new `AddressSpace` and generic `LocalContext`.
|   |-- RET: Returns the fresh `Process` object.
|-- State: Overwrites the current process's `address_space`, `context`, `heap_bottom`, and `program_brk`.
|-- State: Implicitly, when replacing the Struct, Rust's drop semantics recycle the old `AddressSpace` memory (or it relies on garbage collection / explicit unmaps). The `PID` is preserved.
|-- RET: Returns 0.
```

### `wait`
```text
[Syscall] wait()
|-- State: Checks if `.wait()` conditions are met in `PManager`.
|-- [Task] ProcManager::wait()
|   |-- If child found and is Zombie (exited):
|       |-- State: Collects child `PID` and `exit_code`. Removes the child from the memory tracker/task list.
|       |-- State: Writes the `exit_code` to the waiting parent's pointer in memory via `translate()`.
|       |-- RET: Returns child `PID` to the parent.
|   |-- If wait condition not met:
|       |-- RET: Returns -1.
```
