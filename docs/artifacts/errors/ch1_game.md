Here is a summary of what went wrong previously, how we identified the issues, and how we fixed them:

### 1. The Missing QEMU Window
- **What went wrong:** The `cargo run` configuration in Ch1's [.cargo/config.toml](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch1/.cargo/config.toml:0:0-0:0) was explicitly set to use `-nographic` and had no GPU device attached, so QEMU was routing all output to your terminal and ignoring graphical requests.
- **How it was found:** I discovered this by checking [.cargo/config.toml](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch1/.cargo/config.toml:0:0-0:0) for the default runner command.
- **The fix:** I replaced `-nographic` with `-device virtio-gpu-device -display sdl -serial stdio` so a GUI window would spawn while console prints still routed to your terminal.

### 2. The Silent Exit / Crash Loop
- **What went wrong:** QEMU was silently exiting without printing anything except the initial `Hello` message. This was caused by two panics happening during the VirtIO GPU initialization:
  1. `VirtIOGpuWrapper::new` used `.unwrap()` blindly on `MmioTransport::new`. QEMU MMIO regions exist at 0x1000_1000 through 0x1000_8000, but not all of them are the GPU. The first address probed failed the Magic Value check and triggered a `.unwrap()` panic.
  2. Once we caught the error gracefully, `virtio-drivers` panicked on an internal assertion because our `HalImpl::dma_alloc` function returned an address that wasn't exactly 4096-byte (page) aligned.
- **How it was found:** I added a custom `println!` macro and registered a [panic_handler](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-user/src/lib.rs:33:0-43:1) with QEMU's `shutdown(true)` hook. I also registered an [early_trap_handler](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch1/src/main.rs:144:0-152:1) to catch exceptions before the OS had fully booted. This allowed the kernel to print the exact file, line number, and error message causing the crash (`assertion `left == right` failed`).
- **The fix:** I handled the MMIO error gracefully using `.map_err()` instead of `unwrap()`, and I forced our [dma_alloc](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch1/src/main.rs:88:4-104:5) buffer to perfectly align to page boundaries using `#[repr(align(4096))]`.

### 3. User Program Linking Address
- **What went wrong:** Initially, the raw binary produced for `ch1b_os` didn't have its [_start](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-user/src/lib.rs:16:0-26:5) instructions at the very beginning of the file (it had `.rodata` first), which meant jumping to `0x80400000` was attempting to execute data instead of code.
- **How it was found:** I ran `rust-objdump -h` to look at the sections inside the compiled ELF, and noticed the default compiler puts things in an arbitrary order without a linker script.
- **The fix:** I discovered that the `tg-rcore-tutorial-user` workspace automatically generates a RISC-V linker script if you pass the `BASE_ADDRESS` environment variable. I updated the `cargo build` command in [build.rs](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch2/build.rs:0:0-0:0) to pass `BASE_ADDRESS=2151677952` (0x80400000), which correctly placed `.text` at the front.

---

**Current Status:**
With these fixes, the kernel successfully finds the VirtIO GPU, initializes it, prepares the user program, and executes an `sret` to jump to U-mode at `0x80400000`. 
However, the user program immediately traps with an **Illegal Instruction** exception (`scause: Exception(IllegalInstruction)`). Our debugging prints show that the first 2 instructions at the entrypoint are valid RISC-V instructions (`addi sp, sp, -16` and `sd ra, 8(sp)`), so the `IllegalInstruction` is likely caused by the U-mode environment itself (e.g., trying to execute an instruction it doesn't have permissions for, or a misconfigured `sstatus` / stack). 

Let me know if you would like to proceed with diagnosing the `IllegalInstruction` exception!

---
---
---

It looks like you’ve run into a classic pitfall when mixing inline assembly with Rust! Your assembly didn’t get optimized away; it just got placed in the wrong neighborhood.

If you look closely at your symbol output, you'll see the culprit:
`Section: .data (0x7)`

Your `__alltraps` label (and all the instructions following it) was placed into the `.data` section instead of the executable `.text` section.

### Why did this happen?

When you use `core::arch::global_asm!`, you are injecting raw assembly strings directly into the compiler's output. If you don't explicitly tell the assembler which section to place the code in, it will just append it to whatever section was active at that moment during compilation (which, in this case, happened to be `.data`).

Because it ended up in `.data`:

1. **`rust-objdump` ignores it:** By default, objdump only disassembles executable sections like `.text`. It won't try to decode the `.data` section as instructions.
2. **The CPU sees garbage:** The `0002 illegal` instruction you are seeing in QEMU is likely just zero-padding, data, or compressed instruction fragments being misinterpreted by the CPU as it blindly tries to execute memory from the data section.

### The Fix

You need to explicitly declare the `.text` section at the very top of your `global_asm!` block.

Update your code to look like this:

```rust
core::arch::global_asm!(
    "
    .section .text      # <--- ADD THIS LINE
    .align 2
    .global __alltraps
    __alltraps:
        # Save essential scratch registers to TRAP_CONTEXT
        la t0, {trap_context}
        
        # Save x1 (ra)
        sd x1, 1*8(t0)
        
        # ... (rest of your assembly) ...
    ",
    trap_context = sym TRAP_CONTEXT,
);

```

### A Quick Note on `.align`

You used `.align 2`, which in the RISC-V GNU/LLVM assembler correctly aligns the code to a 4-byte boundary ($2^2 = 4$). This is perfect for RISC-V since standard instructions are 32 bits (4 bytes) wide, and the `stvec` register requires the trap vector address to be 4-byte aligned.

---

Once you add `.section .text`, rebuild your kernel, and run `rust-objdump -d` again, you should see your `__alltraps` function beautifully disassembled exactly where it belongs.

Would you like me to take a look at your `TRAP_CONTEXT` saving and restoring logic to make sure no registers are slipping through the cracks during the U-mode transition?

---
---
---

Kernel stack is too SMALL.

---
---
---

You have hit one of the most notorious and frustrating edge cases in Rust OS development!

The short answer is **yes, it is being cached**, but it's not `APP_ASM` or Cargo doing the caching—it's `rustc`'s incremental compiler getting tricked by `.incbin`.

Here is the exact sequence of why your OS stubbornly refuses to update:

1. You delete the folder. Cargo notices and re-runs `build.rs`.
2. `build.rs` perfectly compiles the new games, creates a fresh `user.bin`, and overwrites `app.asm`.
3. `app.asm` contains the exact same text as the last build: `.incbin "/path/to/user.bin"`. The path string hasn't changed.
4. Cargo asks `rustc` to compile your main OS crate.
5. `rustc` hashes the contents of `app.asm`. Because the *text* of `app.asm` is identical to the previous run, `rustc`'s incremental compiler says, "This code hasn't changed at all! I'll just reuse the cached object file to save time."
6. **The fatal flaw:** The Rust compiler has no idea what `.incbin` does. That is an instruction for the LLVM assembler. Because `rustc` skips compiling that module, LLVM is never invoked, and the new `user.bin` is never read from your disk. The old, cached object file (with the old binary baked inside) gets linked into your final OS.

Only `cargo clean` works because it destroys the `rustc` incremental cache, forcing it to invoke LLVM, which finally reads the new `.incbin` payload.

### The Fix: The "Cache Buster"

To force `rustc` to throw away its cached object file, we have to make sure the *text content* of `app.asm` changes every time a new `user.bin` is built. We can do this by appending the file's modification timestamp as a comment inside the assembly file.

Replace the section in your `build.rs` that generates `app.asm` with this:

```rust
        // Generate app.asm
        let app_asm = out_dir.join("app.asm");
        
        // 1. Get the exact modification time of the newly built user.bin
        let mtime = fs::metadata(&user_bin)
            .and_then(|m| m.modified())
            .map(|time| time.duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
            .unwrap_or(0);

        // 2. Inject it as a comment into the assembly. 
        // This forces rustc's incremental compiler to see a "new" file.
        let asm_content = format!(
            r#"
    # Cache buster: {}
    .global user_app
    .section .data
    .align 3
user_app_start:
    .incbin "{}"
user_app_end:
            "#,
            mtime,
            user_bin.display()
        );
        fs::write(&app_asm, asm_content).unwrap();

        println!("cargo:rustc-env=APP_ASM={}", app_asm.display());

```

Now, whenever a new `user.bin` is created, the timestamp in the `# Cache buster: ...` comment will change. `rustc` will see different text, invalidate the cache, and properly link your fresh user applications into the OS.

Would you like me to show you how to completely replace the `app.asm` assembly file with Rust's native `include_bytes!` macro, which handles caching automatically without needing this workaround?