use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub apple_id: AppleId,
    pub task: TaskSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppleId {
    pub email: String,
    pub password: String,
    pub team_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskSettings {
    pub enabled: bool,
    pub advance_days: i64,
}

impl Default for TaskSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            advance_days: 1,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        match std::fs::read(paths::settings_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let path = paths::settings_path();
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}
