//! 无 UI 依赖的小工具：颜色互转、文件名、尺寸适配、时间戳、PNG 后缀。

#![allow(unused_imports)]
use eframe::egui::{Color32, Vec2};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::algo::Img;

pub(crate) fn rgb_of(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}


pub(crate) fn color32_from(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}


pub(crate) fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}


pub(crate) fn file_name_of(p: &Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}


pub(crate) fn fit_size(max: Vec2, w: usize, h: usize) -> Vec2 {
    if w == 0 || h == 0 {
        return Vec2::ZERO;
    }
    let s = (max.x / w as f32).min(max.y / h as f32).min(8.0);
    Vec2::new(w as f32 * s, h as f32 * s)
}


pub(crate) fn ensure_png_ext(path: PathBuf) -> PathBuf {
    const KNOWN: [&str; 17] = [
        "png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "tga", "pam",
        "webp", "ico", "ff", "qoi", "dds", "hdr", "exr", "avif",
    ];
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) {
        Some(ext) if KNOWN.contains(&ext.as_str()) => path,
        _ => {
            let mut os = path.into_os_string();
            os.push(".png");
            PathBuf::from(os)
        }
    }
}

/// 生成/更新预览纹理

pub(crate) fn civil_now() -> (i32, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 + 8 * 3600)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    (
        y as i32,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// 天数 → 年月日（Howard Hinnant 算法）

pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// 字体与主题
// ---------------------------------------------------------------------------
