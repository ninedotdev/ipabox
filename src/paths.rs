use std::path::PathBuf;

pub const APP_NAME: &str = "ipabox";
const LEGACY_APP_NAME: &str = "iNoaload";

pub fn data_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join(APP_NAME);
        if !dir.exists() {
            let legacy = base.join(LEGACY_APP_NAME);
            if legacy.is_dir() {
                match std::fs::rename(&legacy, &dir) {
                    Ok(()) => tracing::info!("migrated data dir from {LEGACY_APP_NAME}"),
                    Err(e) => {
                        tracing::warn!(error = %e, "could not migrate the old data dir; using it in place");
                        return legacy;
                    }
                }
            }
        }
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
    .clone()
}

pub fn db_path() -> PathBuf {
    data_dir().join("app.db")
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}


pub fn tmp_dir() -> PathBuf {
    let dir = data_dir().join("tmp");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn ipa_dir(app_id: i64) -> PathBuf {
    let dir = data_dir().join("ipa").join(app_id.to_string());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn task_log_dir() -> PathBuf {
    let dir = data_dir().join("log");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(APP_NAME);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn augmented_path() -> String {
    let home = home_dir();
    let extra = [
        home.join(".local/bin").display().to_string(),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
    ];
    let current = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", extra.join(":"), current)
}


pub fn pipx_python() -> anyhow::Result<PathBuf> {
    let home = home_dir();
    let candidates = [
        home.join(".local/pipx/venvs/pymobiledevice3/bin/python"),
        home.join(".local/share/pipx/venvs/pymobiledevice3/bin/python"),
        // pipx >= 1.3 defaults to the platform data dir on macOS.
        home.join("Library/Application Support/pipx/venvs/pymobiledevice3/bin/python"),
    ];
    for c in candidates {
        if c.exists() {
            return Ok(c);
        }
    }
    if let Ok(output) = std::process::Command::new("/usr/bin/which")
        .arg("pymobiledevice3")
        .env("PATH", augmented_path())
        .output()
        && output.status.success()
    {
        let shim = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !shim.is_empty() {
            // The pipx shim is a symlink into the venv's bin dir; resolve it so
            // the sibling `python` is the venv's, not one next to the shim.
            let shim = std::fs::canonicalize(&shim).unwrap_or_else(|_| PathBuf::from(&shim));
            let py = shim.parent().unwrap_or(std::path::Path::new("/")).join("python");
            if py.exists() {
                return Ok(py);
            }
        }
    }
    anyhow::bail!("pymobiledevice3 venv not found. Install with: pipx install pymobiledevice3")
}

pub fn pymobiledevice3_cli() -> anyhow::Result<PathBuf> {
    let py = pipx_python()?;
    Ok(py.with_file_name("pymobiledevice3"))
}


pub fn plumesign() -> anyhow::Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("plumesign");
        if sibling.exists() {
            return Ok(sibling);
        }
        let resources = dir.parent().map(|p| p.join("Resources/plumesign"));
        if let Some(r) = resources
            && r.exists()
        {
            return Ok(r);
        }
    }
    let scratch = scratch_dir().join("plumesign");
    if scratch.exists() {
        return Ok(scratch);
    }
    if let Ok(output) = std::process::Command::new("/usr/bin/which")
        .arg("plumesign")
        .env("PATH", augmented_path())
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    anyhow::bail!(
        "plumesign binary not found — place it next to the app binary, in {}, or on PATH",
        scratch_dir().display()
    )
}
