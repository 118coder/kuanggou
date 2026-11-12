//! Unmult 去黑底（黑底光效 / 烟雾 / 火花的反预乘抠像）。
//!
//! 参考 After Effects 插件 Unmult 的原理：黑底素材本质上是「已经预乘过
//! alpha 的前景」，`C_observed = C_true × α`。因此只要从亮度反推出 α，
//! 再把颜色除以 α，就能同时得到透明通道与原始亮度，半透明层次不丢失：
//!
//! ```text
//! α = clamp((max(R,G,B) − 黑场) / (白场 − 黑场)) ^ 浓淡
//! C_true = min(255, C_observed / α)      // 增益受「颜色补偿」上限约束
//! ```
//!
//! 与 `fx_matte` 的「黑屏光效」模型相比，本实现不依赖边框底色场，逐像素
//! 独立运算，所以对整帧视频、边缘压黑、渐变暗角的素材更稳，也更接近
//! Unmult 的一键直出效果。适合黑底光效素材；实拍黑底人像会变半透明，
//! 应改用色键或双底合成。

/// Unmult 参数（全部为 0–255 的显示量纲，便于直接映射到界面滑杆）。
#[derive(Clone, Copy, Debug)]
pub struct UnmultParams {
    /// 黑场：亮度低于该值一律当作纯黑背景（0–64）。
    pub black_point: f32,
    /// 白场：亮度达到该值即完全不透明（128–255）。
    pub white_point: f32,
    /// 浓淡：<1 提升弱光使其更实，>1 压淡（0.2–3.0）。
    pub density: f32,
    /// 清底：α 低于该值（0–255 量纲）直接归零，抹掉暗部噪声（0–32）。
    pub noise_floor: f32,
    /// 颜色补偿上限：反预乘时的最大增益，防止极暗像素被放大成彩色噪点（1–16）。
    pub gain_limit: f32,
    /// 用亮度（Rec.709）而不是最大通道来求 α；发光偏色素材可试。
    pub use_luma: bool,
}

impl Default for UnmultParams {
    fn default() -> Self {
        Self {
            black_point: 0.0,
            white_point: 255.0,
            density: 1.0,
            noise_floor: 0.0,
            gain_limit: 12.0,
            use_luma: false,
        }
    }
}

#[inline]
fn clamp01(x: f32) -> f32 {
    if x < 0.0 { 0.0 } else if x > 1.0 { 1.0 } else { x }
}

/// 原地对 RGBA8 缓冲区做 Unmult 去黑底。
///
/// 输入缓冲按 RGBA8 / 行优先解读；输出的 RGB 为「反预乘后的真实颜色」，
/// A 为由亮度反解的覆盖度。缓冲区长度不足时安全返回。
pub fn unmult_black(buf: &mut [u8], w: u32, h: u32, p: &UnmultParams) {
    let (wu, hu) = (w as usize, h as usize);
    if wu == 0 || hu == 0 || buf.len() < wu * hu * 4 {
        return;
    }
    let bp = p.black_point.clamp(0.0, 200.0);
    let wp = p.white_point.clamp(bp + 1.0, 255.0);
    let span = wp - bp;
    let density = p.density.clamp(0.05, 8.0);
    let floor = p.noise_floor.clamp(0.0, 128.0);
    let gain_cap = p.gain_limit.max(1.0);
    let use_luma = p.use_luma;
    let unit = (density - 1.0).abs() < 1e-4;

    for px in buf.chunks_exact_mut(4) {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let lum = if use_luma {
            0.2126 * r + 0.7152 * g + 0.0722 * b
        } else {
            r.max(g).max(b)
        };
        let mut a = clamp01((lum - bp) / span);
        if !unit {
            a = a.powf(density);
        }
        if a * 255.0 <= floor {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            px[3] = 0;
            continue;
        }
        // 反预乘：暗像素除以小 α 会放大噪声，用增益上限兜住。
        let gain = (1.0 / a.max(1.0 / 255.0)).min(gain_cap);
        px[0] = (r * gain).min(255.0) as u8;
        px[1] = (g * gain).min(255.0) as u8;
        px[2] = (b * gain).min(255.0) as u8;
        px[3] = (a * 255.0 + 0.5) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_black_becomes_fully_transparent() {
        let mut buf = vec![0, 0, 0, 255, 255, 255, 255, 255];
        unmult_black(&mut buf, 2, 1, &UnmultParams::default());
        assert_eq!(&buf[0..4], &[0, 0, 0, 0]);
        assert_eq!(&buf[4..8], &[255, 255, 255, 255]);
    }

    #[test]
    fn half_transparent_white_unpremultiplies_to_white() {
        let mut buf = vec![128, 128, 128, 255];
        unmult_black(&mut buf, 1, 1, &UnmultParams::default());
        assert!((buf[3] as i32 - 128).abs() <= 2, "alpha={}", buf[3]);
        assert!(buf[0] >= 250 && buf[1] >= 250 && buf[2] >= 250, "rgb={:?}", &buf[0..3]);
    }

    #[test]
    fn colored_glow_keeps_hue_after_unpremultiply() {
        let mut buf = vec![120, 20, 0, 255];
        unmult_black(&mut buf, 1, 1, &UnmultParams::default());
        assert!(buf[0] >= 240, "r={}", buf[0]);
        assert!(buf[1] < 60, "g={}", buf[1]);
        assert!(buf[2] < 20, "b={}", buf[2]);
        assert!((buf[3] as i32 - 120).abs() <= 3, "alpha={}", buf[3]);
    }

    #[test]
    fn black_point_and_noise_floor_clean_dark_pixels() {
        let p = UnmultParams { black_point: 20.0, noise_floor: 8.0, ..Default::default() };
        let mut buf = vec![10, 10, 10, 255, 200, 200, 200, 255];
        unmult_black(&mut buf, 2, 1, &p);
        assert_eq!(&buf[0..4], &[0, 0, 0, 0]);
        assert!(buf[7] > 150, "alpha={}", buf[7]);
    }

    #[test]
    fn density_changes_opacity_monotonically() {
        let mut faint = vec![64, 64, 64, 255];
        let mut dense = vec![64, 64, 64, 255];
        unmult_black(&mut faint, 1, 1, &UnmultParams { density: 2.0, ..Default::default() });
        unmult_black(&mut dense, 1, 1, &UnmultParams { density: 0.5, ..Default::default() });
        assert!(dense[3] > faint[3], "dense={} faint={}", dense[3], faint[3]);
    }
}
