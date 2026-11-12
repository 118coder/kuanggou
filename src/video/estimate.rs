//! 导出体积预估与「GIF 体积上限」的参数规划。
//!
//! 预估分两档：默认用经验模型即时给数；点「精确预估」后抽若干帧实际编码一次，
//! 得到每帧字节率回填 `Measured`，预估误差可收敛到 ±20% 内。
//! `plan()` 在体积上限被触发时自动下调帧率 / 分辨率 / 颜色数，并生成给人看的
//! 调整说明；它只改导出参数，不改动界面上的用户设置。

use crate::video::presets::resolve_spec;
use crate::video::types::{ExportPlan, FitMode, LimitStrategy, OutFormat, OutputSpec, VideoInfo, VideoSettings};

/// 一次「样本试编码」得到的实测密度。
#[derive(Clone, Copy, Debug)]
pub struct Measured {
    pub bytes_per_frame: f64,
    pub pixels: u64,
    pub colors: u16,
    pub dither: bool,
}

#[derive(Clone, Debug)]
pub struct Estimate {
    pub bytes: u64,
    pub low: u64,
    pub high: u64,
    pub method: &'static str,
    pub note: String,
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1024.0 { return format!("{bytes} B"); }
    if b < 1024.0 * 1024.0 { return format!("{:.1} KB", b / 1024.0); }
    if b < 1024.0 * 1024.0 * 1024.0 { return format!("{:.2} MB", b / 1024.0 / 1024.0); }
    format!("{:.2} GB", b / 1024.0 / 1024.0 / 1024.0)
}

/// GIF 体积预估：`frames × (像素数 × 每像素字节 + 每帧调色板开销)`。
pub fn estimate_gif(spec: &OutputSpec, colors: u16, dither: bool, m: Option<&Measured>) -> Estimate {
    let pixels = spec.w as f64 * spec.h as f64;
    let (bpp, method, lo, hi) = match m {
        Some(mm) => {
            let base = (mm.bytes_per_frame / mm.pixels.max(1) as f64).clamp(0.015, 1.6);
            let ck = (colors as f64 / mm.colors.max(1) as f64).clamp(0.55, 1.6).powf(0.35);
            (base * ck, "样本试编码", 0.86, 1.18)
        }
        // 经验值：GIF 每像素每帧约 0.1 字节（实测 480p 素材 0.02–0.15，随画面复杂度浮动）。
        None => (0.10, "经验模型", 0.45, 1.85),
    };
    let dither_k = if dither { 1.10 } else { 1.0 };
    let color_k = 0.60 + (colors as f64 / 256.0) * 0.50;
    let color_k = if m.is_some() { 1.0 } else { color_k };
    let per_frame = pixels * bpp * color_k * dither_k + 840.0; // 本地调色板 + LZW 块头
    let total = spec.frames as f64 * per_frame + 2048.0;
    let bytes = total.max(1024.0) as u64;
    Estimate {
        bytes,
        low: (bytes as f64 * lo) as u64,
        high: (bytes as f64 * hi) as u64,
        method,
        note: format!("{} 帧 × {}×{} · {} 色", spec.frames, spec.w, spec.h, colors),
    }
}

/// 视频容器体积预估：有损编码按码率 / CRF，无损类按每像素字节数。
pub fn estimate_video(fmt: OutFormat, spec: &OutputSpec, s: &VideoSettings) -> Estimate {
    let pixels = spec.w as f64 * spec.h as f64;
    let (bytes, method, note) = match fmt {
        OutFormat::Mp4Flat => {
            let b = s.bitrate_kbps as f64 * 1000.0 / 8.0 * spec.duration * 1.02;
            (b, "按设定码率", format!("{} kbps × {:.1}s", s.bitrate_kbps, spec.duration))
        }
        OutFormat::WebmAlpha => {
            let q = (30.0 / (s.crf as f64 + 5.0)).clamp(0.45, 2.0);
            (pixels * spec.frames as f64 * 0.10 * q * 1.02, "按 CRF 经验值", format!("CRF {} · 透明 VP9", s.crf))
        }
        OutFormat::MovProres => (pixels * spec.frames as f64 * 0.95 * 1.01, "按 ProRes 4444 码流", String::from("近似无损 · 体积较大")),
        OutFormat::MovQtrle => (pixels * spec.frames as f64 * 0.72 * 1.01, "按 RLE 经验值", String::from("无损 RLE · 内容越简单越小")),
        OutFormat::Apng => (pixels * spec.frames as f64 * 0.45 * 1.01, "按 PNG 压缩经验值", String::from("无损 · 支持透明")),
        OutFormat::PngSeq => (pixels * spec.frames as f64 * 0.50 * 1.01, "按 PNG 压缩经验值", String::from("逐帧文件总和")),
        OutFormat::Gif => (0.0, "", String::new()),
    };
    let bytes = bytes.max(4096.0) as u64;
    Estimate {
        bytes,
        low: (bytes as f64 * 0.75) as u64,
        high: (bytes as f64 * 1.35) as u64,
        method,
        note: note.to_string(),
    }
}

pub fn estimate(fmt: OutFormat, spec: &OutputSpec, s: &VideoSettings, m: Option<&Measured>) -> Estimate {
    if fmt.is_gif() {
        estimate_gif(spec, s.gif_colors, s.gif_dither, m)
    } else {
        estimate_video(fmt, spec, s)
    }
}

fn scaled_spec(base: &OutputSpec, fps: f64, w: u32, h: u32) -> OutputSpec {
    let fps = fps.clamp(4.0, 120.0);
    let w = w.max(160) & !1;
    let h = h.max(120) & !1;
    let frames = (base.duration * fps).ceil().max(1.0) as u64;
    OutputSpec {
        w, h, fps, start: base.start, end: base.end, duration: base.duration,
        frames, delay_ms: (1000.0 / fps).round().clamp(10.0, 250.0) as u32,
    }
}

/// 生成最终导出计划；GIF 设定了体积上限时自动降级到预算内。
pub fn plan(s: &VideoSettings, info: &VideoInfo, m: Option<&Measured>) -> ExportPlan {
    let base = resolve_spec(s, info);
    let fmt = OutFormat::from_key(&s.out_format);
    let budget = if fmt.is_gif() && s.gif_limit_mb > 0.01 {
        Some((s.gif_limit_mb as f64 * 1024.0 * 1024.0) as u64)
    } else {
        None
    };
    let Some(budget) = budget else {
        return ExportPlan { spec: base, colors: s.gif_colors, speed: crate::video::gif::speed_for(s.gif_colors), budget: None, note: String::new() };
    };

    let strategy = LimitStrategy::from_key(&s.limit_strategy);
    let mut fps = base.fps;
    let mut w = base.w;
    let mut h = base.h;
    let mut colors = s.gif_colors;
    let mut dither = s.gif_dither;
    let mut spec = scaled_spec(&base, fps, w, h);
    let mut notes: Vec<String> = Vec::new();

    for step in 0..26 {
        let est = estimate_gif(&spec, colors, dither, m);
        if est.bytes as f64 <= budget as f64 * 0.92 {
            break;
        }
        let can_fps = fps > 5.0;
        let can_size = w > 200 && h > 140;
        let can_colors = colors > 32;
        if !can_fps && !can_size && !can_colors {
            if dither {
                dither = false;
                notes.push("关闭抖动".into());
                continue;
            }
            break;
        }
        let shrink_fps = match strategy {
            LimitStrategy::FramesFirst => can_fps,
            LimitStrategy::ResolutionFirst => !can_size && can_fps,
            LimitStrategy::Balanced => (step % 2 == 0) && can_fps || !can_size,
        };
        if shrink_fps {
            let new_fps = (fps * 0.82).max(5.0);
            if (new_fps - fps).abs() < 0.01 { break; }
            if notes.is_empty() { notes.push(format!("{:.0} fps", fps)); }
            fps = new_fps;
        } else if can_size {
            if notes.len() == 1 { notes.push(format!("{}×{}", w, h)); }
            let k: f64 = 0.88;
            w = ((w as f64 * k) as u32).max(160) & !1;
            h = ((h as f64 * k) as u32).max(120) & !1;
        } else if can_colors {
            colors = if colors > 128 { 128 } else if colors > 64 { 64 } else { 32 };
            notes.push(format!("{colors} 色"));
        } else if dither {
            dither = false;
            notes.push("关闭抖动".into());
        }
        spec = scaled_spec(&base, fps, w, h);
    }

    let mut note = String::new();
    if spec.fps < base.fps - 0.01 {
        note.push_str(&format!("帧率 {:.0}→{:.0} fps", base.fps, spec.fps));
    }
    if spec.w != base.w || spec.h != base.h {
        if !note.is_empty() { note.push('，'); }
        note.push_str(&format!("尺寸 {}×{}→{}×{}", base.w, base.h, spec.w, spec.h));
    }
    if colors != s.gif_colors || dither != s.gif_dither {
        if !note.is_empty() { note.push('，'); }
        note.push_str(&format!("{s_colors}→{colors} 色", s_colors = s.gif_colors));
    }
    if !note.is_empty() {
        note = format!("为控制体积已自动调整：{note}");
    }
    let _ = notes;

    ExportPlan { spec, colors, speed: crate::video::gif::speed_for(colors), budget: Some(budget), note }
}
