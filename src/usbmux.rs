use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Result, bail};

const SOCKET: &str = "/var/run/usbmuxd";
const LOCKDOWN_PORT: u16 = 62078;
const PROG_NAME: &str = "ipabox";
const HEADER_LEN: u32 = 16;
const VERSION_PLIST: u32 = 1;
const MESSAGE_PLIST: u32 = 8;

#[derive(Debug, Clone)]
pub struct MuxDevice {
    pub device_id: u64,
    pub udid: String,
    pub connection_type: String,
    pub ip: String,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub device_name: String,
    pub product_type: String,
    pub product_version: String,
    pub device_class: String,
}

fn connect() -> Result<UnixStream> {
    let stream = UnixStream::connect(SOCKET)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    Ok(stream)
}

fn send_mux(stream: &mut UnixStream, payload: &plist::Value, tag: u32) -> Result<()> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, payload)?;
    let total = HEADER_LEN + body.len() as u32;
    stream.write_all(&total.to_le_bytes())?;
    stream.write_all(&VERSION_PLIST.to_le_bytes())?;
    stream.write_all(&MESSAGE_PLIST.to_le_bytes())?;
    stream.write_all(&tag.to_le_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;
    Ok(())
}

fn recv_mux(stream: &mut UnixStream) -> Result<plist::Value> {
    let mut header = [0u8; 16];
    stream.read_exact(&mut header)?;
    let total = u32::from_le_bytes(header[0..4].try_into()?);
    if total < HEADER_LEN {
        bail!("usbmuxd sent a short packet ({total} bytes)");
    }
    let mut body = vec![0u8; (total - HEADER_LEN) as usize];
    stream.read_exact(&mut body)?;
    Ok(plist::from_bytes(&body)?)
}

fn dict(pairs: &[(&str, plist::Value)]) -> plist::Value {
    let mut d = plist::Dictionary::new();
    for (k, v) in pairs {
        d.insert((*k).to_string(), v.clone());
    }
    plist::Value::Dictionary(d)
}

fn base_request(message_type: &str) -> Vec<(&'static str, plist::Value)> {
    vec![
        ("MessageType", message_type.to_string().into()),
        ("ClientVersionString", PROG_NAME.into()),
        ("ProgName", PROG_NAME.into()),
        ("kLibUSBMuxVersion", 3i64.into()),
    ]
}

pub fn list_devices() -> Result<Vec<MuxDevice>> {
    let mut stream = connect()?;
    send_mux(&mut stream, &dict(&base_request("ListDevices")), 1)?;
    let response = recv_mux(&mut stream)?;

    let Some(list) = response
        .as_dictionary()
        .and_then(|d| d.get("DeviceList"))
        .and_then(|v| v.as_array())
    else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for entry in list {
        let Some(entry) = entry.as_dictionary() else {
            continue;
        };
        let device_id = entry
            .get("DeviceID")
            .and_then(|v| v.as_unsigned_integer())
            .unwrap_or(0);
        let Some(props) = entry.get("Properties").and_then(|v| v.as_dictionary()) else {
            continue;
        };
        let udid = props
            .get("SerialNumber")
            .and_then(|v| v.as_string())
            .unwrap_or_default()
            .to_string();
        if udid.is_empty() || udid.contains(':') {
            continue;
        }
        let connection_type = props
            .get("ConnectionType")
            .and_then(|v| v.as_string())
            .unwrap_or("USB")
            .to_string();
        let ip = props
            .get("NetworkAddress")
            .and_then(|v| v.as_data())
            .and_then(parse_network_address)
            .unwrap_or_default();
        out.push(MuxDevice {
            device_id,
            udid,
            connection_type,
            ip,
        });
    }
    Ok(out)
}

fn parse_network_address(bytes: &[u8]) -> Option<String> {
    match bytes.first()? {
        16 if bytes.len() >= 8 => {
            let octets: [u8; 4] = bytes[4..8].try_into().ok()?;
            Some(std::net::Ipv4Addr::from(octets).to_string())
        }
        28 if bytes.len() >= 32 => {
            let octets: [u8; 16] = bytes[16..32].try_into().ok()?;
            Some(std::net::Ipv6Addr::from(octets).to_string())
        }
        _ => None,
    }
}

pub fn device_info(device_id: u64) -> Result<DeviceInfo> {
    let mut stream = connect()?;
    // usbmuxd wants the port in network byte order.
    let port = LOCKDOWN_PORT.to_be() as i64;
    let mut req = base_request("Connect");
    req.push(("DeviceID", (device_id as i64).into()));
    req.push(("PortNumber", port.into()));
    send_mux(&mut stream, &dict(&req), 2)?;

    let response = recv_mux(&mut stream)?;
    let number = response
        .as_dictionary()
        .and_then(|d| d.get("Number"))
        .and_then(|v| v.as_unsigned_integer())
        .unwrap_or(1);
    if number != 0 {
        bail!("usbmuxd refused the lockdown connect (Number={number})");
    }

    let mut get = |key: &str| lockdown_get_value(&mut stream, key).unwrap_or_default();
    Ok(DeviceInfo {
        device_name: get("DeviceName"),
        product_type: get("ProductType"),
        product_version: get("ProductVersion"),
        device_class: get("DeviceClass"),
    })
}

fn lockdown_get_value(stream: &mut UnixStream, key: &str) -> Result<String> {
    let request = dict(&[
        ("Request", "GetValue".into()),
        ("Key", key.into()),
        ("Label", PROG_NAME.into()),
    ]);
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &request)?;
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > 1 << 20 {
        bail!("lockdown replied with an implausible length ({len})");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    let value: plist::Value = plist::from_bytes(&body)?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| anyhow::anyhow!("lockdown reply is not a dictionary"))?;
    if let Some(error) = dict.get("Error").and_then(|v| v.as_string()) {
        bail!("lockdown refused {key}: {error}");
    }
    Ok(dict
        .get("Value")
        .and_then(|v| v.as_string())
        .unwrap_or_default()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_sockaddr() {
        let mut bytes = vec![16u8, 2, 0, 0];
        bytes.extend_from_slice(&[192, 168, 1, 40]);
        assert_eq!(parse_network_address(&bytes).as_deref(), Some("192.168.1.40"));
    }

    #[test]
    fn rejects_unknown_family() {
        assert!(parse_network_address(&[99, 0, 0, 0]).is_none());
        assert!(parse_network_address(&[]).is_none());
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore = "diagnostic: needs a real usbmuxd, prints rather than asserts"]
    fn probe_daemon() {
        match super::list_devices() {
            Ok(devices) => {
                println!("usbmuxd listed {} device(s)", devices.len());
                for d in &devices {
                    let info = super::device_info(d.device_id).unwrap_or_default();
                    println!(
                        "  id={} udid={} conn={} ip={:?} name={:?} type={:?} ver={:?} class={:?}",
                        d.device_id, d.udid, d.connection_type, d.ip,
                        info.device_name, info.product_type, info.product_version, info.device_class
                    );
                }
            }
            Err(e) => println!("usbmuxd error: {e}"),
        }
    }
}
