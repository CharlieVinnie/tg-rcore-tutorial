# Goal Description
We will implement the `sys_trace` system call (ID `410`) to track system invocation history and support querying/mutating memory. The changes encompass extending [TaskControlBlock](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#27-36) to store syscall counters per task, correctly intercepting system call counts when [handle_syscall](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#82-122) executes, implementing `sys_trace` handling in `impls::Trace for SyscallContext` in [main.rs](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs), and appropriately expanding the kernel stack to store the extra array in the TCB struct.

## Proposed Changes

### Task Management Component
#### [MODIFY] src/task.rs
- Add `syscall_counts: [u32; 500]` field to [TaskControlBlock](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#27-36) struct to maintain tallies of system call types independently per running task. We choose an array size of 500 as it confidently engulfs standard and test syscall identifiers (up to 410).
- Reset this counter array to zero during initializations (`ZERO` and [init()](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#60-72) methods).
- Intercept the system call invocation routine [handle_syscall](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#82-122):
    - Increment `self.syscall_counts[id_usize]` for valid IDs (i.e., `< 500`).
    - Capture cases where `id_usize == 410` and argument [a(0)](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs#324-329) (which corresponds to `trace_request`) is `2`. Once `tg_syscall::handle` returns, override the return value in register [a(0)](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs#324-329) populated with the specific tally for the targeted query identifier `args[1]`.

### Kernel Entry & Components
#### [MODIFY] src/main.rs
- Expand the kernel stack variable `STACK_SIZE` within the [_start](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs#57-79) assembly entry point safely from [(APP_CAPACITY + 2) * 8192](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#60-72) to [(APP_CAPACITY + 16) * 8192](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#60-72) to evade stack overflow instances (prevent panic induced by augmenting struct sizes for 32 task copies).
- Implement logical paths in [trace()](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs#305-316) inside `impls::Trace for SyscallContext`:
    - Case `trace_request == 0`: Perform physical user memory reads by resolving `id as *const u8` inside `unsafe {}` and returning the read byte as `isize`.
    - Case `trace_request == 1`: Perform physical user memory writes executing byte-truncation `data & 0xff` and resolve pointer to `id as *mut u8`. Keep return 0 on success.
    - Case `trace_request == 2`: This case executes naturally through [TaskControlBlock](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#27-36) logic; hence [trace()](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs#305-316) simply emits baseline 0 value inside [main.rs](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/main.rs).
    - Catch-all fallback (`_`): Returns `-1`. Keep leading traits compliant by renaming variables without `_`.

## Verification Plan

### Automated Tests
- Running test script: Execute `./test.sh all` in terminal mode. Passing conditions mandate outputs acknowledging `OK!` and ending with `Passed!`. I will automate running the script locally and review terminal exit statuses and execution logging.
- Specific check: Verifying `cargo run --features exercise`. Test outcomes indicate proper interaction mapping between user inputs (`0`/`1`/`2`) and responses derived from `SYS_TRACE` calls internally via nested [syscall](file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch3/src/task.rs#82-122) logic.
