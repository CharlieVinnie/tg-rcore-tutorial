use serde::Deserialize;
use std::{collections::HashMap, env, fs, path::PathBuf, process::Command};

const TARGET_ARCH: &str = "riscv64gc-unknown-none-elf";

#[derive(Deserialize, Default)]
struct Cases {
    base: Option<u64>,
    step: Option<u64>,
    cases: Option<Vec<String>>,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-env-changed=TG_GAMES_DIR");
    println!("cargo:rerun-if-env-changed=TG_GAMES_CRATE");
    println!("cargo:rerun-if-env-changed=TG_GAMES_LOCAL_DIR");
    println!("cargo:rerun-if-env-changed=TG_GAMES_VERSION");

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    if target_arch == "riscv64" {
        write_linker();
        if should_skip_build_apps() {
            write_dummy_app_asm();
        } else {
            build_game_apps();
        }
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
    fs::write(&ld, tg_linker::NOBIOS_SCRIPT).unwrap_or_else(|err| {
        panic!("failed to write linker script to {}: {}", ld.display(), err)
    });
    println!("cargo:rustc-link-arg=-T{}", ld.display());
}

fn build_game_apps() {
    let games_dir = ensure_tg_games();
    let cases_path = games_dir.join("cases.toml");
    println!("cargo:rerun-if-changed={}", cases_path.display());
    println!("cargo:rerun-if-changed={}", games_dir.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", games_dir.join("src").display());

    let cfg = fs::read_to_string(&cases_path).unwrap_or_else(|err| {
        panic!(
            "failed to read games cases.toml from {}: {}",
            cases_path.display(),
            err
        )
    });
    let mut cases_map: HashMap<String, Cases> = toml::from_str(&cfg).unwrap_or_else(|err| {
        panic!("failed to parse games cases.toml: {err}")
    });

    let cases = cases_map.remove("ch3").unwrap_or_default();
    let base = cases.base.unwrap_or(0);
    let step = cases.step.unwrap_or(0);
    let names = cases.cases.unwrap_or_default();

    if names.is_empty() {
        panic!("no game cases found for ch3 in {}", cases_path.display());
    }

    let target_dir = games_dir
        .join("target")
        .join(TARGET_ARCH)
        .join("debug");

    let mut bins: Vec<PathBuf> = Vec::with_capacity(names.len());

    for (i, name) in names.iter().enumerate() {
        let base_address = base + i as u64 * step;

        // Build the game binary
        let mut cmd = Command::new("cargo");
        cmd.args([
            "build",
            "--manifest-path",
            games_dir.join("Cargo.toml").to_string_lossy().as_ref(),
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
        let app_path = if base_address != 0 {
            objcopy_to_bin(&elf)
        } else {
            elf
        };
        bins.push(app_path);
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let app_asm = out_dir.join("app.asm");
    write_app_asm(&app_asm, base, step, &bins);
    println!("cargo:rustc-env=APP_ASM={}", app_asm.display());
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

fn write_app_asm(path: &PathBuf, base: u64, step: u64, bins: &[PathBuf]) {
    use std::io::Write;
    let mut asm = fs::File::create(path)
        .unwrap_or_else(|err| panic!("failed to create {}: {}", path.display(), err));

    let version_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    writeln!(
        asm,
        "\
# Version Id: {version_id}
.global apps
.section .data
.align 3
apps:
    .quad {base:#x}
    .quad {step:#x}
    .quad {}",
        bins.len(),
    )
    .unwrap();

    for i in 0..bins.len() {
        writeln!(asm, "    .quad app_{i}_start").unwrap();
    }

    writeln!(asm, "    .quad app_{}_end", bins.len() - 1).unwrap();

    for (i, path) in bins.iter().enumerate() {
        writeln!(
            asm,
            "\
app_{i}_start:
    .incbin {path:?}
app_{i}_end:",
        )
        .unwrap();
    }
}

fn write_dummy_app_asm() {
    use std::io::Write;

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let app_asm = out_dir.join("app.asm");
    let mut asm = fs::File::create(&app_asm)
        .unwrap_or_else(|err| panic!("failed to create {}: {}", app_asm.display(), err));

    writeln!(
        asm,
        "\
.global apps
.section .data
.align 3
apps:
    .quad 0
    .quad 0
    .quad 0
    .quad 0"
    )
    .unwrap();

    println!("cargo:rustc-env=APP_ASM={}", app_asm.display());
}

fn ensure_tg_games() -> PathBuf {
    if let Ok(dir) = env::var("TG_GAMES_DIR") {
        let path = PathBuf::from(dir);
        if path.join("Cargo.toml").exists() {
            return path;
        }
    }

    let crate_name = env::var("TG_GAMES_CRATE")
        .expect("TG_GAMES_CRATE not set; add it to .cargo/config.toml [env]");
    let local_dir_name = env::var("TG_GAMES_LOCAL_DIR")
        .expect("TG_GAMES_LOCAL_DIR not set; add it to .cargo/config.toml [env]");
    let version = env::var("TG_GAMES_VERSION")
        .expect("TG_GAMES_VERSION not set; add it to .cargo/config.toml [env]");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let target_dir = manifest_dir.join(&local_dir_name);

    if target_dir.join("Cargo.toml").exists() {
        ensure_workspace_table(&target_dir);
        return target_dir;
    }

    let sibling_games_dir = manifest_dir.parent().unwrap().join(&crate_name);

    if sibling_games_dir.join("Cargo.toml").exists() {
        println!(
            "cargo:warning=Using local copy of crate {} in path {}",
            crate_name, local_dir_name
        );
        let status = Command::new("cp")
            .args([
                "-r",
                sibling_games_dir.to_string_lossy().as_ref(),
                target_dir.to_string_lossy().as_ref(),
            ])
            .status()
            .unwrap_or_else(|e| panic!("failed to execute cp: {e}"));

        if !status.success() {
            panic!(
                "failed to copy {} to {}",
                sibling_games_dir.display(),
                target_dir.display()
            );
        }

        let copied_target = target_dir.join("target");
        if copied_target.exists() {
            let _ = fs::remove_dir_all(&copied_target);
        }

        // Patch the generic framework dependencies path in the copied Cargo.toml
        let cargo_toml = target_dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = fs::read_to_string(&cargo_toml).unwrap();
            let patched = content.replace("path = \"../tg-", "path = \"../../tg-");
            fs::write(&cargo_toml, patched).unwrap();
        }
    } else {
        println!(
            "cargo:warning=Pulling crate {} with cargo clone",
            crate_name
        );
        let crate_spec = format!("{crate_name}@{version}");
        let status = Command::new("cargo")
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
    }

    if !target_dir.join("Cargo.toml").exists() {
        panic!(
            "failed to populate valid crate at {}",
            target_dir.display()
        );
    }

    ensure_workspace_table(&target_dir);
    target_dir
}

fn ensure_workspace_table(dir: &PathBuf) {
    let cargo_toml = dir.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).unwrap_or_default();
    if !content.contains("[workspace]") {
        fs::write(&cargo_toml, format!("{}\n[workspace]\n", content)).unwrap_or_else(|err| {
            panic!(
                "failed to patch Cargo.toml in {}: {}",
                dir.display(),
                err
            )
        });
    }
}
