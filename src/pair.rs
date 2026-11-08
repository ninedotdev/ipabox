use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::db::Db;
use crate::proc::{self, Line};
use crate::paths;

const PAIR_HELPER: &str = include_str!("../assets/pair_helper.py");

pub struct PairRequest {
    pub selector: String,
    pub name: String,
    pub device_class: String,
    pub ip: String,
    pub port: u16,
}

pub async fn run(
    db: Arc<Db>,
    req: PairRequest,
    tx: mpsc::UnboundedSender<Line>,
    pin_rx: mpsc::UnboundedReceiver<String>,
) -> Result<()> {
    let py = match paths::pipx_python() {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send(Line {
                text: format!("ERROR: {e}"),
                is_err: true,
            });
            return Err(e);
        }
    };
    let _ = tx.send(Line {
        text: format!("using interpreter: {}", py.display()),
        is_err: false,
    });

    let script = paths::scratch_dir().join("pair_helper.py");
    std::fs::write(&script, PAIR_HELPER)?;

    let mut cmd = proc::command(&py);
    cmd.arg("-u").arg(&script).arg(&req.selector);
    if !req.ip.is_empty() && req.port != 0 {
        cmd.arg(&req.ip).arg(req.port.to_string());
    }

    let (inner_tx, mut inner_rx) = mpsc::unbounded_channel::<Line>();
    let persist_tx = tx.clone();
    let db2 = db.clone();
    let name = req.name.clone();
    let class = req.device_class.clone();
    let relay = tokio::spawn(async move {
        while let Some(line) = inner_rx.recv().await {
            if line.text.to_lowercase().starts_with("success:") {
                let _ = db2.save_paired(&name, &class, "");
            }
            let _ = persist_tx.send(line);
        }
    });

    let status = proc::run_streaming(cmd, inner_tx, Some(pin_rx)).await;
    let _ = relay.await;
    match status {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = tx.send(Line {
                text: format!("ERROR: {e}"),
                is_err: true,
            });
            Err(e)
        }
    }
}

pub fn infer_class(name: &str, class: &str) -> String {
    if !class.is_empty() {
        return class.to_string();
    }
    crate::models::Device::class_from_name(name, crate::models::CLASS_APPLE_TV)
}
