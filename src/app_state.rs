use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use gpui::{Context, FocusHandle, ScrollHandle, Task};
use gpui_tokio::Tokio;
use tokio::sync::mpsc;

use crate::db::{self, Db};
use crate::discovery;
use crate::install;
use crate::models::{
    CLASS_APPLE_TV, Device, DeviceStatus, InstallOptions, InstalledApp, PairedDevice, Team,
};
use crate::pair::{self, PairRequest};
use crate::paths;
use crate::proc::Line;
use crate::settings::Settings;
use crate::teams;
use crate::tunneld;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Devices,
    Install,
    Apps,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairPhase {
    Idle,
    Running,
    AwaitingPin,
    Success,
    Error,
}

pub struct PairUi {
    pub device: Device,
    pub log: Vec<String>,
    pub phase: PairPhase,
    pub code: String,
    pub pin_tx: Option<mpsc::UnboundedSender<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPhase {
    Idle,
    Uploading,
    Installing,
    Awaiting2fa,
    Success,
    Error,
}

pub struct InstallUi {
    pub selected_id: Option<String>,
    pub log: Vec<String>,
    pub phase: InstallPhase,
    pub otp: String,
    pub code_tx: Option<mpsc::UnboundedSender<String>>,
    /// Metadata of the IPA currently being installed, persisted on success.
    pub pending: Option<InstalledApp>,
}

impl Default for InstallUi {
    fn default() -> Self {
        Self {
            selected_id: None,
            log: Vec::new(),
            phase: InstallPhase::Idle,
            otp: String::new(),
            code_tx: None,
            pending: None,
        }
    }
}

/// Apple ID sign-in run from Settings. plumesign stores the developer session
/// locally and every other command (`account teams`, `sign`) needs it, so the
/// user must be able to log in without first starting an install.
#[derive(Default)]
pub struct LoginUi {
    pub log: Vec<String>,
    pub phase: LoginPhase,
    pub otp: String,
    pub code_tx: Option<mpsc::UnboundedSender<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginPhase {
    #[default]
    Idle,
    Running,
    Awaiting2fa,
    Success,
    Error,
}

pub struct AppState {
    pub db: Arc<Db>,
    pub settings: Settings,
    pub login: LoginUi,
    pub tab: Tab,
    pub devices: Vec<Device>,
    pub apps: Vec<InstalledApp>,
    pub teams: Vec<Team>,
    pub loading_teams: bool,
    pub tunneld_running: bool,
    pub starting_tunneld: bool,
    pub deep_scanning: bool,
    pub menu_open: bool,
    pub pair: Option<PairUi>,
    pub install: InstallUi,
    pub status: Option<String>,
    pub focus: Focuses,
    pub scrolls: Scrolls,
    _tasks: Vec<Task<()>>,
}

/// One field the keyboard can edit. Routed through `edit_field`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Email,
    Password,
    Otp,
    Pin,
    LoginOtp,
}

/// Scroll handles for the streaming logs, so each can be pinned to its newest
/// line as output arrives.
#[derive(Default)]
pub struct Scrolls {
    pub install: ScrollHandle,
    pub pair: ScrollHandle,
    pub login: ScrollHandle,
}

pub struct Focuses {
    pub email: FocusHandle,
    pub password: FocusHandle,
    pub otp: FocusHandle,
    pub pin: FocusHandle,
    pub login_otp: FocusHandle,
}

impl AppState {
    pub fn new(db: Arc<Db>, cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let mut state = Self {
            db,
            settings,
            login: LoginUi::default(),
            tab: Tab::Devices,
            devices: Vec::new(),
            apps: Vec::new(),
            teams: Vec::new(),
            loading_teams: false,
            tunneld_running: false,
            starting_tunneld: false,
            deep_scanning: false,
            menu_open: false,
            pair: None,
            install: InstallUi::default(),
            status: None,
            focus: Focuses {
                email: cx.focus_handle(),
                password: cx.focus_handle(),
                otp: cx.focus_handle(),
                pin: cx.focus_handle(),
                login_otp: cx.focus_handle(),
            },
            scrolls: Scrolls::default(),
            _tasks: Vec::new(),
        };
        state.bootstrap(cx);
        state
    }

    /// Kick off the standing background loops: device scan, app poll, tunneld
    /// status poll. Called once after the entity is created.
    fn bootstrap(&mut self, cx: &mut Context<Self>) {
        self._tasks.push(self.spawn_scan_loop(cx));
        self._tasks.push(self.spawn_apps_loop(cx));
        self._tasks.push(self.spawn_tunneld_loop(cx));
        self._tasks.push(self.spawn_refresh_loop(cx));
    }

    /// Auto re-sign expiring apps before the 7-day cert expiry — port of
    /// `task.go`'s cron + `RefreshExpiringNow`. Runs once ~15s after launch,
    /// then hourly. Installs are sequential (one at a time) and only fire when
    /// tunneld is up and the account hasn't been flagged invalid. Uses each
    /// app's stored password, so no interactive 2FA (matches the Go task flow).
    fn spawn_refresh_loop(&self, cx: &mut Context<Self>) -> Task<()> {
        let db = self.db.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(15))
                .await;
            loop {
                let advance_days = this
                    .read_with(cx, |s, _| s.settings.task.advance_days)
                    .unwrap_or(1);
                let enabled_task = this
                    .read_with(cx, |s, _| s.settings.task.enabled)
                    .unwrap_or(true);
                let running = Tokio::spawn(cx, async { tunneld::is_running().await })
                    .await
                    .unwrap_or(false);

                if enabled_task && running {
                    let apps = db.enabled_apps().unwrap_or_default();
                    for app in apps {
                        if !app.need_refresh(advance_days) || app.is_account_invalid() {
                            continue;
                        }
                        // Do NOT gate on a RemoteXPC-shaped UDID here. An iPhone
                        // discovered over usbmuxd carries its hardware UDID
                        // (`00008140-0011706...`), which is not UUID-shaped, so
                        // that check silently excluded every iPhone from auto
                        // re-signing — they would just expire. tunneld resolves
                        // by identifier OR device name, so either is enough.
                        if app.udid.is_empty() && app.device.is_empty() {
                            continue;
                        }
                        let opts = InstallOptions {
                            udid: app.udid.clone(),
                            account: app.account.clone(),
                            password: app.password.clone(),
                            ipa_path: app.ipa_path.clone(),
                            remove_extensions: app.remove_extensions,
                            team_id: app.team_id.clone(),
                            device_name: app.device.clone(),
                            device_class: app.device_class.clone(),
                        };
                        let (line_tx, mut line_rx) = mpsc::unbounded_channel::<Line>();
                        let (_code_tx, code_rx) = mpsc::unbounded_channel::<String>();
                        let run =
                            Tokio::spawn(cx, async move { install::run(opts, line_tx, code_rx).await });
                        while line_rx.recv().await.is_some() {}
                        let ok = matches!(run.await, Ok(Ok(())));
                        let mut updated = app;
                        let now = Utc::now();
                        updated.refreshed_date = Some(now);
                        updated.expiration_date = Some(now + ChronoDuration::days(7));
                        updated.refreshed_result = ok;
                        updated.refreshed_error = if ok {
                            crate::models::REFRESH_ERROR_NONE
                        } else {
                            crate::models::REFRESH_ERROR_OTHER
                        };
                        let _ = db.update_refresh_result(&updated);
                        // Space installs out like the Go queue does.
                        cx.background_executor()
                            .timer(Duration::from_secs(10))
                            .await;
                    }
                }

                if this.update(cx, |s, cx| s.refresh_apps(cx)).is_err() {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_secs(3600))
                    .await;
            }
        })
    }

    fn field_value(&self, field: Field) -> String {
        match field {
            Field::Email => self.settings.apple_id.email.clone(),
            Field::Password => self.settings.apple_id.password.clone(),
            Field::Otp => self.install.otp.clone(),
            Field::Pin => self.pair.as_ref().map(|p| p.code.clone()).unwrap_or_default(),
            Field::LoginOtp => self.login.otp.clone(),
        }
    }

    fn set_field(&mut self, field: Field, value: String) {
        match field {
            Field::Email => self.settings.apple_id.email = value,
            Field::Password => self.settings.apple_id.password = value,
            Field::Otp => self.install.otp = value,
            Field::Pin => {
                if let Some(p) = self.pair.as_mut() {
                    p.code = value;
                }
            }
            Field::LoginOtp => self.login.otp = value,
        }
    }

    /// Append pasted text, honouring the digits-only/6-char rule for codes.
    /// Auto-submits when a pasted code completes the field.
    fn append_to_field(&mut self, field: Field, text: &str, cx: &mut Context<Self>) {
        let mut value = self.field_value(field);
        if matches!(field, Field::Otp | Field::Pin | Field::LoginOtp) {
            for ch in text.chars().filter(|c| c.is_ascii_digit()) {
                if value.len() >= 6 {
                    break;
                }
                value.push(ch);
            }
        } else {
            value.push_str(text);
        }
        let complete = value.len() == 6;
        self.set_field(field, value);
        match (field, complete) {
            (Field::Otp, true) => self.submit_2fa(cx),
            (Field::Pin, true) => self.submit_pin(cx),
            (Field::LoginOtp, true) => self.submit_login_2fa(cx),
            _ => cx.notify(),
        }
    }

    /// Append/erase a character on the focused text field, and handle the
    /// clipboard verbs. gpui ships no text field, so this is the whole input
    /// model: no cursor, no selection — edits always apply at the end.
    pub fn edit_field(&mut self, field: Field, ev: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let mods = ev.keystroke.modifiers;

        // Clipboard verbs first: ⌘V pastes, ⌘C copies, ⌘A+delete clears. These
        // must run before the character path, which ignores platform-modified
        // keystrokes.
        if mods.platform {
            match key {
                "v" => {
                    let pasted = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default();
                    if !pasted.is_empty() {
                        // Paste arrives as a whole string; strip newlines so a
                        // copied credential can't smuggle in a line break.
                        let clean: String =
                            pasted.chars().filter(|c| !c.is_control()).collect();
                        self.append_to_field(field, &clean, cx);
                    }
                    return;
                }
                "c" | "x" => {
                    let value = self.field_value(field);
                    if !value.is_empty() {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(value));
                    }
                    if key == "x" {
                        self.set_field(field, String::new());
                        cx.notify();
                    }
                    return;
                }
                "a" => {
                    // No selection model — treat ⌘A as "clear on next edit" by
                    // clearing immediately, which is what the user wants when
                    // they ⌘A to replace a value.
                    self.set_field(field, String::new());
                    cx.notify();
                    return;
                }
                _ => return,
            }
        }

        let target: &mut String = match field {
            Field::Email => &mut self.settings.apple_id.email,
            Field::Password => &mut self.settings.apple_id.password,
            Field::Otp => &mut self.install.otp,
            Field::Pin => match self.pair.as_mut() {
                Some(p) => &mut p.code,
                None => return,
            },
            Field::LoginOtp => &mut self.login.otp,
        };
        match key {
            "backspace" => {
                target.pop();
            }
            "enter" => match field {
                Field::Otp => {
                    self.submit_2fa(cx);
                    return;
                }
                Field::Pin => {
                    self.submit_pin(cx);
                    return;
                }
                Field::LoginOtp => {
                    self.submit_login_2fa(cx);
                    return;
                }
                Field::Email | Field::Password => {}
            },
            _ => {
                if let Some(ch) = &ev.keystroke.key_char
                    && !ch.is_empty()
                    && !ev.keystroke.modifiers.control
                    && !ev.keystroke.modifiers.platform
                {
                    // OTP/PIN accept digits only, max 6.
                    if matches!(field, Field::Otp | Field::Pin) {
                        if ch.chars().all(|c| c.is_ascii_digit()) && target.len() < 6 {
                            target.push_str(ch);
                        }
                    } else {
                        target.push_str(ch);
                    }
                }
            }
        }
        // Auto-submit when a 6-digit code is complete.
        let complete = match field {
            Field::Otp => self.install.otp.len() == 6,
            Field::Pin => self.pair.as_ref().map(|p| p.code.len() == 6).unwrap_or(false),
            Field::LoginOtp => self.login.otp.len() == 6,
            _ => false,
        };
        if complete {
            match field {
                Field::Otp => self.submit_2fa(cx),
                Field::Pin => self.submit_pin(cx),
                Field::LoginOtp => self.submit_login_2fa(cx),
                _ => {}
            }
        } else {
            cx.notify();
        }
    }

    // ---- background loops ----------------------------------------------

    fn spawn_scan_loop(&self, cx: &mut Context<Self>) -> Task<()> {
        let db = self.db.clone();
        cx.spawn(async move |this, cx| {
            loop {
                let scan_db = db.clone();
                // mdns-sd browse is blocking; run it on tokio's blocking pool.
                let scan = Tokio::spawn(cx, async move {
                    tokio::task::spawn_blocking(move || {
                        discovery::scan(&scan_db, Duration::from_secs(3)).unwrap_or_default()
                    })
                    .await
                    .unwrap_or_default()
                });
                if let Ok(mut devices) = scan.await {
                    merge_db_paired(&mut devices, &db);
                    discovery::sort_devices(&mut devices);
                    if this
                        .update(cx, |s, cx| {
                            if !devices.is_empty() || s.devices.is_empty() {
                                s.devices = devices;
                            }
                            s.deep_scanning = false;
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                cx.background_executor().timer(Duration::from_secs(12)).await;
            }
        })
    }

    fn spawn_apps_loop(&self, cx: &mut Context<Self>) -> Task<()> {
        let db = self.db.clone();
        cx.spawn(async move |this, cx| {
            loop {
                let apps = db.list_apps().unwrap_or_default();
                if this
                    .update(cx, |s, cx| {
                        s.apps = apps;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(10)).await;
            }
        })
    }

    fn spawn_tunneld_loop(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                let running = Tokio::spawn(cx, async { tunneld::is_running().await })
                    .await
                    .unwrap_or(false);
                if this
                    .update(cx, |s, cx| {
                        if s.tunneld_running != running {
                            s.tunneld_running = running;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(3)).await;
            }
        })
    }

    // ---- device helpers -------------------------------------------------

    /// Install targets: every paired device. The class is not used to filter —
    /// a device whose class could not be determined (no lockdown answer, no
    /// model keyword in the hostname) would otherwise be silently unusable.
    pub fn install_targets(&self) -> Vec<Device> {
        self.devices
            .iter()
            .filter(|d| d.status == DeviceStatus::Paired)
            .cloned()
            .collect()
    }

    pub fn deep_scan(&mut self, cx: &mut Context<Self>) {
        if self.deep_scanning {
            return;
        }
        self.deep_scanning = true;
        cx.notify();
        let db = self.db.clone();
        cx.spawn(async move |this, cx| {
            let scan_db = db.clone();
            let scan = Tokio::spawn(cx, async move {
                tokio::task::spawn_blocking(move || {
                    discovery::scan(&scan_db, Duration::from_secs(5)).unwrap_or_default()
                })
                .await
                .unwrap_or_default()
            });
            let mut devices = scan.await.unwrap_or_default();
            merge_db_paired(&mut devices, &db);
            discovery::sort_devices(&mut devices);
            let _ = this.update(cx, |s, cx| {
                let found = devices.len();
                if !devices.is_empty() {
                    s.devices = devices;
                }
                s.deep_scanning = false;
                s.status = Some(format!("Deep scan found {found} device(s)"));
                cx.notify();
            });
        })
        .detach();
    }

    pub fn unpair(&mut self, device: &Device, cx: &mut Context<Self>) {
        let class = if device.device_class.is_empty() {
            CLASS_APPLE_TV.to_string()
        } else {
            device.device_class.clone()
        };
        let _ = self.db.delete_paired(&device.name, &class);
        self.status = Some(format!("Unpaired {}", device.name));
        cx.notify();
    }

    // ---- pairing --------------------------------------------------------

    pub fn open_pair(&mut self, device: Device, cx: &mut Context<Self>) {
        self.pair = Some(PairUi {
            device,
            log: Vec::new(),
            phase: PairPhase::Idle,
            code: String::new(),
            pin_tx: None,
        });
        cx.notify();
    }

    pub fn close_pair(&mut self, cx: &mut Context<Self>) {
        self.pair = None;
        cx.notify();
    }

    pub fn start_pair(&mut self, cx: &mut Context<Self>) {
        let Some(pair_ui) = self.pair.as_mut() else {
            return;
        };
        let device = pair_ui.device.clone();
        // Route by shape: RemoteXPC devices identify by their UUID; carry name+
        // class so the backend persists a matchable pair record.
        let looks_remote = Device::is_remote_udid(&device.udid)
            || device.udid.len() >= 20 && device.udid.contains('-');
        let selector = if looks_remote && !device.udid.is_empty() {
            device.udid.clone()
        } else {
            device.name.clone()
        };
        let class = pair::infer_class(&device.name, &device.device_class);

        pair_ui.phase = PairPhase::Running;
        pair_ui
            .log
            .push(format!("→ Pairing {} via pymobiledevice3 (RemoteXPC)", device.name));
        if device.port == 0 {
            // Only the manual-pairing advert carries a port. Without it the
            // device is merely visible on the network, so say so rather than
            // letting the helper fail with a confusing browse error.
            pair_ui.log.push(
                "! This device is not advertising pairing mode — put it in pairing mode first."
                    .into(),
            );
        }
        pair_ui
            .log
            .push("→ Accept the trust prompt on the device when it appears.".into());

        let (line_tx, mut line_rx) = mpsc::unbounded_channel::<Line>();
        let (pin_tx, pin_rx) = mpsc::unbounded_channel::<String>();
        pair_ui.pin_tx = Some(pin_tx);

        let db = self.db.clone();
        let req = PairRequest {
            selector,
            name: device.name.clone(),
            device_class: class,
            ip: device.ip.clone(),
            port: device.port,
        };
        // Run the helper on tokio.
        Tokio::spawn(cx, async move { pair::run(db, req, line_tx, pin_rx).await })
            .detach();

        // Fold each line into the pair log on gpui's executor.
        cx.spawn(async move |this, cx| {
            while let Some(line) = line_rx.recv().await {
                let stop = this
                    .update(cx, |s, cx| {
                        if let Some(p) = s.pair.as_mut() {
                            let lower = line.text.to_lowercase();
                            p.log.push(line.text);
                            s.scrolls.pair.scroll_to_bottom();
                            if lower.starts_with("success:") {
                                p.phase = PairPhase::Success;
                                // Flip the row now instead of waiting up to a
                                // scan cycle (~15s) for the registry to apply.
                                let id = p.device.id.clone();
                                if let Some(d) = s.devices.iter_mut().find(|d| d.id == id) {
                                    d.status = DeviceStatus::Paired;
                                }
                            } else if lower.starts_with("error:") {
                                p.phase = PairPhase::Error;
                            } else if lower.starts_with("enter pin") {
                                p.phase = PairPhase::AwaitingPin;
                            }
                            cx.notify();
                            false
                        } else {
                            true
                        }
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn submit_pin(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.pair.as_mut() else { return };
        let pin = p.code.trim().to_string();
        if pin.is_empty() {
            return;
        }
        if let Some(tx) = &p.pin_tx {
            let _ = tx.send(pin.clone());
        }
        p.log.push(format!("→ Submitted PIN {pin}"));
        p.code.clear();
        p.phase = PairPhase::Running;
        cx.notify();
    }

    // ---- install --------------------------------------------------------

    pub fn selected_install_device(&self) -> Option<Device> {
        let targets = self.install_targets();
        let id = self.install.selected_id.clone();
        id.and_then(|id| targets.iter().find(|d| d.id == id).cloned())
            .or_else(|| targets.first().cloned())
    }

    pub fn start_install(&mut self, ipa_path: std::path::PathBuf, cx: &mut Context<Self>) {
        let Some(device) = self.selected_install_device() else {
            self.status = Some("Pair a device first".into());
            cx.notify();
            return;
        };
        let apple = self.settings.apple_id.clone();
        if apple.email.is_empty() || apple.password.is_empty() {
            self.status = Some("Enter your Apple ID in Settings first".into());
            cx.notify();
            return;
        }

        self.install.phase = InstallPhase::Uploading;
        self.install.log = vec![format!("→ Parsing {}", ipa_path.display())];
        cx.notify();

        // Parse IPA metadata + icon on a background thread.
        let path = ipa_path.clone();
        let db = self.db.clone();
        let device2 = device.clone();
        let apple2 = apple.clone();
        cx.spawn(async move |this, cx| {
            let parse = cx
                .background_executor()
                .spawn(async move { crate::ipa::parse_file(&path) })
                .await;

            let (record, opts) = match parse {
                Ok(meta) => {
                    // Persist icon PNG to the tmp dir for later move.
                    let icon_path = meta.icon.as_ref().and_then(|icon| {
                        let dst = paths::tmp_dir()
                            .join(format!("{}.png", meta.bundle_identifier.replace('.', "_")));
                        crate::ipa::write_icon_png(&dst, icon).ok().map(|_| dst)
                    });
                    let mut record = InstalledApp {
                        ipa_name: meta.name.clone(),
                        ipa_path: ipa_path.display().to_string(),
                        device: device2.name.clone(),
                        device_class: device2.device_class.clone(),
                        udid: device2.udid.clone(),
                        account: apple2.email.clone(),
                        password: apple2.password.clone(),
                        bundle_identifier: meta.bundle_identifier.clone(),
                        version: meta.version.clone(),
                        team_id: apple2.team_id.clone(),
                        icon: icon_path
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                        ..Default::default()
                    };
                    record.enabled = true;
                    let opts = InstallOptions {
                        udid: device2.udid.clone(),
                        account: apple2.email.clone(),
                        password: apple2.password.clone(),
                        ipa_path: ipa_path.display().to_string(),
                        remove_extensions: false,
                        team_id: apple2.team_id.clone(),
                        device_name: device2.name.clone(),
                        device_class: device2.device_class.clone(),
                    };
                    (record, opts)
                }
                Err(e) => {
                    let _ = this.update(cx, |s, cx| {
                        s.install.phase = InstallPhase::Error;
                        s.install.log.push(format!("ERROR: {e}"));
                        cx.notify();
                    });
                    return;
                }
            };

            let _ = this.update(cx, |s, cx| {
                s.install.pending = Some(record);
                s.install.phase = InstallPhase::Installing;
                s.install
                    .log
                    .push("→ Install started on backend".into());
                cx.notify();
            });

            let (line_tx, mut line_rx) = mpsc::unbounded_channel::<Line>();
            let (code_tx, code_rx) = mpsc::unbounded_channel::<String>();
            let _ = this.update(cx, |s, _| {
                s.install.code_tx = Some(code_tx);
            });

            // Keep the task: its Result is the ONLY truth about the outcome.
            // Reading success/failure out of the log text marked healthy
            // installs as failed, because plumesign and pymobiledevice3 both
            // print recoverable conditions as "Error: …" while carrying on.
            let run = Tokio::spawn(cx, async move {
                install::run(opts, line_tx, code_rx)
                    .await
                    .map_err(|e| format!("{e:#}"))
            });

            while let Some(line) = line_rx.recv().await {
                let stop = this
                    .update(cx, |s, cx| {
                        let lower = line.text.to_lowercase();
                        s.install.log.push(line.text);
                        s.scrolls.install.scroll_to_bottom();
                        // Text still drives ONE thing: the 2FA prompt, which is
                        // a request for input rather than an outcome.
                        // Stays until submit_2fa clears it: plumesign keeps
                        // printing while it waits, and any of those lines would
                        // otherwise yank the code field away mid-typing.
                        // Skip our own "→" phase lines: "Phase 0/2 … (2FA may
                        // be requested)" would otherwise pop the code field
                        // before plumesign has asked for anything.
                        let ours = lower.starts_with('→');
                        if !ours && (lower.contains("2fa") || lower.contains("verification code")) {
                            s.install.phase = InstallPhase::Awaiting2fa;
                        }
                        cx.notify();
                    })
                    .is_err();
                if stop {
                    break;
                }
            }

            let outcome = match run.await {
                Ok(result) => result,
                Err(join) => Err(join.to_string()),
            };
            let db2 = db.clone();
            let _ = this.update(cx, |s, cx| {
                match outcome {
                    Ok(()) => {
                        s.install.phase = InstallPhase::Success;
                        if let Some(mut rec) = s.install.pending.take() {
                            let _ = db::save_remote_installed(&db2, &mut rec);
                        }
                    }
                    Err(message) => {
                        s.install.phase = InstallPhase::Error;
                        s.install.log.push(format!("✗ {message}"));
                        s.scrolls.install.scroll_to_bottom();
                    }
                }
                s.install.code_tx = None;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn submit_2fa(&mut self, cx: &mut Context<Self>) {
        let code = self.install.otp.trim().to_string();
        if code.is_empty() {
            return;
        }
        if let Some(tx) = &self.install.code_tx {
            let _ = tx.send(code);
        }
        self.install.otp.clear();
        self.install.phase = InstallPhase::Installing;
        cx.notify();
    }

    pub fn reset_install(&mut self, cx: &mut Context<Self>) {
        self.install = InstallUi {
            selected_id: self.install.selected_id.clone(),
            ..Default::default()
        };
        cx.notify();
    }

    // ---- apps actions ---------------------------------------------------

    pub fn refresh_apps(&mut self, cx: &mut Context<Self>) {
        let apps = self.db.list_apps().unwrap_or_default();
        self.apps = apps;
        cx.notify();
    }

    pub fn queue_refresh(&mut self, app: InstalledApp, cx: &mut Context<Self>) {
        // Re-run the install pipeline for an existing app (extends 7 days).
        self.status = Some(format!("Re-signing {} queued", app.ipa_name));
        let apple = self.settings.apple_id.clone();
        let db = self.db.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let opts = InstallOptions {
                udid: app.udid.clone(),
                account: if app.account.is_empty() {
                    apple.email.clone()
                } else {
                    app.account.clone()
                },
                password: if app.password.is_empty() {
                    apple.password.clone()
                } else {
                    app.password.clone()
                },
                ipa_path: app.ipa_path.clone(),
                remove_extensions: app.remove_extensions,
                team_id: apple.team_id.clone(),
                device_name: app.device.clone(),
                device_class: app.device_class.clone(),
            };
            let (line_tx, mut line_rx) = mpsc::unbounded_channel::<Line>();
            let (_code_tx, code_rx) = mpsc::unbounded_channel::<String>();
            let run = Tokio::spawn(cx, async move { install::run(opts, line_tx, code_rx).await });
            while line_rx.recv().await.is_some() {}
            let ok = matches!(run.await, Ok(Ok(())));
            let mut updated = app;
            let now = Utc::now();
            updated.refreshed_date = Some(now);
            updated.expiration_date = Some(now + ChronoDuration::days(7));
            updated.refreshed_result = ok;
            updated.refreshed_error = if ok {
                crate::models::REFRESH_ERROR_NONE
            } else {
                crate::models::REFRESH_ERROR_OTHER
            };
            let _ = db.update_refresh_result(&updated);
            let _ = this.update(cx, |s, cx| {
                s.refresh_apps(cx);
            });
        })
        .detach();
    }

    pub fn delete_app(&mut self, id: i64, cx: &mut Context<Self>) {
        self.apps.retain(|a| a.id != id);
        if let Ok(Some(path)) = self.db.delete_app(id)
            && !path.is_empty()
            && let Some(dir) = std::path::Path::new(&path).parent()
        {
            let _ = std::fs::remove_dir_all(dir);
        }
        cx.notify();
    }

    pub fn toggle_app(&mut self, id: i64, cx: &mut Context<Self>) {
        let _ = self.db.toggle_enabled(id);
        self.refresh_apps(cx);
    }

    pub fn cleanup_duplicates(&mut self, cx: &mut Context<Self>) {
        let deleted = db::cleanup_duplicates(&self.db).unwrap_or(0);
        self.status = Some(format!("Removed {deleted} duplicate(s)"));
        self.refresh_apps(cx);
    }

    pub fn reinstall_app(&mut self, app: InstalledApp, cx: &mut Context<Self>) {
        // Uninstall then re-sign+install.
        let bundle = app.bundle_identifier.clone();
        let udid = app.udid.clone();
        let name = app.device.clone();
        cx.spawn(async move |_this, cx| {
            let _ = Tokio::spawn(cx, async move {
                let _ = teams::uninstall(&udid, &name, &bundle).await;
            })
            .await;
        })
        .detach();
        self.queue_refresh(app, cx);
    }

    // ---- settings -------------------------------------------------------

    pub fn save_apple_id(&mut self, cx: &mut Context<Self>) {
        self.settings.save();
        self.status = Some("Apple ID saved".into());
        cx.notify();
    }

    pub fn clear_apple_id(&mut self, cx: &mut Context<Self>) {
        self.settings.apple_id = Default::default();
        self.settings.save();
        self.teams.clear();
        self.status = Some("Apple ID cleared".into());
        cx.notify();
    }

    pub fn start_tunneld(&mut self, cx: &mut Context<Self>) {
        if self.starting_tunneld {
            return;
        }
        self.starting_tunneld = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = Tokio::spawn(cx, async { tunneld::start_with_admin().await })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!(e.to_string())));
            let _ = this.update(cx, |s, cx| {
                s.starting_tunneld = false;
                match result {
                    Ok(()) => {
                        s.tunneld_running = true;
                        s.status = Some("Tunneld running".into());
                    }
                    Err(e) => s.status = Some(format!("Failed to start tunneld: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sign the saved Apple ID into plumesign (`account login`), streaming the
    /// transcript into the Settings log and surfacing the OTP field when Apple
    /// asks for a 2FA code. Every other plumesign command needs this session,
    /// so without it `account teams` fails with "Account not found".
    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        let apple = self.settings.apple_id.clone();
        if apple.email.is_empty() || apple.password.is_empty() {
            self.status = Some("Enter your Apple ID email and password first".into());
            cx.notify();
            return;
        }
        if self.login.phase == LoginPhase::Running
            || self.login.phase == LoginPhase::Awaiting2fa
        {
            return;
        }

        self.login = LoginUi {
            phase: LoginPhase::Running,
            log: vec![format!("→ Signing in as {}", apple.email)],
            ..Default::default()
        };
        let (line_tx, mut line_rx) = mpsc::unbounded_channel::<Line>();
        let (code_tx, code_rx) = mpsc::unbounded_channel::<String>();
        self.login.code_tx = Some(code_tx);
        cx.notify();

        Tokio::spawn(cx, async move {
            let _ = teams::sign_in(&apple.email, &apple.password, &apple.team_id, line_tx, code_rx)
                .await;
        })
        .detach();

        cx.spawn(async move |this, cx| {
            while let Some(line) = line_rx.recv().await {
                let stop = this
                    .update(cx, |s, cx| {
                        let lower = line.text.to_lowercase();
                        s.login.log.push(line.text);
                        s.scrolls.login.scroll_to_bottom();
                        if lower.contains("2fa") || lower.contains("verification code") {
                            s.login.phase = LoginPhase::Awaiting2fa;
                        }
                        cx.notify();
                    })
                    .is_err();
                if stop {
                    return;
                }
            }
            // The stream closed: the subprocess exited. Decide the outcome from
            // the transcript, then refresh the team list if we got in.
            let succeeded = this
                .read_with(cx, |s, _| {
                    let text = s.login.log.join("\n").to_lowercase();
                    !text.contains("error") && !text.contains("can't log-in")
                })
                .unwrap_or(false);
            let _ = this.update(cx, |s, cx| {
                s.login.phase = if succeeded {
                    LoginPhase::Success
                } else {
                    LoginPhase::Error
                };
                s.login.code_tx = None;
                cx.notify();
                if succeeded {
                    s.load_teams(cx);
                }
            });
        })
        .detach();
    }

    pub fn submit_login_2fa(&mut self, cx: &mut Context<Self>) {
        let code = self.login.otp.trim().to_string();
        if code.is_empty() {
            return;
        }
        if let Some(tx) = &self.login.code_tx {
            let _ = tx.send(code);
        }
        self.login.otp.clear();
        self.login.phase = LoginPhase::Running;
        cx.notify();
    }

    pub fn load_teams(&mut self, cx: &mut Context<Self>) {
        let email = self.settings.apple_id.email.clone();
        if email.is_empty() {
            self.status = Some("Save Apple ID first".into());
            cx.notify();
            return;
        }
        self.loading_teams = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = Tokio::spawn(cx, async move { teams::list_teams(&email).await })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!(e.to_string())));
            let _ = this.update(cx, |s, cx| {
                s.loading_teams = false;
                match result {
                    Ok(teams) => {
                        if teams.is_empty() {
                            s.status = Some("No teams found (is the account logged in?)".into());
                        }
                        s.teams = teams;
                    }
                    Err(e) => s.status = Some(format!("Failed to list teams: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// Merge DB-persisted pairs in as authoritative "paired" entries so the Install
/// tab is populated even before mDNS catches up (mirrors the frontend's
/// dbPaired merge).
fn merge_db_paired(devices: &mut Vec<Device>, db: &Db) {
    let paired: Vec<PairedDevice> = db.list_paired().unwrap_or_default();
    for p in paired {
        let exists = devices.iter().any(|d| {
            d.name.eq_ignore_ascii_case(&p.name)
                && d.class_lower() == p.device_class.to_lowercase()
        });
        if !exists {
            devices.push(Device {
                id: format!("db-{}", p.name),
                name: p.name,
                service_name: String::new(),
                ip: p.ip,
                mac_addr: String::new(),
                udid: String::new(),
                status: DeviceStatus::Paired,
                device_class: p.device_class,
                product_type: String::new(),
                product_version: String::new(),
                port: 0,
            });
        }
    }
}
