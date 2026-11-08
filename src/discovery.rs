use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::db::Db;
use crate::models::{CLASS_APPLE_TV, CLASS_IPHONE, Device, DeviceStatus};

const SVC_LEGACY: &str = "_apple-mobdev2._tcp.local.";
const SVC_REMOTE: &str = "_remotepairing._tcp.local.";
const SVC_MANUAL: &str = "_remotepairing-manual-pairing._tcp.local.";
pub fn scan(db: &Db, timeout: Duration) -> Result<Vec<Device>> {
    let mut devices = scan_usbmuxd();
    devices.extend(browse(SVC_MANUAL, CLASS_APPLE_TV, DeviceStatus::Pairable, timeout)?);
    devices.extend(browse(SVC_REMOTE, CLASS_APPLE_TV, DeviceStatus::Unpaired, timeout)?);
    devices.extend(browse(SVC_LEGACY, CLASS_IPHONE, DeviceStatus::Unpaired, timeout)?);

    let mut result = dedupe(devices);
    result = merge_across_sources(result);
    apply_paired_registry(db, &mut result);
    Ok(result)
}

fn merge_across_sources(input: Vec<Device>) -> Vec<Device> {
    let normalize = |name: &str| -> String {
        name.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect()
    };

    let mut out: Vec<Device> = Vec::with_capacity(input.len());
    for device in input {
        let key = normalize(&device.name);
        // An empty or class-only name is too weak a join key to merge on.
        if key.is_empty() {
            out.push(device);
            continue;
        }
        match out
            .iter_mut()
            .find(|existing| normalize(&existing.name) == key)
        {
            Some(existing) => {
                if existing.udid.is_empty()
                    || (Device::is_remote_udid(&existing.udid)
                        && !Device::is_remote_udid(&device.udid)
                        && !device.udid.is_empty())
                {
                    // Prefer a hardware UDID over a rotating mDNS identifier.
                    existing.udid = device.udid.clone();
                }
                if existing.ip.is_empty() || (!device.ip.is_empty() && !existing.ip.contains('.')) {
                    existing.ip = device.ip.clone();
                }
                if existing.product_type.is_empty() {
                    existing.product_type = device.product_type.clone();
                }
                if existing.product_version.is_empty() {
                    existing.product_version = device.product_version.clone();
                }
                if existing.service_name.is_empty() {
                    existing.service_name = device.service_name.clone();
                }
                // Paired (a real pair record) outranks Pairable (in pairing
                // mode right now), which outranks Unpaired (merely on the net).
                let rank = |s: DeviceStatus| match s {
                    DeviceStatus::Paired => 2,
                    DeviceStatus::Pairable => 1,
                    DeviceStatus::Unpaired => 0,
                };
                if rank(device.status) > rank(existing.status) {
                    existing.status = device.status;
                }
                // The manual-pairing advert is the only one carrying a port,
                // and its identifier is the one the pair helper must use.
                if device.port != 0 {
                    existing.port = device.port;
                    if !device.udid.is_empty() {
                        existing.udid = device.udid.clone();
                    }
                }
            }
            None => out.push(device),
        }
    }
    out
}

fn scan_usbmuxd() -> Vec<Device> {
    let muxed = match crate::usbmux::list_devices() {
        Ok(devices) => devices,
        Err(e) => {
            tracing::debug!(error = %e, "usbmuxd enumeration unavailable");
            return Vec::new();
        }
    };

    muxed
        .into_iter()
        .map(|d| {
            let info = crate::usbmux::device_info(d.device_id).unwrap_or_default();
            let name = if info.device_name.is_empty() {
                if info.product_type.is_empty() {
                    format!("iPhone ({})", d.connection_type)
                } else {
                    info.product_type.clone()
                }
            } else {
                info.device_name.clone()
            };
            let device_class = if info.device_class.is_empty() {
                Device::class_from_name(&info.product_type, CLASS_IPHONE)
            } else {
                info.device_class.clone()
            };
            Device {
                id: format!("{:x}", md5::compute(&d.udid)),
                name,
                service_name: String::new(),
                ip: d.ip.clone(),
                mac_addr: String::new(),
                udid: d.udid,
                status: DeviceStatus::Paired,
                device_class,
                product_type: info.product_type,
                product_version: info.product_version,
                port: 0,
            }
        })
        .collect()
}

fn browse(
    service: &str,
    default_class: &str,
    status: DeviceStatus,
    timeout: Duration,
) -> Result<Vec<Device>> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(service)?;
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();

    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let ip = info
                    .get_addresses()
                    .iter()
                    .map(|a| a.to_string())
                    .min_by_key(|a| if a.contains('.') { 0 } else { 1 });
                let Some(ip) = ip else { continue };

                let host = strip_local(info.get_hostname());
                let service_name = info.get_fullname().to_string();
                let mac = service_name
                    .split('@')
                    .next()
                    .unwrap_or("")
                    .to_string();

                // Extract identifier= from TXT records (present on _remotepairing).
                let udid = info
                    .get_properties()
                    .iter()
                    .find(|p| p.key() == "identifier")
                    .and_then(|p| p.val_str().to_string().into())
                    .unwrap_or_default();

                let name = if host.is_empty() {
                    service_name.clone()
                } else {
                    host.clone()
                };
                let device_class = Device::class_from_name(&name, default_class);

                out.push(Device {
                    id: format!("{:x}", md5::compute(&service_name)),
                    name,
                    service_name,
                    ip,
                    mac_addr: mac,
                    udid,
                    status,
                    device_class,
                    product_type: String::new(),
                    product_version: String::new(),
                    // Only the manual-pairing port accepts pair-setup. The
                    // _remotepairing advert has its own port (49152), and if
                    // it leaked in here the merge would overwrite the real
                    // pairing port and the device would drop the connection.
                    port: if service == SVC_MANUAL { info.get_port() } else { 0 },
                });
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    Ok(out)
}

fn strip_local(host: &str) -> String {
    host.trim_end_matches('.')
        .trim_end_matches(".local")
        .to_string()
}

fn dedupe(input: Vec<Device>) -> Vec<Device> {
    let key_of = |d: &Device| -> String {
        if !d.udid.is_empty() {
            format!("u:{}", d.udid)
        } else {
            format!("nc:{}/{}", d.name.to_lowercase(), d.class_lower())
        }
    };
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Device> = Vec::new();
    for d in input {
        let key = key_of(&d);
        if let Some(&idx) = seen.get(&key) {
            let existing_v4 = out[idx].ip.contains('.');
            let new_v4 = d.ip.contains('.');
            if new_v4 && !existing_v4 {
                out[idx] = d;
            }
            continue;
        }
        seen.insert(key, out.len());
        out.push(d);
    }
    out
}

fn apply_paired_registry(db: &Db, devices: &mut [Device]) {
    let Ok(records) = db.list_paired() else {
        return;
    };
    let paired: std::collections::HashSet<String> = records
        .iter()
        .map(|r| format!("{}/{}", r.name.to_lowercase(), r.device_class.to_lowercase()))
        .collect();
    for d in devices.iter_mut() {
        let key = format!("{}/{}", d.name.to_lowercase(), d.class_lower());
        if paired.contains(&key) {
            d.status = DeviceStatus::Paired;
        }
    }
}

pub fn sort_devices(devices: &mut [Device]) {
    devices.sort_by(|a, b| {
        let atv_a = a.class_lower().contains("appletv");
        let atv_b = b.class_lower().contains("appletv");
        atv_b
            .cmp(&atv_a)
            .then(a.class_lower().cmp(&b.class_lower()))
            .then(a.name.cmp(&b.name))
    });
}
