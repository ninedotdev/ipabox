use anyhow::Result;
use regex::Regex;

use crate::models::Team;
use crate::paths;
use crate::proc;
use crate::tunneld;


pub async fn sign_in(
    email: &str,
    password: &str,
    team_id: &str,
    tx: tokio::sync::mpsc::UnboundedSender<crate::proc::Line>,
    code_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let plumesign = paths::plumesign()?;
    let mut cmd = proc::command(&plumesign);
    cmd.args(["account", "login", "-u", email, "-p", password]);
    if !team_id.is_empty() {
        cmd.env("PLUMESIGN_TEAM_ID", team_id);
    }
    let status = proc::run_streaming(cmd, tx, Some(code_rx)).await?;
    if !status.success() {
        anyhow::bail!("plumesign account login exited with {status}");
    }
    Ok(())
}

pub async fn list_teams(email: &str) -> Result<Vec<Team>> {
    let plumesign = paths::plumesign()?;
    let mut cmd = proc::command(&plumesign);
    cmd.args(["account", "teams", "-u", email]);
    let (ok, out) = proc::combined_output(cmd).await?;
    if !ok {
        anyhow::bail!("plumesign account teams failed: {}", out.trim());
    }
    Ok(parse_teams(&out))
}

fn parse_teams(out: &str) -> Vec<Team> {
    let trimmed = out.trim();
    if trimmed.starts_with('[')
        && let Ok(teams) = serde_json::from_str::<Vec<Team>>(trimmed)
    {
        return teams;
    }
    let re = Regex::new(r"`([^`]+)`,\s+with ID\s+`([A-Z0-9]+)`").unwrap();
    re.captures_iter(trimmed)
        .map(|c| Team {
            name: c[1].trim().to_string(),
            id: c[2].to_string(),
        })
        .collect()
}

pub async fn uninstall(mdns_id: &str, device_name: &str, bundle_id: &str) -> Result<()> {
    if bundle_id.is_empty() {
        anyhow::bail!("bundle_identifier is empty");
    }
    let resolution = tunneld::resolve(mdns_id, device_name).await?;
    let cli = paths::pymobiledevice3_cli()?;
    let mut cmd = proc::command(&cli);
    cmd.args([
        "apps",
        "uninstall",
        "--tunnel",
        &resolution.hardware_udid,
        bundle_id,
    ]);
    let (ok, out) = proc::combined_output(cmd).await?;
    if !ok {
        anyhow::bail!("apps uninstall failed: {}", out.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_format() {
        let out = "Found teams:\n - `Carlos Personal`, with ID `ABC123XYZ`.\n";
        let teams = parse_teams(out);
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].id, "ABC123XYZ");
        assert_eq!(teams[0].name, "Carlos Personal");
    }
}
