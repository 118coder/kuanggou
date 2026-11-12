//! 规格预设表与「设置 → 实际输出规格」的解析。
//!
//! 预设只是常量数据；新增一个平台规格只需往 `SPECS` 里加一行，
//! UI 下拉会自动出现，其它模块无需改动。

use crate::video::types::{FitMode, OutputSpec, VideoInfo, VideoSettings};

/// 一条分辨率预设；`w=h=0` 表示「按源尺寸」或「用自定义值」。
#[derive(Clone, Copy, Debug)]
pub struct SpecPreset {
    pub key: &'static str,
    pub label: &'static str,
    pub w: u32,
    pub h: u32,
    pub note: &'static str,
}

pub const SPECS: &[SpecPreset] = &[
    SpecPreset { key: "source", label: "原始尺寸", w: 0, h: 0, note: "不改分辨率" },
    SpecPreset { key: "2160p", label: "4K 3840×2160", w: 3840, h: 2160, note: "16:9 横屏" },
    SpecPreset { key: "1080p", label: "1080P 1920×1080", w: 1920, h: 1080, note: "16:9 横屏 · B 站 / 抖音横版" },
    SpecPreset { key: "720p", label: "720P 1280×720", w: 1280, h: 720, note: "16:9 横屏 · 体积友好" },
    SpecPreset { key: "480p", label: "480P 854×480", w: 854, h: 480, note: "16:9 横屏 · 快速预览" },
    SpecPreset { key: "portrait", label: "竖屏 1080×1920", w: 1080, h: 1920, note: "9:16 竖屏 · 抖音 / 快手" },
    SpecPreset { key: "square", label: "方形 1080×1080", w: 1080, h: 1080, note: "1:1 方形 · 朋友圈" },
    SpecPreset { key: "sticker", label: "表情 480×480", w: 480, h: 480, note: "1:1 · GIF 常用" },
    SpecPreset { key: "wechat", label: "微信表情 240×240", w: 240, h: 240, note: "1:1 · 小体积" },
    SpecPreset { key: "custom", label: "自定义尺寸", w: 0, h: 0, note: "在下方输入宽高" },
];

pub fn spec_by_key(key: &str) -> &'static SpecPreset {
    SPECS.iter().find(|p| p.key == key).unwrap_or(&SPECS[3])
}

/// 帧率预设：`source` 跟随源视频，其余为固定帧率，`custom` 用 fps_custom。
pub const FPS: &[(&str, &str)] = &[
    ("source", "跟随源"), ("8", "8 fps"), ("10", "10 fps"), ("12", "12 fps"),
    ("15", "15 fps"), ("20", "20 fps"), ("24", "24 fps"), ("25", "25 fps"),
    ("30", "30 fps"), ("50", "50 fps"), ("60", "60 fps"), ("custom", "自定义"),
];

/// GIF 体积上限预设（MB），0 表示不限制。
pub const SIZE_LIMITS: &[(&str, &str)] = &[
    ("0", "不限制"), ("1", "≤ 1 MB"), ("2", "≤ 2 MB"), ("3", "≤ 3 MB"),
    ("5", "≤ 5 MB"), ("8", "≤ 8 MB"), ("10", "≤ 10 MB"), ("20", "≤ 20 MB"),
    ("custom", "自定义"),
];

pub const GIF_COLORS: &[(u16, &str)] = &[
    (32, "32 色（最小）"), (64, "64 色"), (128, "128 色"), (256, "256 色（最清晰）"),
];

/// 让宽高变成编码器友好的偶数，且不小于 2。
fn even(v: u32) -> u32 {
    v.max(2) & !1
}

/// 把界面设置解析成实际输出规格（分辨率、帧率、区间、帧数、GIF 帧延时）。
pub fn resolve_spec(s: &VideoSettings, info: &VideoInfo) -> OutputSpec {
    let preset = spec_by_key(&s.preset);
    let (mut w, mut h) = if preset.key == "custom" {
        (s.custom_w, s.custom_h)
    } else if preset.key == "source" {
        (info.w, info.h)
    } else {
        (preset.w, preset.h)
    };
    w = even(w.min(8192));
    h = even(h.min(8192));

    let fps = if s.fps_preset == "source" {
        info.fps
    } else if s.fps_preset == "custom" {
        s.fps_custom as f64
    } else {
        s.fps_preset.parse::<f64>().unwrap_or(15.0)
    }
    .clamp(1.0, 120.0);

    let dur_total = info.duration.max(0.01);
    let start = (s.trim_start as f64).max(0.0).min(dur_total);
    let mut end = if s.trim_end > 0.01 { s.trim_end as f64 } else { dur_total };
    end = end.clamp(start + 0.01, dur_total);
    let duration = end - start;
    let frames = (duration * fps).ceil().max(1.0) as u64;
    let delay_ms = (1000.0 / fps).round().clamp(10.0, 1000.0) as u32;

    OutputSpec { w, h, fps, start, end, duration, frames, delay_ms }
}

/// 实际写到画布上的缩放尺寸（供预览与估算共用）。
pub fn fit_size(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32, fit: FitMode) -> (u32, u32) {
    match fit {
        FitMode::Stretch => (dst_w, dst_h),
        FitMode::Contain => {
            let k = (dst_w as f64 / src_w as f64).min(dst_h as f64 / src_h as f64);
            ((src_w as f64 * k).round().max(2.0) as u32, (src_h as f64 * k).round().max(2.0) as u32)
        }
        FitMode::Cover => {
            let k = (dst_w as f64 / src_w as f64).max(dst_h as f64 / src_h as f64);
            ((src_w as f64 * k).round().max(2.0) as u32, (src_h as f64 * k).round().max(2.0) as u32)
        }
    }
}
