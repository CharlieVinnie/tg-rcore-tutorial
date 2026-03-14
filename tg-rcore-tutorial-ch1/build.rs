//! 构建脚本：为 RISC-V64 目标自动生成链接脚本。
//!
//! 链接脚本控制程序各段在内存中的布局，确保：
//! - M-mode 代码（tg-sbi）从 0x80000000 开始
//! - S-mode 代码（_start 入口）从 0x80200000 开始
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::{env, fs, path::PathBuf};

fn main() {
    use std::{env, fs, path::PathBuf, process::Command};

    let game_dir = ensure_tg_game();
    let game_bin = env::var("TG_GAME_BIN").expect("TG_GAME_BIN not set; add it to .cargo/config.toml [env]");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", game_dir.display());
    
    // 仅在交叉编译到 RISC-V64 时生成链接脚本
    if env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default() == "riscv64" {
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let ld = out_dir.join("linker.ld");
        fs::write(&ld, LINKER_SCRIPT).unwrap();
        // 告诉 rustc 使用此链接脚本
        println!("cargo:rustc-link-arg=-T{}", ld.display());

        // Games workspace ensured at top

        let base_address: usize = 0x80600000;

        // Build the games application
        let status = Command::new("cargo")
            .env("BASE_ADDRESS", base_address.to_string())
            .args([
                "build",
                "--manifest-path",
                game_dir.join("Cargo.toml").to_str().unwrap(),
                "--bin",
                game_bin.as_str(),
                "--target",
                "riscv64gc-unknown-none-elf"
            ])
            .status()
            .unwrap();

        if !status.success() {
            panic!("Failed to build user application");
        }

        // Objcopy to raw binary
        let games_elf = game_dir.join("target").join("riscv64gc-unknown-none-elf").join("debug").join(game_bin);
        let user_bin = out_dir.join("user.bin");

        let objcopy_status = Command::new("rust-objcopy")
            .args([
                games_elf.to_str().unwrap(),
                "--strip-all",
                "-O",
                "binary",
                user_bin.to_str().unwrap()
            ])
            .status()
            .unwrap();

        if !objcopy_status.success() {
            panic!("Failed to objcopy user application");
        }

        let bins = vec![user_bin.clone()];
        let mut hasher = DefaultHasher::new();

        for path in &bins {
            // 1. Tell Cargo to wake up and run build.rs if the file metadata changes
            println!("cargo:rerun-if-changed={}", path.display());

            // 2. Read the actual bytes of the binary and feed them into the hasher
            if let Ok(bytes) = fs::read(path) {
                bytes.hash(&mut hasher);
            } else {
                // Optional: Handle the error if the file is missing during the build script phase
                println!("cargo:warning=Could not read file for hashing: {}", path.display());
            }
        }

        // 3. Emit the final hash as a rustc environment variable.
        // This breaks the rustc cache ONLY when the contents of the binaries change.
        println!("cargo:rustc-env=BINS_CONTENT_HASH={}", hasher.finish());

        // Generate app.asm
        let app_asm = out_dir.join("app.asm");
        let asm_content = format!(
            r#"
    .global user_app
    .section .data
    .align 3
user_app_start:
    .incbin "{}"
user_app_end:
            "#,
            user_bin.display()
        );
        fs::write(&app_asm, asm_content).unwrap();

        println!("cargo:rustc-env=APP_ASM={}", app_asm.display());
    }
}

/// 链接脚本内容。
///
/// 内存布局：
///
/// ```text
/// 0x80000000  M-mode 区域（tg-sbi 提供）
///   .text.m_entry   M-mode 入口代码
///   .text.m_trap    M-mode 中断处理
///   .bss.m_stack    M-mode 栈空间
///   .bss.m_data     M-mode 数据
///
/// 0x80200000  S-mode 区域（本程序）
///   .text           代码段（含 .text.entry 入口）
///   .rodata         只读数据段
///   .data           可读写数据段
///   .bss            未初始化数据段（含栈空间）
/// ```
///
/// 注意：链接脚本是字节字符串，不能包含非 ASCII 字符，
/// 因此脚本内注释使用英文。
const LINKER_SCRIPT: &[u8] = b"
OUTPUT_ARCH(riscv)
ENTRY(_m_start)

/* M-mode code base address: start of RAM on QEMU virt platform */
M_BASE_ADDRESS = 0x80000000;
/* S-mode code base address: M-mode jumps here after init */
S_BASE_ADDRESS = 0x80200000;

SECTIONS {
    /* ===== M-mode region (provided by tg-sbi) ===== */
    . = M_BASE_ADDRESS;
    .text.m_entry : { *(.text.m_entry) }
    .text.m_trap  : { *(.text.m_trap)  }
    .bss.m_stack  : { *(.bss.m_stack)  }
    .bss.m_data   : { *(.bss.m_data)   }

    /* ===== S-mode region (this program) ===== */
    . = S_BASE_ADDRESS;
    .text   : {
        *(.text.entry)          /* _start entry, must come first */
        *(.text .text.*)        /* other code */
    }
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    .data   : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    . = ALIGN(4K);
    __boot_stack_bottom = .;
    .boot : {
        KEEP(*(.boot.stack))
    }
    __boot_stack_top = .;
    .bss    : {
        sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        ebss = .;
    }
    __kernel_end = .;
}";

fn ensure_tg_game() -> PathBuf {
    if let Ok(dir) = env::var("TG_GAME_DIR") {
        let path = PathBuf::from(dir);
        if path.join("Cargo.toml").exists() {
            return path;
        }
    }

    let crate_name = env::var("TG_GAME_CRATE")
        .expect("TG_GAME_CRATE not set; add it to .cargo/config.toml [env]");
    let local_dir_name = env::var("TG_GAME_LOCAL_DIR")
        .expect("TG_GAME_LOCAL_DIR not set; add it to .cargo/config.toml [env]");
    let cache_dir_name = env::var("TG_GAME_CACHE")
        .expect("TG_GAME_CACHE not set; add it to .cargo/config.toml [env]");
    let version = env::var("TG_GAME_VERSION")
        .expect("TG_GAME_VERSION not set; add it to .cargo/config.toml [env]");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_dir = out_dir.join(&cache_dir_name);

    let manifest_path = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let parent_dir = manifest_path.parent().unwrap();
    let local_dir = parent_dir.join(&local_dir_name);
    
    if local_dir.join("Cargo.toml").exists() {
        println!("cargo:warning=Using local copy of crate {} in path {}", crate_name, local_dir_name);
        if target_dir.exists() {
            let _ = fs::remove_dir_all(&target_dir);
        }
        let status = std::process::Command::new("cp")
            .args(["-r", local_dir.to_string_lossy().as_ref(), target_dir.to_string_lossy().as_ref()])
            .status()
            .unwrap_or_else(|e| panic!("failed to execute cp: {e}"));
            
        if !status.success() {
            panic!("failed to copy {} to {}", local_dir.display(), target_dir.display());
        }

        let copied_target = target_dir.join("target");
        if copied_target.exists() {
            let _ = fs::remove_dir_all(&copied_target);
        }
    } else if !target_dir.join("Cargo.toml").exists() {
        println!("cargo:warning=Pulling crate {} with cargo clone", crate_name);
        let crate_spec = format!("{crate_name}@{version}");
        let status = std::process::Command::new("cargo")
            .args(["clone", crate_spec.as_str(), "--", target_dir.to_string_lossy().as_ref()])
            .status()
            .unwrap_or_else(|e| panic!("failed to execute cargo clone {crate_spec}: {e}"));

        if !status.success() {
            panic!("failed to clone {crate_spec} into {}; ensure cargo-clone is installed", target_dir.display());
        }
    } else {
        println!("cargo:warning=Using cached crate {}", crate_name);
    }

    if !target_dir.join("Cargo.toml").exists() {
        panic!("failed to populate valid crate at {}", target_dir.display());
    }

    ensure_workspace_table(&target_dir);
    target_dir
}

/// 若 Cargo.toml 末尾尚无 [workspace] 表，则追加一个空的，
/// 使该 crate 成为独立 workspace 根，避免父 workspace 冲突。
fn ensure_workspace_table(dir: &PathBuf) {
    let cargo_toml = dir.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
    if !content.contains("[workspace]") {
        fs::write(&cargo_toml, format!("{}\n[workspace]\n", content))
            .unwrap_or_else(|err| panic!("failed to patch Cargo.toml in {}: {}", dir.display(), err));
    }
}
