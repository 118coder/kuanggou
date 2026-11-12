//! 逐帧图像处理流水线：去底 → 缩放适配 → 输出格式预处理。
//!
//! 这里是「视频模式复用图片算法」的唯一衔接点：所有算法都来自 `crate::algo`，
//! 视频侧只负责把参数换算成算法量纲，并保证整段视频使用同一个自动识别底色
//! （避免逐帧重新取样造成底色闪烁）。

use crate::algo::{
    FxModel, Img, chroma_key, erode_alpha, smooth_alpha, unmult_black, UnmultParams,
};
use crate::video::presets::fit_size;
use crate::video::types::{FitMode, KeyMode, VideoSettings};
use image::{RgbaImage, imageops};

/// 一段视频共用的帧处理器。自动模式的底色只在首帧识别一次。
pub struct FrameProcessor {
    pub s: VideoSettings,
    pub auto_bg: Option<[u8; 3]>,
    pub auto_model: FxModel,
    pub auto_tried: bool,
}

impl FrameProcessor {
    pub fn new(s: &VideoSettings) -> Self {
        Self { s: s.clone(), auto_bg: None, auto_model: FxModel::Color, auto_tried: false }
    }

    pub fn mode(&self) -> KeyMode { KeyMode::from_key(&self.s.key_mode) }

    /// 自动模式首帧取样（从边框统计主色），失败时返回 false。
    pub fn detect_auto(&mut self, frame: &Img) -> bool {
        self.auto_tried = true;
        let probe = frame.downscale_to_long(480);
        match crate::algo::detect_bg_auto(&probe) {
            Some((c, model)) => { self.auto_bg = Some(c); self.auto_model = model; true }
            None => false,
        }
    }

    pub fn reset_auto(&mut self) {
        self.auto_bg = None;
        self.auto_tried = false;
    }

    /// 当前生效的键色（供 UI 显示 / 取色）。
    pub fn active_key(&self) -> [u8; 3] {
        let m = self.mode();
        if m == KeyMode::Auto {
            return self.auto_bg.unwrap_or([0, 255, 0]);
        }
        let v = self.s.key_color;
        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
    }

    /// 原地处理一帧 RGBA。输入尺寸为 w×h。
    pub fn process(&mut self, buf: &mut [u8], w: u32, h: u32) {
        let m = self.mode();
        if m == KeyMode::Auto && !self.auto_tried {
            self.detect_auto(&Img::from_raw(w as usize, h as usize, buf.to_vec()));
        }

        if m == KeyMode::Unmult {
            let p = UnmultParams {
                black_point: self.s.un_black,
                white_point: self.s.un_white,
                density: self.s.un_density,
                noise_floor: self.s.un_floor,
                gain_limit: self.s.un_gain,
                use_luma: self.s.un_luma,
            };
            unmult_black(buf, w, h, &p);
        } else {
            let key = if m == KeyMode::Auto {
                self.auto_bg.unwrap_or([0, 255, 0])
            } else if m == KeyMode::Custom || m.needs_pick() {
                let v = self.s.key_color;
                [(v >> 16) as u8, (v >> 8) as u8, v as u8]
            } else {
                m.default_rgb()
            };
            let (mode_id, c1, c2) = match m {
                KeyMode::Green => (0, key, [0, 0, 255]),
                KeyMode::Blue => (1, key, [0, 255, 0]),
                KeyMode::White => (2, key, key),
                // 自动识别到明显绿/蓝时改用带抑色的色键，效果更干净
                KeyMode::Auto if key[1] > key[0] + 30 && key[1] > key[2] + 30 => (0, key, key),
                KeyMode::Auto if key[2] > key[0] + 30 && key[2] > key[1] + 30 => (1, key, key),
                _ => (3, key, key),
            };
            let tol = ((self.s.tol * 2.0 / 3.0) as u32).clamp(0, 100);
            let fea = ((self.s.feather * 2.0) as u32).clamp(0, 100);
            let _ = chroma_key(buf, w, h, mode_id, tol, fea, self.s.suppress as u32, c1, c2);
        }
        erode_alpha(buf, w, h, self.s.erode);
        smooth_alpha(buf, w, h, self.s.smooth);
    }
}

/// 把一帧缩放/裁切到目标规格，返回固定尺寸的 RGBA（多余区域为透明）。
pub fn fit_frame(src: &Img, dst_w: u32, dst_h: u32, fit: FitMode) -> Img {
    if src.w == 0 || src.h == 0 || dst_w == 0 || dst_h == 0 {
        return Img::from_raw(dst_w.max(1) as usize, dst_h.max(1) as usize, vec![0u8; (dst_w.max(1) * dst_h.max(1) * 4) as usize]);
    }
    let full = match RgbaImage::from_raw(src.w as u32, src.h as u32, src.buf.clone()) {
        Some(i) => i,
        None => return Img::from_raw(dst_w as usize, dst_h as usize, vec![0u8; (dst_w * dst_h * 4) as usize]),
    };
    if fit == FitMode::Stretch {
        let r = imageops::resize(&full, dst_w, dst_h, imageops::FilterType::Triangle);
        return Img::from_raw(dst_w as usize, dst_h as usize, r.into_raw());
    }
    let (tw, th) = fit_size(src.w as u32, src.h as u32, dst_w, dst_h, fit);
    let scaled = imageops::resize(&full, tw.max(2), th.max(2), imageops::FilterType::Triangle);
    let mut canvas = RgbaImage::new(dst_w, dst_h);
    let ox = (dst_w as i64 - scaled.width() as i64) / 2;
    let oy = (dst_h as i64 - scaled.height() as i64) / 2;
    imageops::overlay(&mut canvas, &scaled, ox, oy);
    Img::from_raw(dst_w as usize, dst_h as usize, canvas.into_raw())
}

/// 把 alpha 合成到不透明底色（MP4 等不支持透明的容器用）。
pub fn composite_flat(buf: &mut [u8], bg: [u8; 3]) {
    for px in buf.chunks_exact_mut(4) {
        let a = px[3] as f32 / 255.0;
        for c in 0..3 {
            px[c] = (px[c] as f32 * a + bg[c] as f32 * (1.0 - a)).min(255.0) as u8;
        }
        px[3] = 255;
    }
}

// 8×8 Bayer 有序抖动矩阵（值域 0..1），用来把半透明近似成 1-bit 透明点阵。
const BAYER8: [[f32; 8]; 8] = [
    [0.0, 32.0, 8.0, 40.0, 2.0, 34.0, 10.0, 42.0],
    [48.0, 16.0, 56.0, 24.0, 50.0, 18.0, 58.0, 26.0],
    [12.0, 44.0, 4.0, 36.0, 14.0, 46.0, 6.0, 38.0],
    [60.0, 28.0, 52.0, 20.0, 62.0, 30.0, 54.0, 22.0],
    [3.0, 35.0, 11.0, 43.0, 1.0, 33.0, 9.0, 41.0],
    [51.0, 19.0, 59.0, 27.0, 49.0, 17.0, 57.0, 25.0],
    [15.0, 47.0, 7.0, 39.0, 13.0, 45.0, 5.0, 37.0],
    [63.0, 31.0, 55.0, 23.0, 61.0, 29.0, 53.0, 21.0],
];

/// GIF 只支持 1-bit 透明（alpha=0 才透），这里把连续 alpha 二值化；
/// `dither=true` 时用 Bayer 点阵保留半透明观感。
pub fn gif_alpha_binary(buf: &mut [u8], w: u32, h: u32, threshold: u8, dither: bool) {
    let t = threshold as f32;
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            let a = buf[o + 3] as f32;
            let cut = if dither {
                (BAYER8[(y % 8) as usize][(x % 8) as usize] / 64.0) * 255.0
            } else {
                t
            };
            if a < cut.max(1.0) {
                buf[o] = 0;
                buf[o + 1] = 0;
                buf[o + 2] = 0;
                buf[o + 3] = 0;
            } else {
                buf[o + 3] = 255;
            }
        }
    }
}

/// 预量化到近似 `colors` 色（每通道级数 = ∛colors），提高 LZW 压缩率。
pub fn quantize_colors(buf: &mut [u8], colors: u16) {
    let levels = (colors as f32).cbrt().round().clamp(2.0, 8.0);
    let step = 255.0 / (levels - 1.0);
    for px in buf.chunks_exact_mut(4) {
        if px[3] == 0 { continue; }
        for c in 0..3 {
            px[c] = ((px[c] as f32 / step).round() * step).min(255.0) as u8;
        }
    }
}

/// 把 Color32 风格的 u32（0xRRGGBB）拆成 RGB。
pub fn rgb_of_u32(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}
