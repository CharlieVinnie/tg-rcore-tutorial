use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    
    // Cargo strongly discourages modifying CARGO_MANIFEST_DIR. 
    // The idiomatic workaround for OS development is to traverse up from OUT_DIR to the target profile directory.
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_dir = out_dir.parent().unwrap().parent().unwrap().parent().unwrap();
    let disk_img = target_dir.join("disk.img");

    if !disk_img.exists() {
        let file = std::fs::File::create(&disk_img).unwrap();
        file.set_len(64 * 1024 * 1024).unwrap();
    }
    if env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default() == "riscv64" {
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let ld = out_dir.join("linker.ld");
        fs::write(&ld, LINKER_SCRIPT).unwrap();
        println!("cargo:rustc-link-arg=-T{}", ld.display());
    }
}

const LINKER_SCRIPT: &[u8] = b"
OUTPUT_ARCH(riscv)
ENTRY(_m_start)
M_BASE_ADDRESS = 0x80000000;
S_BASE_ADDRESS = 0x80200000;
SECTIONS {
    . = M_BASE_ADDRESS;
    .text.m_entry : { *(.text.m_entry) }
    .text.m_trap  : { *(.text.m_trap)  }
    .bss.m_stack  : { *(.bss.m_stack)  }
    .bss.m_data   : { *(.bss.m_data)   }

    . = S_BASE_ADDRESS;
    .text   : {
        *(.text.entry)
        *(.text .text.*)
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
}
";
