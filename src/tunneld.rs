use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use regex::Regex;
use serde::Deserialize;
use tokio::net::TcpStream;

use crate::paths;
use crate::proc;

pub const TUNNELD_HOST: &str = "127.0.0.1";
pub const TUNNELD_PORT: u16 = 49151;

#[derive(Debug, Clone, Deserialize)]
struct TunneldEntry {
    #[serde(rename = "tunnel-address")]
    address: String,
    #[serde(rename = "tunnel-port")]
    port: u16,
}

pub struct TunnelResolution {
    pub hardware_udid: String,
    pub tunnel_key: String,
}

pub async fn is_running() -> bool {
    tokio::time::timeout(
        Duration::from_millis(500),
        TcpStream::connect((TUNNELD_HOST, TUNNELD_PORT)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

pub async fn start_with_admin() -> Result<()> {
    if is_running().await {
        return Ok(());
    }
    let cli = paths::pymobiledevice3_cli()?;
    let home = paths::home_dir();
    // Create the data dir as the user before the root daemon can: otherwise
    // it ends up root-owned and pairing (run as the user) can't save records.
    let _ = std::fs::create_dir_all(home.join(".pymobiledevice3"));

    let shell_cmd = format!(
        "HOME={} {} remote tunneld --daemonize --host {} --port {}",
        shell_single_quote(&home.display().to_string()),
        shell_single_quote(&cli.display().to_string()),
        TUNNELD_HOST,
        TUNNELD_PORT
    );
    let osa = format!(
        "do shell script {} with administrator privileges with prompt {}",
        applescript_string(&shell_cmd),
        applescript_string("ipabox needs admin rights to start the install tunnel daemon.")
    );

    let output = proc::command("osascript").arg("-e").arg(&osa).output().await?;
    if !output.status.success() {
        bail!(
            "osascript failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if is_running().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    bail!("tunneld did not start within 5s")
}

async fn fetch_index() -> Result<HashMap<String, Vec<TunneldEntry>>> {
    let url = format!("http://{TUNNELD_HOST}:{TUNNELD_PORT}/");
    let body = http_get(&url).await?;
    Ok(serde_json::from_str(&body)?)
}

pub async fn resolve(mdns_id: &str, name: &str) -> Result<TunnelResolution> {
    if !is_running().await {
        bail!("tunneld is not running");
    }
    let cli = paths::pymobiledevice3_cli()?;
    let deadline = Instant::now() + Duration::from_secs(25);

    let (entry, key) = loop {
        if let Ok(tunnels) = fetch_index().await {
            if let Some(entries) = tunnels.get(mdns_id)
                && let Some(first) = entries.first()
            {
                break (clone_entry(first), mdns_id.to_string());
            }
            if !name.is_empty()
                && let Some((e, k)) = find_by_name(&cli, &tunnels, name).await
            {
                break (e, k);
            }
        }
        if Instant::now() > deadline {
            bail!("tunneld has no tunnel for {name:?} after 25s — is the device awake and on this network?");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };

    let udid = lockdown_field(&cli, &entry, "UniqueDeviceID").await?;
    Ok(TunnelResolution {
        hardware_udid: udid,
        tunnel_key: key,
    })
}

async fn find_by_name(
    cli: &std::path::Path,
    tunnels: &HashMap<String, Vec<TunneldEntry>>,
    target: &str,
) -> Option<(TunneldEntry, String)> {
    for (key, entries) in tunnels {
        for e in entries {
            if let Ok(name) = lockdown_field(cli, e, "DeviceName").await
                && name.eq_ignore_ascii_case(target)
            {
                return Some((clone_entry(e), key.clone()));
            }
        }
    }
    None
}

async fn lockdown_field(cli: &std::path::Path, entry: &TunneldEntry, field: &str) -> Result<String> {
    let mut cmd = proc::command(cli);
    cmd.args([
        "lockdown",
        "info",
        "--rsd",
        &entry.address,
        &entry.port.to_string(),
    ]);
    let (ok, out) = proc::combined_output(cmd).await?;
    if !ok {
        bail!("lockdown info failed: {}", out.trim());
    }
    let re = Regex::new(&format!(r#""{field}"\s*:\s*"([^"]+)""#)).unwrap();
    re.captures(&out)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| anyhow::anyhow!("{field} not found in lockdown output"))
}

fn clone_entry(e: &TunneldEntry) -> TunneldEntry {
    TunneldEntry {
        address: e.address.clone(),
        port: e.port,
    }
}

async fn http_get(url: &str) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = url.trim_start_matches("http://");
    let (host_port, path) = match path.find('/') {
        Some(i) => (&path[..i], &path[i..]),
        None => (path, "/"),
    };
    let mut stream = tokio::time::timeout(
        Duration::from_secs(3),
        TcpStream::connect(host_port),
    )
    .await??;
    let req = format!("GET {path} HTTP/1.0\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or(&text);
    Ok(body.to_string())
}

fn applescript_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
