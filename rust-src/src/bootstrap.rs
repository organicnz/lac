//! bootstrap v2.6 — idempotent machine setup (Rust).
//!
//! v2.6: never creates directories under /Volumes unless the volume is
//! really mounted (the old code shadowed the future mount with an
//! internal-disk directory of the same name); never overwrites
//! user-edited loop files; actually installs the zprofile hooks the
//! docs always claimed existed; verifies Time Machine exclusions.

mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

fn get_project_root() -> String {
    common::project_root()
}

fn tm_excluded(path: &str) -> bool {
    Command::new("tmutil")
        .args(["isexcluded", path])
        .output()
        .ok()
        .map(|o| {
            let t = String::from_utf8_lossy(&o.stdout);
            t.contains("[Excluded]")
        })
        .unwrap_or(false)
}

const ZPROFILE_MARKER: &str = "# >>> LAC env (managed by lac bootstrap) >>>";

fn ensure_zprofile(model_base: &str) {
    let home = common::home_dir();
    let zp = format!("{}/.zprofile", home);
    let current = fs::read_to_string(&zp).unwrap_or_default();
    if current.contains(ZPROFILE_MARKER) {
        eprintln!("   zprofile hooks already present");
        return;
    }
    let block = format!(
        "\n{1}\n\
         export HF_HOME=\"{0}/hf\"\n\
         export HF_HUB_CACHE=\"{0}/hf/hub\"\n\
         export PATH=\"$HOME/.local/bin:$PATH\"\n\
         # <<< LAC env <<<\n",
        model_base,
        ZPROFILE_MARKER
    );
    let mut new = current;
    if !new.ends_with('\n') && !new.is_empty() {
        new.push('\n');
    }
    new.push_str(&block);
    match common::atomic_write(&zp, &new) {
        Ok(()) => eprintln!("   zprofile hooks installed in {}", zp),
        Err(e) => eprintln!("   zprofile install failed: {}", e),
    }
}

fn main() {
    common::ignore_sigpipe();
    eprintln!("=== LAC bootstrap v2.6 (Rust, idempotent) ===");
    let home = common::home_dir();
    let root = get_project_root();
    let mut warns = 0;

    // 1. Homebrew
    eprintln!("1. Ensuring Homebrew...");
    let brew_path = common::which("brew").unwrap_or_default();
    if brew_path.is_empty() {
        eprintln!("   brew NOT FOUND — install from https://brew.sh then rerun");
        warns += 1;
    } else {
        eprintln!("   brew: {}", brew_path);
    }

    // 2. Brew bundle (failures are real errors, not silence).
    let brewfile_path = format!("{}/Brewfile", root);
    if Path::new(&brewfile_path).exists() && !brew_path.is_empty() {
        eprintln!("2. Running brew bundle ({})...", brewfile_path);
        match Command::new("brew")
            .args(["bundle", "install", &format!("--file={}", brewfile_path)])
            .output()
        {
            Ok(o) if o.status.success() => eprintln!("   brew bundle: ok"),
            Ok(o) => {
                eprintln!("   brew bundle exit: {} (see output above)", o.status);
                warns += 1;
            }
            Err(e) => {
                eprintln!("   brew bundle failed to run: {}", e);
                warns += 1;
            }
        }
    } else {
        eprintln!("2. Brewfile step skipped (no Brewfile or no brew).");
    }

    // 3. Rust via rustup
    eprintln!("3. Ensuring Rust via rustup...");
    if common::which("rustc").is_some() {
        if let Ok(o) = Command::new("rustc").arg("-V").output() {
            eprintln!("   rustc: {}", String::from_utf8_lossy(&o.stdout).trim());
        }
    } else {
        eprintln!("   rustc missing — run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y");
        warns += 1;
    }

    // 4. uv / python
    eprintln!("4. Ensuring uv/python...");
    match Command::new("uv").arg("--version").output() {
        Ok(o) if o.status.success() => {
            eprintln!("   uv: {}", String::from_utf8_lossy(&o.stdout).trim())
        }
        _ => {
            eprintln!("   uv missing — run: curl -LsSf https://astral.sh/uv/install.sh | sh");
            warns += 1;
        }
    }

    // 5. mise
    eprintln!("5. Ensuring mise...");
    match Command::new("mise").arg("--version").output() {
        Ok(o) if o.status.success() => {
            eprintln!("   mise: {}", String::from_utf8_lossy(&o.stdout).trim())
        }
        _ => {
            eprintln!("   mise missing — run: curl https://mise.run | sh");
            warns += 1;
        }
    }

    // 6. Model library dirs — mount-gated, never shadow /Volumes.
    eprintln!("6. Creating model library dirs...");
    let external = common::volume_mounted("/Volumes/AIModels");
    let model_base = if external {
        "/Volumes/AIModels".to_string()
    } else {
        eprintln!("   /Volumes/AIModels not mounted — using internal fallback");
        common::model_base()
    };
    for d in [
        model_base.clone(),
        format!("{}/hf", model_base),
        format!("{}/ollama", model_base),
        format!("{}/lmstudio", model_base),
        format!("{}/indexes", model_base),
    ] {
        match fs::create_dir_all(&d) {
            Ok(()) => eprintln!("   verified: {}", d),
            Err(e) => {
                eprintln!("   FAILED {}: {}", d, e);
                warns += 1;
            }
        }
    }

    // 7. Ollama symlink (migrate-aware, never destroy data).
    eprintln!("7. Setting up ollama symlink...");
    let ollama_parent = format!("{}/.ollama", home);
    let _ = fs::create_dir_all(&ollama_parent);
    let target = format!("{}/ollama", model_base);
    let link = format!("{}/models", ollama_parent);
    match fs::symlink_metadata(&link) {
        Ok(m) if m.file_type().is_symlink() => {
            let ok = fs::read_link(&link).ok().map(|t| t.exists()).unwrap_or(false);
            if ok {
                eprintln!("   symlink healthy: {} -> {}", link, target);
            } else {
                let _ = fs::remove_file(&link);
                match symlink(&target, &link) {
                    Ok(()) => eprintln!("   relinked dangling symlink -> {}", target),
                    Err(e) => {
                        eprintln!("   relink failed: {}", e);
                        warns += 1;
                    }
                }
            }
        }
        Ok(_) => {
            eprintln!("   {} is a real directory with user data — NOT replacing.", link);
            eprintln!("   To migrate: move its contents to {} then rerun.", target);
            warns += 1;
        }
        Err(_) => match symlink(&target, &link) {
            Ok(()) => eprintln!("   symlink: {} -> {}", link, target),
            Err(e) => {
                eprintln!("   symlink failed: {}", e);
                warns += 1;
            }
        },
    }

    // 8. Kanban templates — never overwrite user edits.
    eprintln!("8. Initializing Kanban task & loop templates...");
    let loops_dir = format!("{}/todo/lac-loops", home);
    let _ = fs::create_dir_all(&loops_dir);
    let templates_loops = format!("{}/templates/loops", root);
    if let Ok(entries) = fs::read_dir(&templates_loops) {
        for entry in entries.flatten() {
            let dest = format!("{}/{}", loops_dir, entry.file_name().to_string_lossy());
            if fs::metadata(&dest).is_ok() {
                eprintln!("   keeping existing: {}", dest);
            } else {
                let _ = fs::copy(entry.path(), &dest);
                eprintln!("   installed loop: {}", dest);
            }
        }
    }
    let task_template = format!("{}/templates/tasks/lac-tasks.yaml", root);
    let task_dest = format!("{}/todo/lac-tasks.yaml", home);
    if fs::metadata(&task_dest).is_err() && Path::new(&task_template).exists() {
        let _ = fs::copy(&task_template, &task_dest);
        eprintln!("   installed task queue: {}", task_dest);
    } else {
        eprintln!("   keeping existing task queue: {}", task_dest);
    }

    // 9. Shell env hooks (the docs always promised these).
    eprintln!("9. Ensuring shell env hooks...");
    ensure_zprofile(&model_base);

    // 10. Time Machine exclusion, verified.
    eprintln!("10. Applying Time Machine exclusions...");
    let _ = Command::new("tmutil").args(["addexclusion", &model_base]).output();
    if tm_excluded(&model_base) {
        eprintln!("   verified excluded: {}", model_base);
    } else {
        eprintln!("   WARNING: tmutil exclusion not confirmed for {}", model_base);
        warns += 1;
    }

    if warns == 0 {
        eprintln!("=== LAC bootstrap complete: all green ===");
    } else {
        eprintln!("=== LAC bootstrap complete with {} warning(s) (see above) ===", warns);
    }
}
