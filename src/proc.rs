use std::process::Stdio;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::paths;


#[derive(Debug, Clone)]
pub struct Line {
    pub text: String,
    pub is_err: bool,
}

pub fn command(bin: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(bin);
    cmd.env("PATH", paths::augmented_path());
    cmd
}


pub async fn run_streaming(
    mut cmd: Command,
    tx: mpsc::UnboundedSender<Line>,
    mut stdin_rx: Option<mpsc::UnboundedReceiver<String>>,
) -> Result<std::process::ExitStatus> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child: Child = cmd.spawn()?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let mut stdin = child.stdin.take();

    let tx_out = tx.clone();
    let out_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx_out.send(Line {
                text: line,
                is_err: false,
            });
        }
    });
    let tx_err = tx.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx_err.send(Line {
                text: line,
                is_err: true,
            });
        }
    });

    let stdin_task = tokio::spawn(async move {
        if let (Some(rx), Some(stdin)) = (stdin_rx.as_mut(), stdin.as_mut()) {
            while let Some(mut value) = rx.recv().await {
                if !value.ends_with('\n') {
                    value.push('\n');
                }
                if stdin.write_all(value.as_bytes()).await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        }
    });

    let status = child.wait().await?;
    let _ = out_task.await;
    let _ = err_task.await;
    stdin_task.abort();
    Ok(status)
}


pub async fn combined_output(mut cmd: Command) -> Result<(bool, String)> {
    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), text))
}
