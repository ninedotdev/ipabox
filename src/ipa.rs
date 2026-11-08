use std::io::Read;
use std::path::Path;

use anyhow::{Result, bail};
use regex::Regex;

pub struct IpaMetadata {
    pub name: String,
    pub bundle_identifier: String,
    pub version: String,
    pub icon: Option<(u32, u32, Vec<u8>)>,
}

pub fn parse_file(path: &Path) -> Result<IpaMetadata> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let re_info = Regex::new(r"^Payload/.*\.app/Info\.plist$").unwrap();
    let re_new = Regex::new(
        r"^Payload/.*\.app/AppIcon-?_?\w*(\d+(\.\d+)?)x(\d+(\.\d+)?)(@\dx)?(~ipad)?\.png$",
    )
    .unwrap();
    let re_old = Regex::new(r"^Payload/.*\.app/Icon-?_?\w*(\d+(\.\d+)?)?\.png$").unwrap();

    let mut info_name = None;
    let mut icon_candidates: Vec<(String, i32)> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if re_info.is_match(&name) {
            info_name = Some(name.clone());
        }
        if re_new.is_match(&name) || re_old.is_match(&name) {
            let size = icon_size(&name, &re_old, &re_new);
            icon_candidates.push((name, size));
        }
    }

    let Some(info_name) = info_name else {
        bail!("Info.plist not found");
    };

    let plist = {
        let mut f = archive.by_name(&info_name)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        buf
    };
    let info: plist::Value = plist::from_bytes(&plist)?;
    let dict = info
        .as_dictionary()
        .ok_or_else(|| anyhow::anyhow!("Info.plist is not a dictionary"))?;
    let get = |key: &str| {
        dict.get(key)
            .and_then(|v| v.as_string())
            .unwrap_or("")
            .to_string()
    };
    let name = {
        let display = get("CFBundleDisplayName");
        if display.is_empty() {
            get("CFBundleName")
        } else {
            display
        }
    };
    let bundle_identifier = get("CFBundleIdentifier");
    let version = get("CFBundleShortVersionString");

    // Pick the largest icon and decode it.
    icon_candidates.sort_by_key(|(_, s)| *s);
    let mut icon = None;
    if let Some((icon_name, _)) = icon_candidates.last() {
        let mut f = archive.by_name(icon_name)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        icon = decode_icon(&buf);
    }

    Ok(IpaMetadata {
        name,
        bundle_identifier,
        version,
        icon,
    })
}

fn icon_size(name: &str, re_old: &Regex, re_new: &Regex) -> i32 {
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if re_old.is_match(name) {
        let parts: Vec<&str> = stem.split('-').collect();
        if parts.len() == 2 {
            return parts[1].parse::<f64>().unwrap_or(0.0) as i32;
        }
        return 160;
    }
    if re_new.is_match(name) {
        if let Some(after_x) = stem.split('@').next().and_then(|s| s.split('x').nth(1)) {
            let digits = after_x.split('~').next().unwrap_or(after_x);
            let mut size = digits.parse::<f64>().unwrap_or(0.0);
            if stem.contains("@2x") {
                size *= 2.0;
            } else if stem.contains("@3x") {
                size *= 3.0;
            }
            return size as i32;
        }
    }
    0
}

fn decode_icon(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if let Ok(img) = image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
        let rgba = img.to_rgba8();
        return Some((rgba.width(), rgba.height(), rgba.into_raw()));
    }
    decode_cgbi(bytes)
}

fn decode_cgbi(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() < 8 || bytes[..8] != SIG {
        return None;
    }
    let mut pos = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat = Vec::new();
    let mut is_cgbi = false;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;

    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        if data_start + len + 4 > bytes.len() {
            break;
        }
        let data = &bytes[data_start..data_start + len];
        match ctype {
            b"CgBI" => is_cgbi = true,
            b"IHDR" => {
                width = u32::from_be_bytes(data[0..4].try_into().ok()?);
                height = u32::from_be_bytes(data[4..8].try_into().ok()?);
                bit_depth = data[8];
                color_type = data[9];
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos = data_start + len + 4; 
    }

    if !is_cgbi || width == 0 || height == 0 || bit_depth != 8 || color_type != 6 {
        return None;
    }

    let mut decoder = flate2::read::DeflateDecoder::new(idat.as_slice());
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).ok()?;

    let bpp = 4usize;
    let stride = width as usize * bpp;
    let mut out = vec![0u8; height as usize * stride];
    let mut prev_row = vec![0u8; stride];
    let mut src = 0usize;
    for y in 0..height as usize {
        if src >= raw.len() {
            return None;
        }
        let filter = raw[src];
        src += 1;
        let row_src = &raw[src..(src + stride).min(raw.len())];
        if row_src.len() < stride {
            return None;
        }
        let row_out = &mut out[y * stride..(y + 1) * stride];
        for x in 0..stride {
            let a = if x >= bpp { row_out[x - bpp] } else { 0 };
            let b = prev_row[x];
            let c = if x >= bpp { prev_row[x - bpp] } else { 0 };
            let raw_val = row_src[x] as i32;
            let recon = match filter {
                0 => raw_val,
                1 => raw_val + a as i32,
                2 => raw_val + b as i32,
                3 => raw_val + ((a as i32 + b as i32) / 2),
                4 => raw_val + paeth(a as i32, b as i32, c as i32),
                _ => raw_val,
            };
            row_out[x] = (recon & 0xff) as u8;
        }
        prev_row.copy_from_slice(row_out);
        src += stride;
    }

    for px in out.chunks_exact_mut(4) {
        let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
        if a > 0 {
            px[0] = ((r as u16 * 255 / a as u16).min(255)) as u8;
            px[1] = ((g as u16 * 255 / a as u16).min(255)) as u8;
            px[2] = ((b as u16 * 255 / a as u16).min(255)) as u8;
        } else {
            px[0] = r;
            px[1] = g;
            px[2] = b;
        }
        px[3] = a;
    }
    Some((width, height, out))
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub fn write_icon_png(dst: &Path, icon: &(u32, u32, Vec<u8>)) -> Result<()> {
    let (w, h, pixels) = icon;
    let buffer = image::RgbaImage::from_raw(*w, *h, pixels.clone())
        .ok_or_else(|| anyhow::anyhow!("invalid icon buffer"))?;
    buffer.save_with_format(dst, image::ImageFormat::Png)?;
    Ok(())
}
