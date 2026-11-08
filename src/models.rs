use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const CLASS_APPLE_TV: &str = "AppleTV";
pub const CLASS_IPHONE: &str = "iPhone";
pub const CLASS_IPAD: &str = "iPad";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    Unpaired,
    Paired,
    Pairable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub service_name: String,
    pub ip: String,
    pub mac_addr: String,
    pub udid: String,
    pub status: DeviceStatus,
    pub device_class: String,
    pub product_type: String,
    pub product_version: String,
    pub port: u16,
}

impl Device {
    pub fn class_lower(&self) -> String {
        self.device_class.to_lowercase()
    }

    pub fn is_apple_tv(&self) -> bool {
        self.class_lower() == "appletv"
    }

    pub fn class_from_name(name: &str, default: &str) -> String {
        let lower = name.to_lowercase();
        if lower.contains("appletv") || lower.contains("apple-tv") {
            CLASS_APPLE_TV.into()
        } else if lower.contains("ipad") {
            CLASS_IPAD.into()
        } else if lower.contains("iphone") {
            CLASS_IPHONE.into()
        } else {
            default.into()
        }
    }

    pub fn is_remote_udid(udid: &str) -> bool {
        let parts: Vec<&str> = udid.split('-').collect();
        parts.len() == 5
            && [8, 4, 4, 4, 12]
                .iter()
                .zip(&parts)
                .all(|(len, p)| p.len() == *len && p.chars().all(|c| c.is_ascii_hexdigit()))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledApp {
    pub id: i64,
    pub ipa_name: String,
    pub ipa_path: String,
    pub device: String,
    pub device_class: String,
    pub udid: String,
    pub account: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub installed_date: Option<DateTime<Utc>>,
    pub refreshed_date: Option<DateTime<Utc>>,
    pub expiration_date: Option<DateTime<Utc>>,
    pub refreshed_result: bool,
    pub refreshed_error: i64,
    pub icon: String,
    pub bundle_identifier: String,
    pub version: String,
    pub remove_extensions: bool,
    pub team_id: String,
    pub enabled: bool,
    pub created_at: Option<DateTime<Utc>>,
}

pub const REFRESH_ERROR_NONE: i64 = 0;
pub const REFRESH_ERROR_INVALID_ACCOUNT: i64 = 1;
pub const REFRESH_ERROR_OTHER: i64 = 99;

impl InstalledApp {
    pub fn need_refresh(&self, advance_days: i64) -> bool {
        let now = Utc::now();
        let Some(refreshed) = self.refreshed_date else {
            return true;
        };
        let expiration = self
            .expiration_date
            .unwrap_or(refreshed + Duration::days(7));
        let advance = if advance_days <= 0 { 1 } else { advance_days };
        expiration - Duration::days(advance) < now
    }

    pub fn is_expired(&self) -> bool {
        match self.expiration_date {
            Some(d) => d < Utc::now(),
            None => true,
        }
    }

    pub fn is_account_invalid(&self) -> bool {
        self.refreshed_error == REFRESH_ERROR_INVALID_ACCOUNT
    }
    pub fn dedupe_key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.device, self.device_class, self.bundle_identifier, self.account
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: i64,
    pub name: String,
    pub device_class: String,
    pub udid: String,
    pub ip: String,
    pub pair_type: String,
    pub last_paired_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct IpaInfo {
    pub name: String,
    pub path: String,
    pub icon: String,
    pub bundle_identifier: String,
    pub version: String,
}

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub udid: String,
    pub account: String,
    pub password: String,
    pub ipa_path: String,
    pub remove_extensions: bool,
    pub team_id: String,
    pub device_name: String,
    pub device_class: String,
}

impl InstallOptions {
    pub fn apple_platform(&self) -> &'static str {
        if self.device_class.eq_ignore_ascii_case(CLASS_APPLE_TV) {
            "tvos"
        } else {
            "ios"
        }
    }

    pub fn display_name(&self) -> String {
        if self.device_name.is_empty() {
            if self.device_class.is_empty() {
                "the device".to_string()
            } else {
                self.device_class.clone()
            }
        } else {
            self.device_name.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_udid_shapes() {
        assert!(Device::is_remote_udid("12345678-ABCD-ef01-2345-0123456789ab"));
        assert!(!Device::is_remote_udid("00008110-000C143E3E0A201E"));
        assert!(!Device::is_remote_udid(""));
    }

    #[test]
    fn apple_platform_follows_the_device_class() {
        let tv = InstallOptions {
            device_class: CLASS_APPLE_TV.into(),
            ..Default::default()
        };
        assert_eq!(tv.apple_platform(), "tvos");
        for class in [CLASS_IPHONE, CLASS_IPAD, ""] {
            let ios = InstallOptions {
                device_class: class.into(),
                ..Default::default()
            };
            assert_eq!(ios.apple_platform(), "ios", "class {class:?}");
        }
    }

    #[test]
    fn display_name_prefers_the_device_name() {
        let named = InstallOptions {
            device_name: "iPhone Carli".into(),
            device_class: CLASS_IPHONE.into(),
            ..Default::default()
        };
        assert_eq!(named.display_name(), "iPhone Carli");
        let unnamed = InstallOptions {
            device_class: CLASS_IPHONE.into(),
            ..Default::default()
        };
        assert_eq!(unnamed.display_name(), "iPhone");
        assert_eq!(InstallOptions::default().display_name(), "the device");
    }

    #[test]
    fn need_refresh_defaults() {
        let mut app = InstalledApp::default();
        assert!(app.need_refresh(1));
        app.refreshed_date = Some(Utc::now());
        app.expiration_date = Some(Utc::now() + Duration::days(7));
        assert!(!app.need_refresh(1));
        app.expiration_date = Some(Utc::now() + Duration::hours(12));
        assert!(app.need_refresh(1));
    }
}
