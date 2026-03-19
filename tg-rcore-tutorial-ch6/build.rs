use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
};
use tg_easy_fs::{BlockDevice, EasyFileSystem};

const TARGET_ARCH: &str = "riscv64gc-unknown-none-elf";
const BLOCK_SZ: usize = 512;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-env-changed=TG_GAME_DIR");
    println!("cargo:rerun-if-env-changed=TG_GAME_CRATE");
    println!("cargo:rerun-if-env-changed=TG_GAME_LOCAL_DIR");
    println!("cargo:rerun-if-env-changed=TG_GAME_CACHE");
    println!("cargo:rerun-if-env-changed=TG_GAME_VERSION");
    println!("cargo:rerun-if-env-changed=TG_GAME_APPS");
    println!("cargo:rerun-if-env-changed=TG_GAME_BASE");
    println!("cargo:rerun-if-env-changed=TG_GAME_STEP");

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    if target_arch == "riscv64" {
        write_linker();
        if should_skip_build_apps() {
            return;
        }
        build_game_apps();
    }
}

fn should_skip_build_apps() -> bool {
    if env::var_os("TG_SKIP_USER_APPS").is_some() {
        return true;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let manifest_dir = manifest_dir.to_string_lossy();
    manifest_dir.contains("/target/package/") || manifest_dir.contains("\\target\\package\\")
}

fn write_linker() {
    let ld = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("linker.ld");
    fs::write(&ld, tg_linker::NOBIOS_SCRIPT)
        .unwrap_or_else(|err| panic!("failed to write linker script to {}: {}", ld.display(), err));
    println!("cargo:rustc-link-arg=-T{}", ld.display());
}

fn build_game_apps() {
    let game_dir = ensure_tg_game();
    println!(
        "cargo:rerun-if-changed={}",
        game_dir.join("Cargo.toml").display()
    );
    println!("cargo:rerun-if-changed={}", game_dir.join("src").display());

    // Read apps from environment variable (comma-separated list)
    let apps_str = env::var("TG_GAME_APPS").expect(
        "TG_GAME_APPS not set; add it to .cargo/config.toml [env] as a comma-separated list",
    );

    let names: Vec<String> = apps_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if names.is_empty() {
        panic!("no game apps found in TG_GAME_APPS environment variable");
    }

    // Parse base and step (supports hex like "0x80400000" or decimal)
    let base = parse_env_u64("TG_GAME_BASE").unwrap_or(0);
    let step = parse_env_u64("TG_GAME_STEP").unwrap_or(0);

    let target_dir = game_dir.join("target").join(TARGET_ARCH).join("debug");

    let mut built_apps: Vec<(String, PathBuf)> = Vec::with_capacity(names.len());

    for (i, name) in names.iter().enumerate() {
        let base_address = base + i as u64 * step;

        // Build the game binary
        let mut cmd = Command::new("cargo");
        cmd.args([
            "build",
            "--manifest-path",
            game_dir.join("Cargo.toml").to_string_lossy().as_ref(),
            "--bin",
            name,
            "--target",
            TARGET_ARCH,
        ]);

        if base_address != 0 {
            cmd.env("BASE_ADDRESS", base_address.to_string());
        }

        let status = cmd
            .status()
            .expect("failed to execute cargo build for game app");

        if !status.success() {
            panic!("failed to build game app {name}");
        }

        let elf = target_dir.join(name);
        
        // Retaining your original logic: if base_address != 0, it likely needs to be a raw binary
        let app_path = if base_address != 0 {
            objcopy_to_bin(&elf)
        } else {
            elf
        };
        built_apps.push((name.clone(), app_path));
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let fs_target_dir = manifest_dir
        .join("target")
        .join(TARGET_ARCH)
        .join("debug");
    
    // Pack the built apps into easy-fs
    easy_fs_pack(&built_apps, &fs_target_dir).unwrap_or_else(|err| {
        panic!(
            "failed to pack easy-fs image in {}: {err}",
            fs_target_dir.display()
        )
    });
}

struct BlockFile(Mutex<std::fs::File>);

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
    apps: &[(String, PathBuf)],
    out_dir: &PathBuf,
) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Read;

    fs::create_dir_all(out_dir)?;
    let fs_file = out_dir.join("fs.img");
    println!("cargo:warning=fs.img can be found at {:?}", fs_file);
    println!("cargo:rerun-if-changed={}", fs_file.display());
    
    let block_file = Arc::new(BlockFile(Mutex::new({
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&fs_file)?;
        f.set_len(64 * 2048 * BLOCK_SZ as u64).unwrap();
        f
    })));

    let efs = EasyFileSystem::create(block_file, 64 * 2048, 1);
    let root_inode = Arc::new(EasyFileSystem::root_inode(&efs));

    for (name, path) in apps {
        let mut host_file = std::fs::File::open(path)?;
        let mut all_data: Vec<u8> = Vec::new();
        host_file.read_to_end(&mut all_data)?;
        let inode = root_inode.create(name.as_str()).unwrap();
        inode.write_at(0, all_data.as_slice());
    }

    Ok(())
}

fn parse_env_u64(var_name: &str) -> Option<u64> {
    env::var(var_name).ok().and_then(|s| {
        let s = s.trim();
        if s.starts_with("0x") || s.starts_with("0X") {
            u64::from_str_radix(&s[2..], 16).ok()
        } else {
            s.parse().ok()
        }
    })
}

fn objcopy_to_bin(elf: &PathBuf) -> PathBuf {
    let bin = elf.with_extension("bin");
    let status = Command::new("rust-objcopy")
        .args([
            elf.to_string_lossy().as_ref(),
            "--strip-all",
            "-O",
            "binary",
            bin.to_string_lossy().as_ref(),
        ])
        .status()
        .expect("failed to execute rust-objcopy");
    if !status.success() {
        panic!("rust-objcopy failed for {}", elf.display());
    }
    bin
}

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
        println!(
            "cargo:warning=Using local copy of crate {} in path {}",
            crate_name, local_dir_name
        );
        if target_dir.exists() {
            let _ = fs::remove_dir_all(&target_dir);
        }
        let status = std::process::Command::new("cp")
            .args([
                "-r",
                local_dir.to_string_lossy().as_ref(),
                target_dir.to_string_lossy().as_ref(),
            ])
            .status()
            .unwrap_or_else(|e| panic!("failed to execute cp: {e}"));

        if !status.success() {
            panic!(
                "failed to copy {} to {}",
                local_dir.display(),
                target_dir.display()
            );
        }

        let copied_target = target_dir.join("target");
        if copied_target.exists() {
            let _ = fs::remove_dir_all(&copied_target);
        }
    } else if !target_dir.join("Cargo.toml").exists() {
        println!(
            "cargo:warning=Pulling crate {} with cargo clone",
            crate_name
        );
        let crate_spec = format!("{crate_name}@{version}");
        let status = std::process::Command::new("cargo")
            .args([
                "clone",
                crate_spec.as_str(),
                "--",
                target_dir.to_string_lossy().as_ref(),
            ])
            .status()
            .unwrap_or_else(|e| panic!("failed to execute cargo clone {crate_spec}: {e}"));

        if !status.success() {
            panic!(
                "failed to clone {crate_spec} into {}; ensure cargo-clone is installed",
                target_dir.display()
            );
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

fn ensure_workspace_table(dir: &PathBuf) {
    let cargo_toml = dir.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
    if !content.contains("[workspace]") {
        fs::write(&cargo_toml, format!("{}\n[workspace]\n", content)).unwrap_or_else(|err| {
            panic!("failed to patch Cargo.toml in {}: {}", dir.display(), err)
        });
    }
}