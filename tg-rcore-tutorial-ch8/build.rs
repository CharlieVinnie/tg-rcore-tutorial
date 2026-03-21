use std::{env, fs, path::PathBuf, process::Command};
use tg_easy_fs::{BlockDevice, EasyFileSystem};

const BLOCK_SZ: usize = 512;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-env-changed=TG_SKIP_USER_APPS");
    println!("cargo:rerun-if-env-changed=TG_DOOMGENERIC_DIR");
    println!("cargo:rerun-if-env-changed=TG_FS_RESOURCES");

    let target_arch: String = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    if target_arch == "riscv64" {
        write_linker();
        if env::var_os("TG_SKIP_USER_APPS").is_some() {
            return;
        }
        build_and_pack();
    }
}

fn write_linker() {
    let ld: PathBuf = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("linker.ld");
    fs::write(&ld, tg_linker::NOBIOS_SCRIPT).unwrap_or_else(|err| {
        panic!("failed to write linker script to {}: {}", ld.display(), err)
    });
    println!("cargo:rustc-link-arg=-T{}", ld.display());
}

/// Parse "name=relative/path" entries from TG_FS_RESOURCES (semicolon-separated).
/// Returns Vec<(fs_name, host_absolute_path)>.
fn parse_resources(manifest_dir: &PathBuf) -> Vec<(String, PathBuf)> {
    let raw: String = env::var("TG_FS_RESOURCES").unwrap_or_default();
    raw.split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|entry| {
            let (name, rel_path) = entry.split_once('=').unwrap_or_else(|| {
                panic!(
                    "TG_FS_RESOURCES entry '{}' must be in 'name=path' format",
                    entry
                )
            });
            let abs_path: PathBuf = if rel_path.starts_with("@OUT_DIR@/") {
                PathBuf::from(env::var("OUT_DIR").unwrap()).join(&rel_path[10..])
            } else {
                manifest_dir.join(rel_path)
            };
            if !abs_path.exists() {
                panic!(
                    "TG_FS_RESOURCES: '{}' resolved to '{}' which does not exist",
                    entry,
                    abs_path.display()
                );
            }
            println!("cargo:rerun-if-changed={}", abs_path.display());
            (name.to_string(), abs_path)
        })
        .collect()
}

fn build_and_pack() {
    let manifest_dir: PathBuf = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());

    // --- Build doomgeneric via make ---
    if let Ok(doomgeneric_rel) = env::var("TG_DOOMGENERIC_DIR") {
        let doomgeneric_src: PathBuf = manifest_dir.join(&doomgeneric_rel);
        println!("cargo:rerun-if-changed={}", doomgeneric_src.display());
        // Also track the rcore stubs directory (sibling of doomgeneric source)
        let rcore_dir: PathBuf = doomgeneric_src.parent().unwrap().join("rcore");
        if rcore_dir.exists() {
            println!("cargo:rerun-if-changed={}", rcore_dir.display());
        }

        let out_dir = env::var("OUT_DIR").unwrap();
        let doomgeneric_out = PathBuf::from(&out_dir).join("doomgeneric");
        let doomgeneric_obj = PathBuf::from(&out_dir).join("doomgeneric_obj");

        let status: std::process::ExitStatus = Command::new("make")
            .args([
                "clean",
                "all",
                &format!("OBJDIR={}", doomgeneric_obj.display()),
                &format!("OUTPUT={}", doomgeneric_out.display()),
            ])
            .current_dir(&doomgeneric_src)
            .status()
            .expect("failed to execute make for doomgeneric");

        if !status.success() {
            panic!("failed to build doomgeneric");
        }
    }

    // --- Collect files entirely from TG_FS_RESOURCES ---
    let files: Vec<(String, PathBuf)> = parse_resources(&manifest_dir);
    if files.is_empty() {
        panic!("TG_FS_RESOURCES is empty or not set; nothing to pack into fs.img");
    }

    // --- Pack into fs.img ---
    let fs_target_dir: PathBuf = manifest_dir.join("target/riscv64gc-unknown-none-elf/debug");
    let files_ref: Vec<(&str, &PathBuf)> = files.iter().map(|(n, p)| (n.as_str(), p)).collect();

    easy_fs_pack(&files_ref, &fs_target_dir).unwrap_or_else(|err| {
        panic!(
            "failed to pack easy-fs image in {}: {err}",
            fs_target_dir.display()
        )
    });
}

struct BlockFile(std::sync::Mutex<std::fs::File>);

impl BlockDevice for BlockFile {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = self.0.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SZ) as u64))
            .expect("Error when seeking!");
        assert_eq!(file.read(buf).unwrap(), BLOCK_SZ, "Not a complete block!");
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = self.0.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SZ) as u64))
            .expect("Error when seeking!");
        assert_eq!(file.write(buf).unwrap(), BLOCK_SZ, "Not a complete block!");
    }
}

fn easy_fs_pack(
    files: &[(&str, &PathBuf)],
    fs_target: &PathBuf,
) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Read;
    use std::sync::Arc;

    fs::create_dir_all(fs_target)?;
    let fs_file: PathBuf = fs_target.join("fs.img");
    println!("cargo:rerun-if-changed={}", fs_file.display());

    // Calculate required size: doom1.wad is ~4MB, so use enough blocks
    // 64 * 2048 blocks * 512 = 64 MiB — should be plenty
    let total_blocks: u32 = 64 * 2048;

    let block_file = Arc::new(BlockFile(std::sync::Mutex::new({
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&fs_file)?;
        f.set_len(total_blocks as u64 * BLOCK_SZ as u64).unwrap();
        f
    })));

    let efs = EasyFileSystem::create(block_file, total_blocks, 1);
    let root_inode = Arc::new(EasyFileSystem::root_inode(&efs));

    for (name, path) in files {
        eprintln!("  packing: {} <- {}", name, path.display());
        let mut host_file = std::fs::File::open(path).unwrap_or_else(|e| {
            panic!("failed to open {}: {e}", path.display())
        });
        let mut all_data: Vec<u8> = Vec::new();
        host_file.read_to_end(&mut all_data).unwrap();
        let inode = root_inode.create(name).unwrap();
        inode.write_at(0, all_data.as_slice());
    }

    Ok(())
}