Here's the full journey of hurdles, with evidence and fixes:

---

## Hurdle 1: picolibc Default Memory Layout Conflicts with Kernel

**Symptom:** `assertion failed: !pte.is_valid()` panic in `mapper.rs:37`

**Evidence:** `readelf -l doomgeneric` showed picolibc's default linker script places code at `0x80000000`:
```
LOAD  VirtAddr=0x80000000  MemSiz=0x3e098  R E   (text)
LOAD  VirtAddr=0x80200000  MemSiz=0x13490  RW    (data)
LOAD  VirtAddr=0x80213490  MemSiz=0x425e8  RW    (bss)
```

picolibc's [picolibc.ld](cci:7://file:///usr/lib/picolibc/riscv64-unknown-elf/lib/picolibc.ld:0:0-0:0) defaults:
```ld
flash (rx!w) : ORIGIN = DEFINED(__flash) ? __flash : 0x80000000,
ram   (w!rx) : ORIGIN = DEFINED(__ram)   ? __ram   : 0x80200000,
```

**Fix:** Override via `--defsym` in [Makefile](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/user/doomgeneric/doomgeneric/Makefile:0:0-0:0):
```makefile
LDFLAGS+=-Wl,--defsym,__flash=0x10000 -Wl,--defsym,__ram=0x200000
```

---

## Hurdle 2: Overlapping LOAD Segments Share a Page

**Symptom:** Same `assertion failed: !pte.is_valid()` panic, even after rebasing addresses.

**Evidence:** After rebasing, `readelf -l` showed:
```
LOAD  VirtAddr=0x200000  MemSiz=0x13490  RW  (data)  → pages 0x200..0x214
LOAD  VirtAddr=0x213490  MemSiz=0x425e8  RW  (bss)   → pages 0x213..0x256
```
Pages `0x213` and `0x213` overlap! `.data` ceils to `0x214`, `.bss` floors to `0x213`. This is because picolibc's linker script puts them in separate LOAD segments (`:ram_init` vs `:ram` phdrs) without page-aligning the boundary.

**Root cause analysis:** This is a kernel loader issue, not picolibc's fault. The ELF spec allows overlapping segments; Linux handles it gracefully. The kernel's [mapper.rs](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-kernel-vm/src/space/mapper.rs:0:0-0:0) had `assert!(!pte.is_valid())` which assumes every page is mapped exactly once.

**Fix:** In [tg-rcore-tutorial-games-kernel-vm/src/space/mapper.rs](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-kernel-vm/src/space/mapper.rs:0:0-0:0), replace the assert with a skip:
```rust
// Before:
assert!(!pte.is_valid());
*pte = self.flags.build_pte(self.range.start);
self.range.start += 1;

// After:
if pte.is_valid() {
    self.range.start += 1;  // Skip already-mapped page
} else {
    *pte = self.flags.build_pte(self.range.start);
    self.range.start += 1;
}
```

Also updated [Cargo.toml](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/Cargo.toml:0:0-0:0) to use local path:
```toml
tg-kernel-vm = { package = "tg-rcore-tutorial-games-kernel-vm", path = "../tg-rcore-tutorial-games-kernel-vm" }
```

---

## Hurdle 3: picolibc's crt0 Accesses M-mode CSRs

**Symptom:** `Exception(IllegalInstruction), sepc=0x10010, stval=0x0`

**Evidence:** Disassembly at `sepc=0x10010`:
```asm
0x10000 <_start>:
   10000:  auipc   sp, 0x3f0
   ...
   10010:  csrr    t0, mstatus    ← M-mode CSR! Illegal in U-mode
   10014:  lui     t1, 0x2
   ...
   1001e:  csrwi   fcsr, 0        ← Also M-mode
```

picolibc's [_start](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/main.rs:100:0-117:1) is designed for bare-metal M-mode. It reads `mstatus` to enable the FPU and clears `fcsr`. These are Machine-mode CSRs — illegal when the process runs in U-mode under an OS.

**Fix:** Created custom [user/doomgeneric/rcore/crt0.S](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/user/doomgeneric/rcore/crt0.S:0:0-0:0) and linked with `-nostartfiles`:
```asm
    .section .init, "ax"
    .globl _start
_start:
    la      tp, __tls_base    /* TLS init (see Hurdle 5) */
    la      a0, __bss_start   /* Zero .bss */
    la      a1, __end
    bgeu    a0, a1, 2f
1:  sd      zero, 0(a0)
    addi    a0, a0, 8
    bltu    a0, a1, 1b
2:  li      a0, 0             /* main(0, NULL) */
    li      a1, 0
    call    main
    li      a7, 93            /* exit syscall */
    ecall
```

```makefile
LDFLAGS+=-nostartfiles
SRC_DOOM = crt0.o dummy.o ...
```

---

## Hurdle 4: crt0 Data Copy Accesses Unmapped Flash LMA

**Symptom:** `Exception(LoadPageFault), sepc=0x1003c, stval=0x51000`

**Evidence:** `0x1003c` was the `lb a3, 0(a1)` instruction in crt0's data copy loop, trying to read from `__data_source` at `0x50ba0` (the flash LMA). This address was never mapped because the kernel's [from_elf](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/process.rs:110:4-211:5) maps segments at their **VMA** (virtual address), not LMA (load address).

picolibc's linker script uses `>ram AT>flash` for `.data`, meaning VMA is in [ram](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/process.rs:213:4-238:5) (`0x200000`) but LMA is in `flash` (`0x50ba0`). On bare metal, crt0 copies from flash→ram. Under an OS, the ELF loader already places data at the VMA — no copy needed.

**Fix:** Removed the data copy from [crt0.S](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/user/doomgeneric/rcore/crt0.S:0:0-0:0) entirely (see crt0 above — it only zeroes `.bss` and calls [main](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/build.rs:5:0-21:1)).

---

## Hurdle 5: TLS Not Initialized — errno Dereferences Null

**Symptom:** `Exception(StorePageFault), sepc=0x33afa, stval=0x0`

**Evidence:** `addr2line` showed `sepc=0x33afa` → `mkdir` in `syscall_stubs.c:50`, which sets `errno = ENOSYS`. In picolibc, `errno` is a TLS variable accessed via the [tp](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:29:4-31:5) register. With `-nostartfiles`, picolibc's TLS setup (which sets [tp](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:29:4-31:5)) was skipped, so `tp=0` and `errno` deref → null store.

**Fix:** Added `la tp, __tls_base` to [crt0.S](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/user/doomgeneric/rcore/crt0.S:0:0-0:0) before any C code runs:
```asm
_start:
    la      tp, __tls_base    /* picolibc errno uses tp-relative TLS */
```

---

## Hurdle 6: .data Content Lost Due to LOAD Segment Ordering

**Symptom:** `Exception(InstructionPageFault), sepc=0x0, stval=0x0` — null function pointer call.

**Evidence:** `ra=0x32544` → `W_AddFile` in `w_wad.c:153`. The code in `W_OpenFile` loads a function pointer from `stdc_wad_file` (at `0x213198` in `.data`):
```asm
32438:  auipc   a5, 0x1e1        # a5 = &stdc_wad_file (0x213198)
32446:  ld      a5, 0(a5)        # a5 = *stdc_wad_file → 0x0 (NULL!)
3244a:  jr      a5               # jump to NULL → fault
```

The ELF has `.bss` as **segment 02** and `.data` as **segment 03**. The kernel iterates segments in ELF order, so:
1. `.bss` (VirtAddr `0x213490`) maps first → page `0x213` allocated and **zeroed**
2. `.data` (VirtAddr `0x200000`) maps second → page `0x213` already mapped → **skipped** by our Hurdle 2 fix

The `.data` content for page `0x213` (including `stdc_wad_file` at `0x213198`) was never written — it stayed zero.

**Fix:** Sort LOAD segments by VirtAddr before mapping in [process.rs](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/process.rs:0:0-0:0):
```rust
// Before:
for program in elf.program_iter() {
    if !matches!(program.get_type(), Ok(program::Type::Load)) { continue; }
    ...
}

// After:
let mut load_segments: Vec<_> = elf
    .program_iter()
    .filter(|p| matches!(p.get_type(), Ok(program::Type::Load)))
    .collect();
load_segments.sort_by_key(|p| p.virtual_addr());

for program in &load_segments {
    ...
}
```

Now `.data` (`0x200000`) maps first with real file content, and `.bss` (`0x213490`) maps second — page `0x213` already has `.data` content, and the mapper skips it correctly.

---

## Hurdle 7: Missing [lseek](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/file.rs:67:4-85:5) Syscall

**Symptom:** `[ INFO] id = SyscallId(62)` → process killed as unsupported.

**Evidence:** Doom's `W_AddFile` opens the WAD file then calls [lseek()](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/file.rs:67:4-85:5) to seek within it. Syscall 62 = [lseek](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/file.rs:67:4-85:5) per `syscall.h.in`, but the kernel's [IO](cci:2://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:40:0-79:1) trait and dispatch table didn't include it.

**Fix:** Added [lseek](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/file.rs:67:4-85:5) to three places:
1. [IO](cci:2://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:40:0-79:1) trait in [tg-rcore-tutorial-games-syscall/src/kernel/mod.rs](cci:7://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:0:0-0:0)
2. Dispatch table in same file
3. [File](cci:2://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/file.rs:7:0-18:1) trait + [DiskFile](cci:2://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/file.rs:21:0-23:1) impl + `IO for SyscallContext` in the kernel

---

## Hurdle 8 (current): Missing `nanosleep` Syscall

**Symptom:** `[ INFO] id = SyscallId(101)` after full Doom initialization succeeds.

Doom fully initializes (WAD loaded, renderer started, status bar, HUD) and then calls `nanosleep` in its game loop via `DG_SleepMs()`. This is the next syscall to implement.

---

## Key Takeaways

1. **Embedded C toolchains assume bare-metal M-mode.** picolibc's crt0 does `csrr mstatus` — you must provide your own [_start](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-ch8/src/main.rs:100:0-117:1) for user-mode.

2. **The ELF spec allows overlapping LOAD segments.** OS loaders must handle shared pages gracefully. An `assert!(!pte.is_valid())` is too strict.

3. **LOAD segment order matters for shared pages.** If `.bss` maps before `.data`, the shared page gets zeroed and `.data` content is lost. Always sort by VirtAddr.

4. **picolibc's flash/RAM split creates two LOAD segments** for what would normally be one contiguous RW region. The `>ram AT>flash` linker directive separates VMA from LMA, which confuses naive ELF loaders.

5. **TLS must be explicitly initialized** when bypassing crt0. picolibc's `errno` is thread-local — without [tp](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:29:4-31:5) pointing to `__tls_base`, every `errno` access is a null dereference.

6. **Incremental debugging with `sepc`/`stval`/[ra](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:179:4-181:5)** is essential. Each register tells a different part of the story: `sepc` = where, `stval` = what address faulted, [ra](cci:1://file:///home/charlie/tg-rcore-tutorial/tg-rcore-tutorial-games-syscall/src/kernel/mod.rs:179:4-181:5) = who called.