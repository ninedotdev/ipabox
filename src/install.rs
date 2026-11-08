use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use tokio::sync::mpsc;

use crate::models::InstallOptions;
use crate::paths;
use crate::proc::{self, Line};
use crate::tunneld;

fn emit(tx: &mpsc::UnboundedSender<Line>, text: impl Into<String>) {
    let _ = tx.send(Line {
        text: text.into(),
        is_err: false,
    });
}

type CurrentSink = Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>;

pub async fn run(
    opts: InstallOptions,
    tx: mpsc::UnboundedSender<Line>,
    mut code_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let current: CurrentSink = Arc::new(Mutex::new(None));
    let router_sink = current.clone();
    let router = tokio::spawn(async move {
        while let Some(code) = code_rx.recv().await {
            let sink = router_sink.lock().unwrap().clone();
            if let Some(sink) = sink {
                let _ = sink.send(code);
            }
        }
    });
    let _router_guard = AbortOnDrop(router);
    if !tunneld::is_running().await {
        emit(&tx, "ERROR: tunneld daemon is not running. Start it from the Settings tab.");
        bail!("tunneld not running");
    }
    if !opts.team_id.is_empty() {
        emit(&tx, format!("→ Using Apple Developer team: {}", opts.team_id));
    }

    emit(&tx, format!("→ Waiting for tunneld to reach {}…", opts.display_name()));
    let resolution = tunneld::resolve(&opts.udid, &opts.device_name).await?;
    emit(
        &tx,
        format!(
            "→ Resolved: hw={} tunnel={}",
            resolution.hardware_udid, resolution.tunnel_key
        ),
    );
    let hardware_udid = resolution.hardware_udid;

    let plumesign = paths::plumesign()?;

    if !account_registered(&plumesign, &opts.account).await {
        emit(&tx, "→ Phase 0/2: logging in to Apple ID (2FA may be requested)");
        let mut cmd = proc::command(&plumesign);
        cmd.args(["account", "login", "-u", &opts.account, "-p", &opts.password]);
        apply_team_env(&mut cmd, &opts.team_id);
        let (login_tx, login_rx) = mpsc::unbounded_channel::<String>();
        *current.lock().unwrap() = Some(login_tx);
        let status = proc::run_streaming(cmd, tx.clone(), Some(login_rx)).await;
        *current.lock().unwrap() = None;
        if status.map(|s| !s.success()).unwrap_or(true) {
            bail!("apple id login failed");
        }
    }

    let signed_path = paths::scratch_dir().join("signed.ipa");
    let _ = std::fs::remove_file(&signed_path);


    let platform = opts.apple_platform();
    let reg_name = opts.display_name();
    emit(
        &tx,
        format!("→ Registering {reg_name} with the developer team ({platform})"),
    );
    let mut reg = proc::command(&plumesign);
    reg.args([
        "account",
        "register-device",
        "-u",
        &opts.account,
        "--udid",
        &hardware_udid,
        "--name",
        &reg_name,
        "--platform",
        platform,
    ]);
    if !opts.team_id.is_empty() {
        reg.args(["-t", &opts.team_id]);
    }
    apply_team_env(&mut reg, &opts.team_id);
    let _ = proc::run_streaming(reg, tx.clone(), None).await; 


    emit(&tx, "→ Phase 1/2: signing IPA");
    let mut sign = proc::command(&plumesign);
    sign.args([
        "sign",
        "--apple-id",
        "-u",
        &opts.account,
        "-p",
        &opts.ipa_path,
        "-o",
        &signed_path.display().to_string(),
    ]);
    if opts.remove_extensions {
        sign.arg("--remove-extensions");
    }
    apply_team_env(&mut sign, &opts.team_id);
    let (sign_tx, sign_rx) = mpsc::unbounded_channel::<String>();
    *current.lock().unwrap() = Some(sign_tx);
    let status = proc::run_streaming(sign, tx.clone(), Some(sign_rx)).await;
    *current.lock().unwrap() = None;
    if status.map(|s| !s.success()).unwrap_or(true) {
        bail!("signing failed");
    }
    if !signed_path.exists() {
        emit(&tx, format!("ERROR: signed IPA not produced at {}", signed_path.display()));
        bail!("signed IPA missing");
    }

    let cli = paths::pymobiledevice3_cli()?;
    emit(&tx, "→ Phase 2/2: pushing signed IPA via RemoteXPC tunnel");
    let mut push = proc::command(&cli);
    push.args([
        "apps",
        "install",
        "--tunnel",
        &hardware_udid,
        &signed_path.display().to_string(),
    ]);

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<Line>();
    let forward_tx = tx.clone();
    let watcher = tokio::spawn(async move {
        let mut device_missing = false;
        while let Some(line) = line_rx.recv().await {
            if line.is_err && line.text.to_lowercase().contains("device not found") {
                device_missing = true;
            }
            let _ = forward_tx.send(line);
        }
        device_missing
    });
    let status = proc::run_streaming(push, line_tx, None).await;
    let device_missing = watcher.await.unwrap_or(false);

    if device_missing {
        emit(&tx, "Installation Failed!");
        bail!("apps install could not reach the device (\"Device not found\")");
    }
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            emit(&tx, "Installation Failed!");
            bail!("apps install exited with {status}");
        }
        Err(e) => {
            emit(&tx, "Installation Failed!");
            bail!("apps install could not be run: {e}");
        }
    }

    emit(&tx, "Installation Succeeded!");
    Ok(())
}

async fn account_registered(plumesign: &std::path::Path, email: &str) -> bool {
    let mut cmd = proc::command(plumesign);
    cmd.args(["account", "list"]);
    match proc::combined_output(cmd).await {
        Ok((_, out)) => out.to_lowercase().contains(&email.to_lowercase()),
        Err(_) => false,
    }
}

fn apply_team_env(cmd: &mut tokio::process::Command, team_id: &str) {
    if !team_id.is_empty() {
        cmd.env("PLUMESIGN_TEAM_ID", team_id);
    }
}


struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
