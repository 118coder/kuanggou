//! 算法单元测试：色键、白底泛洪、连通域、双背景、特效反解、底色场。
//!
//! 测试用例沿用单文件版，断言语义不变。

#![allow(unused_imports)]
use crate::algo::*;
use crate::algo::chroma::*;
use crate::algo::dual_bg::*;
use crate::algo::fx::*;
use crate::algo::fx_matte::*;
use crate::algo::bg_field::*;
use crate::algo::unmult::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// 简单构造纯色图
    pub(crate) fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h { v.extend_from_slice(&rgba); }
        v
    }

    pub(crate) fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * w + x) * 4) as usize;
        [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
    }

    #[test]
    pub(crate) fn green_screen_removes_green_and_suppresses_spill() {
        // 2 像素：纯绿（应全透明且 g 被抑色压回）、纯红（应完整保留）
        let mut buf = vec![
            0, 255, 0, 255,   // 纯绿
            255, 0, 0, 255,   // 纯红
        ];
        chroma_key(&mut buf, 2, 1, 0, 35, 25, 60, [0, 255, 0], [0, 0, 0]).unwrap();
        let g = px(&buf, 2, 0, 0);
        assert_eq!(g[3], 0, "纯绿应全透明");
        // 抑色：g=(255-0)*0.6=153 → 255-153=102
        assert_eq!(g[1], 102, "溢出绿应被抑色压回 max(r,b)");
        let r = px(&buf, 2, 1, 0);
        assert_eq!(r, [255, 0, 0, 255], "纯红应完整保留");
    }

    #[test]
    pub(crate) fn blue_screen_removes_blue() {
        let mut buf = vec![
            0, 0, 255, 255,
            255, 0, 0, 255,
        ];
        chroma_key(&mut buf, 2, 1, 1, 35, 25, 60, [0, 0, 255], [0, 0, 0]).unwrap();
        assert_eq!(px(&buf, 2, 0, 0)[3], 0, "纯蓝应全透明");
        assert_eq!(px(&buf, 2, 1, 0), [255, 0, 0, 255]);
    }

    #[test]
    pub(crate) fn white_background_flood_from_edges() {
        // 3x3：全白，中心红点。边缘泛洪应只抠掉与边相连的白色。
        let mut buf = solid(3, 3, [255, 255, 255, 255]);
        let o = (1 * 3 + 1) * 4;
        buf[o] = 200; buf[o + 1] = 30; buf[o + 2] = 30;
        chroma_key(&mut buf, 3, 3, 2, 35, 25, 60, [0, 0, 0], [0, 0, 0]).unwrap();
        for (i, p) in buf.chunks_exact(4).enumerate() {
            if i == 4 {
                assert_eq!(p[3], 255, "中心红色应保留");
            } else {
                assert_eq!(p[3], 0, "与边缘连通的白色应抠掉");
            }
        }
    }

    #[test]
    pub(crate) fn corner_flood_keeps_isolated_same_color() {
        // 3x3：左上角键色 (10,20,30)，中心同色但不连通 → 必须保留（羽化不得误伤内部）
        let mut buf = solid(3, 3, [200, 200, 200, 255]);
        let o = (0 * 3 + 0) * 4;
        buf[o] = 10; buf[o + 1] = 20; buf[o + 2] = 30;
        let c = (1 * 3 + 1) * 4;
        buf[c] = 10; buf[c + 1] = 20; buf[c + 2] = 30;
        // 连通域模式经典参数：容差 80 / 羽化 5
        chroma_key(&mut buf, 3, 3, 5, 80, 5, 60, [0, 0, 0], [0, 0, 0]).unwrap();
        assert_eq!(px(&buf, 3, 0, 0)[3], 0, "左上角连通域应抠掉");
        assert_eq!(px(&buf, 3, 1, 1)[3], 255, "不连通的同色内部像素必须保留");
        assert_eq!(px(&buf, 3, 2, 2)[3], 255);
    }

    #[test]
    pub(crate) fn dual_color_key_uses_nearest_of_two_keys() {
        // 两把键色：绿与白。绿、白像素都应透明，蓝保留。
        let mut buf = vec![
            0, 255, 0, 255,
            255, 255, 255, 255,
            0, 0, 255, 255,
        ];
        chroma_key(&mut buf, 3, 1, 4, 10, 1, 60, [0, 255, 0], [255, 255, 255]).unwrap();
        assert_eq!(px(&buf, 3, 0, 0)[3], 0);
        assert_eq!(px(&buf, 3, 1, 0)[3], 0);
        assert_eq!(px(&buf, 3, 2, 0)[3], 255);
    }

    #[test]
    pub(crate) fn dual_bg_exact_alpha_and_color_recovery() {
        // 与原 README 已验证用例对齐：黑/白底合成对 → alpha 精确求解 + 前景色精确恢复
        // 6x6：上半 3 行不透明前景 (180,60,40)（黑底白底拍摄结果相同，面积 18 ≥ 16 不会被去碎片清除）
        //       下半 3 行背景（黑底拍出 0,0,0，白底拍出 255,255,255）
        let n = 6 * 6 * 4;
        let mut black = vec![0u8; n];
        let mut white = vec![0u8; n];
        for y in 0..6 {
            for x in 0..6 {
                let o = (y * 6 + x) * 4;
                if y < 3 {
                    black[o] = 180; black[o + 1] = 60; black[o + 2] = 40; black[o + 3] = 255;
                    white[o] = 180; white[o + 1] = 60; white[o + 2] = 40; white[o + 3] = 255;
                } else {
                    white[o] = 255; white[o + 1] = 255; white[o + 2] = 255; white[o + 3] = 255;
                }
            }
        }
        // tolerance=100 → tol_scale=1.5 → 背景 diff=255 → alpha=0
        double_bg(&mut black, &white, 6, 6, 100, 0, 0).unwrap();
        let fg = px(&black, 6, 1, 1);
        assert_eq!([fg[0], fg[1], fg[2]], [180, 60, 40], "前景色应逐通道精确恢复");
        assert_eq!(fg[3], 255, "前景 alpha 应为 255");
        assert_eq!(px(&black, 6, 4, 4)[3], 0, "背景 alpha 应为 0");
    }

    #[test]
    pub(crate) fn dual_bg_removes_tiny_fragments() {
        // 8x8 全不透明（面积 64 ≥ 16 保留）+ 单独 1 像素碎片 → 碎片应被清除
        let mut black = vec![0u8; 8 * 8 * 4];
        let mut white = vec![0u8; 8 * 8 * 4];
        // 全图设为不透明同色前景：两底同色 → diff=0 → alpha=255
        for o in (0..8 * 8).map(|i| i * 4) {
            black[o] = 180; black[o + 1] = 60; black[o + 2] = 40; black[o + 3] = 255;
            white[o] = 180; white[o + 1] = 60; white[o + 2] = 40; white[o + 3] = 255;
        }
        double_bg(&mut black, &white, 8, 8, 100, 0, 0).unwrap();
        // 全图是一个大连通域，无人被清；再做一个碎片：右上角改为背景
        // （黑底该像素拍出黑、白底拍出白 → alpha 0），周围留下孤立透明点旁边的 2 像素连通域
        // 直接验证：把 (7,0) 变成半透明 64 → alpha 低仍 >51，需另造 <16 像素连通域
        // 简化：直接造 4x4 中 2x2=4 像素的碎片场景
        let mut b2 = vec![0u8; 8 * 8 * 4];
        let mut w2 = vec![0u8; 8 * 8 * 4];
        for o in (0..8 * 8).map(|i| i * 4) {
            w2[o] = 255; w2[o + 1] = 255; w2[o + 2] = 255; w2[o + 3] = 255;
            b2[o + 3] = 255;
        }
        // 2x2 碎片在 (4..6, 4..6)：黑底同白底 → diff=0 → alpha=255（前景）
        for y in 4..6 { for x in 4..6 {
            let o = ((y * 8 + x) * 4) as usize;
            b2[o] = 180; b2[o + 1] = 60; b2[o + 2] = 40; b2[o + 3] = 255;
            w2[o] = 180; w2[o + 1] = 60; w2[o + 2] = 40; w2[o + 3] = 255;
        }}
        double_bg(&mut b2, &w2, 8, 8, 100, 0, 0).unwrap();
        assert_eq!(px(&b2, 8, 5, 5)[3], 0, "小于 16 像素的碎片应被清除");
    }

    #[test]
    pub(crate) fn dual_bg_erode_shaves_border() {
        // 5x5 全不透明，erode=25 → 14 层削边：边界削到 0，内圈被削 2 层（211），中心保留
        let mut black = solid(5, 5, [180, 60, 40, 255]);
        let white = solid(5, 5, [180, 60, 40, 255]);
        double_bg(&mut black, &white, 5, 5, 100, 0, 25).unwrap();
        assert_eq!(px(&black, 5, 0, 0)[3], 0, "角点应被削边削到透明");
        assert_eq!(px(&black, 5, 1, 1)[3], 211, "内圈被削 2 层（每层 22）");
        assert_eq!(px(&black, 5, 2, 2)[3], 255, "中心不受削边影响");
    }

    #[test]
    pub(crate) fn reject_bad_args() {
        let mut buf = vec![0u8; 4];
        assert!(chroma_key(&mut buf, 0, 0, 0, 0, 0, 0, [0; 3], [0; 3]).is_err());
        assert!(chroma_key(&mut buf, 4, 4, 0, 0, 0, 0, [0; 3], [0; 3]).is_err(), "缓冲区过小");
        let b = vec![0u8; 16];
        let mut c = vec![0u8; 16];
        assert!(double_bg(&mut c, &b, 0, 1, 0, 0, 0).is_err());
    }

    pub(crate) fn fx_params(model: FxModel, bg: FxBgType) -> FxParams {
        // 与原站 Xe 预设一致
        let black = bg == FxBgType::Black;
        FxParams {
            model,
            bg: bg.pure_color(),
            color_tolerance: if black { 0.0 } else { 3.0 },
            black_point: if black { 0.0 } else { 1.0 },
            white_point: 100.0,
            density: 1.0,
            despill: if matches!(bg, FxBgType::Blue | FxBgType::Green | FxBgType::Magenta) { 20.0 } else { 0.0 },
        }
    }

    #[test]
    pub(crate) fn fx_green_bg_pixel_fully_transparent() {
        // 纯底色像素：逐通道偏离为 0 → 全透明
        let mut buf = vec![0, 255, 0, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Color, FxBgType::Green));
        assert_eq!(buf, vec![0, 0, 0, 0]);
    }

    #[test]
    pub(crate) fn fx_smoke_neutral_gray_on_green_kept_opaque() {
        // 绿幕上的中性烟雾：烟雾模型覆盖度=1，灰色完整保留
        let mut buf = vec![128, 128, 128, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Smoke, FxBgType::Green));
        assert_eq!(buf, vec![128, 128, 128, 255]);
    }

    #[test]
    pub(crate) fn fx_smoke_greenish_mix_separates_to_white() {
        // 绿幕 75% + 前景 25% 的混合像素 → 25% 不透明白色（un-premultiply 分离）
        let mut buf = vec![64, 255, 64, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Smoke, FxBgType::Green));
        assert_eq!(buf[3], 62, "alpha = (A-r)/(1-r) ≈ 0.243");
        assert_eq!([buf[0], buf[1], buf[2]], [255, 255, 255]);
    }

    #[test]
    pub(crate) fn fx_white_bg_separates_pale_purple() {
        // 白底淡紫 (200,180,220)：分离背景混色 → 饱和紫 (68,0,136) @ ~29%
        let mut buf = vec![200, 180, 220, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Color, FxBgType::White));
        assert_eq!(buf[3], 73, "覆盖度 A = max 通道偏离 ≈ 0.294");
        assert_eq!([buf[0], buf[1], buf[2]], [68, 0, 136]);
    }

    #[test]
    pub(crate) fn fx_white_bg_pure_white_transparent() {
        let mut buf = vec![255, 255, 255, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Color, FxBgType::White));
        assert_eq!(buf, vec![0, 0, 0, 0]);
    }

    #[test]
    pub(crate) fn fx_black_glow_compensates_color() {
        // 黑屏光效：亮度=覆盖度，颜色反解补偿 (60,40,80) → (191,128,255) @ 80
        let mut buf = vec![60, 40, 80, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Black, FxBgType::Black));
        assert_eq!(buf[3], 80);
        assert_eq!([buf[0], buf[1], buf[2]], [191, 128, 255]);
    }

    #[test]
    pub(crate) fn fx_despill_trims_dominant_excess_on_edges() {
        // 取样式底色 (80,240,60) 上同绿但更暗的像素 (60,240,40)（A=1/3）：
        // unmix → (20,240,0)，溢色抑制 20%×(1-A)=13.3% → 绿通道 240→211
        let mut buf = vec![60, 240, 40, 255];
        let p = FxParams {
            model: FxModel::Color,
            bg: [80, 240, 60],
            color_tolerance: 3.0,
            black_point: 1.0,
            white_point: 100.0,
            density: 1.0,
            despill: 20.0,
        };
        fx_matte(&mut buf, 1, 1, &p);
        assert_eq!(buf[3], 83, "覆盖度 A = 1/3");
        assert_eq!([buf[0], buf[1], buf[2]], [20, 211, 0], "绿通道应被抑制 29.3");
    }

    #[test]
    pub(crate) fn fx_auto_bg_samples_green_screen() {
        // 自动取样：绿幕图（多数像素纯绿）→ 取样出接近 (0,255,0)
        let mut img = Vec::new();
        for _ in 0..100 {
            img.extend_from_slice(&[0, 255, 0, 255]);
        }
        // 中央放一个前景像素
        img[40..44].copy_from_slice(&[128, 128, 128, 255]);
        let sampled = auto_bg_color(&Img::from_raw(10, 10, img), FxBgType::Green);
        assert!(sampled.is_some());
        let [r, g, b] = sampled.unwrap();
        assert!(g > 200 && r < 60 && b < 60, "取样结果 ({r},{g},{b})");
    }

    #[test]
    pub(crate) fn erode_alpha_expands_hole() {
        // 9x9 全不透明，中心 1px 透明孔；radius=1.0，孔洞扩大为 3x3，远处不受影响
        let mut buf = solid(9, 9, [10, 20, 30, 255]);
        buf[(4 * 9 + 4) * 4 + 3] = 0;
        erode_alpha(&mut buf, 9, 9, 1.0);
        assert_eq!(px(&buf, 9, 4, 4)[3], 0);
        for dx in -1isize..=1 {
            for dy in -1isize..=1 {
                if dx == 0 && dy == 0 { continue; }
                let x = (4 + dx) as u32;
                let y = (4 + dy) as u32;
                assert_eq!(px(&buf, 9, x, y)[3], 0, "孔洞应向四周扩大");
            }
        }
        assert_eq!(px(&buf, 9, 4, 2)[3], 255, "距孔洞 2 像素处不受影响");
        assert_eq!(px(&buf, 9, 0, 0)[3], 255);
        // radius=0 不做任何事
        let mut buf2 = solid(9, 9, [10, 20, 30, 255]);
        erode_alpha(&mut buf2, 9, 9, 0.0);
        assert_eq!(px(&buf2, 9, 4, 4)[3], 255);
    }

    #[test]
    pub(crate) fn smooth_alpha_blurs_edge() {
        // 5x1 左透明右不透明，r=1 → 边界出现过渡灰阶
        let mut buf = Vec::new();
        for a in [0u8, 0, 255, 255, 255] {
            buf.extend_from_slice(&[10, 20, 30, a]);
        }
        smooth_alpha(&mut buf, 5, 1, 1.0);
        assert_eq!(px(&buf, 5, 0, 0)[3], 0);
        assert_eq!(px(&buf, 5, 1, 0)[3], 85);
        assert_eq!(px(&buf, 5, 2, 0)[3], 170);
        assert_eq!(px(&buf, 5, 3, 0)[3], 255);
        assert_eq!(px(&buf, 5, 4, 0)[3], 255);
        // RGB 不应被改动
        assert_eq!(px(&buf, 5, 1, 0)[0], 10);
    }




    // -----------------------------------------------------------------------
    // 合成真值场景（全部由代码生成，不依赖外部图片）
    //
    // 每个场景记录：输入 RGBA、逐像素期望前景覆盖率 truth、像素分类 tag
    //   tag 0 = 纯底色（alpha 必须≈0） 1 = 不透明前景（≈1）
    //       2 = 半透明（按 truth 比）  3 = 投影（必须≈0）
    // -----------------------------------------------------------------------

    struct Synth {
        w: usize,
        h: usize,
        buf: Vec<u8>,
        truth: Vec<f32>,
        tag: Vec<u8>,
        bg_type: FxBgType,
    }

    pub(crate) fn syn_px(bg: [f32; 3], fg: [u8; 3], a: f32) -> [u8; 3] {
        [
            (bg[0] * (1.0 - a) + fg[0] as f32 * a).round().clamp(0.0, 255.0) as u8,
            (bg[1] * (1.0 - a) + fg[1] as f32 * a).round().clamp(0.0, 255.0) as u8,
            (bg[2] * (1.0 - a) + fg[2] as f32 * a).round().clamp(0.0, 255.0) as u8,
        ]
    }

    impl Synth {
        fn new(w: usize, h: usize, bg_type: FxBgType) -> Self {
            Self {
                w,
                h,
                buf: vec![0u8; w * h * 4],
                truth: vec![0.0; w * h],
                tag: vec![0u8; w * h],
                bg_type,
            }
        }
        fn set(&mut self, x: usize, y: usize, c: [u8; 3], a: f32, t: u8) {
            let o = (y * self.w + x) * 4;
            self.buf[o] = c[0];
            self.buf[o + 1] = c[1];
            self.buf[o + 2] = c[2];
            self.buf[o + 3] = 255;
            self.truth[y * self.w + x] = a;
            self.tag[y * self.w + x] = t;
        }
        fn img(&self) -> Img {
            Img::from_raw(self.w, self.h, self.buf.clone())
        }
        /// 模拟自动路径：识别 → 用识别出的底色/模型跑 fx_matte
        fn run_auto(&self) -> (Vec<u8>, Option<([u8; 3], FxModel)>) {
            let det = detect_bg_auto(&self.img());
            let (bg, model) = det.unwrap_or((self.bg_type.pure_color(), FxModel::Color));
            let mut p = fx_preset_for(self.bg_type);
            p.model = model;
            p.bg = bg;
            let mut out = self.buf.clone();
            fx_matte(&mut out, self.w as u32, self.h as u32, &p);
            (out, det)
        }
        fn mean_alpha(&self, out: &[u8], tag: u8) -> f32 {
            let (mut s, mut n) = (0.0f64, 0u32);
            for i in 0..self.w * self.h {
                if self.tag[i] == tag {
                    s += out[i * 4 + 3] as f64 / 255.0;
                    n += 1;
                }
            }
            if n == 0 { 0.0 } else { (s / n as f64) as f32 }
        }
        fn mae(&self, out: &[u8]) -> f32 {
            let mut s = 0.0f64;
            for i in 0..self.w * self.h {
                s += (out[i * 4 + 3] as f32 / 255.0 - self.truth[i]).abs() as f64;
            }
            (s / (self.w * self.h) as f64) as f32
        }
        fn mae_of(&self, out: &[u8], tag: u8) -> f32 {
            let (mut s, mut n) = (0.0f64, 0u32);
            for i in 0..self.w * self.h {
                if self.tag[i] == tag {
                    s += (out[i * 4 + 3] as f32 / 255.0 - self.truth[i]).abs() as f64;
                    n += 1;
                }
            }
            if n == 0 { 0.0 } else { (s / n as f64) as f32 }
        }
        /// 半透明区里被削成硬边（≈0 或 ≈255）的比例
        fn hard_ratio(&self, out: &[u8]) -> f32 {
            let (mut s, mut n) = (0.0f64, 0u32);
            for i in 0..self.w * self.h {
                if self.tag[i] == 2 {
                    let a = out[i * 4 + 3] as f32 / 255.0;
                    if a < 0.02 || a > 0.98 {
                        s += 1.0;
                    }
                    n += 1;
                }
            }
            if n == 0 { 0.0 } else { (s / n as f64) as f32 }
        }
    }

    /// A：绿底 #00B140 + 白烟 0.3/0.6 + 橙圆 + 黑环
    pub(crate) fn synth_green_smoke() -> Synth {
        let (w, h) = (320usize, 240usize);
        let bg = [0.0f32, 177.0, 64.0];
        let mut s = Synth::new(w, h, FxBgType::Green);
        for y in 0..h {
            for x in 0..w {
                let (mut a, mut fg, mut t) = (0.0f32, [255u8, 255, 255], 0u8);
                if x >= 20 && x < 140 && y >= 150 && y < 230 {
                    a = 0.3;
                    t = 2;
                }
                if x >= 170 && x < 305 && y >= 140 && y < 225 {
                    a = 0.6;
                    t = 2;
                }
                let (dx, dy) = (x as f32 - 100.0, y as f32 - 110.0);
                if dx * dx + dy * dy <= 45.0 * 45.0 {
                    a = 1.0;
                    fg = [240, 140, 30];
                    t = 1;
                }
                let (dx, dy) = (x as f32 - 230.0, y as f32 - 90.0);
                let r2 = dx * dx + dy * dy;
                if (1225.0..=2500.0).contains(&r2) {
                    a = 1.0;
                    fg = [22, 22, 22];
                    t = 1;
                }
                s.set(x, y, syn_px(bg, fg, a), a, t);
            }
        }
        s
    }

    /// B：四角渐变绿底 + 白烟
    pub(crate) fn synth_gradient() -> Synth {
        let (w, h) = (320usize, 240usize);
        let (tl, tr) = ([0.0f32, 120.0, 32.0], [30.0f32, 190.0, 70.0]);
        let (bl, br) = ([20.0f32, 150.0, 50.0], [80.0f32, 220.0, 120.0]);
        let mut s = Synth::new(w, h, FxBgType::Green);
        for y in 0..h {
            for x in 0..w {
                let tx = x as f32 / (w - 1) as f32;
                let ty = y as f32 / (h - 1) as f32;
                let mut bg = [0f32; 3];
                for v in 0..3 {
                    let top = tl[v] + (tr[v] - tl[v]) * tx;
                    let bot = bl[v] + (br[v] - bl[v]) * tx;
                    bg[v] = top + (bot - top) * ty;
                }
                let (mut a, mut fg, mut t) = (0.0f32, [255u8, 255, 255], 0u8);
                if x < 150 && y > 120 {
                    a = 0.5;
                    t = 2;
                }
                if x >= 180 && x < 300 && y >= 30 && y < 90 {
                    a = 0.25;
                    t = 2;
                }
                let (dx, dy) = (x as f32 - 160.0, y as f32 - 120.0);
                if dx * dx + dy * dy <= 40.0 * 40.0 {
                    a = 1.0;
                    fg = [240, 140, 30];
                    t = 1;
                }
                s.set(x, y, syn_px(bg, fg, a), a, t);
            }
        }
        s
    }

    /// C：白底 + 浅灰投影 + 蓝盒 + 中灰盒
    pub(crate) fn synth_white_shadow() -> Synth {
        let (w, h) = (320usize, 240usize);
        let mut s = Synth::new(w, h, FxBgType::White);
        for y in 0..h {
            for x in 0..w {
                let mut c = [255u8, 255, 255];
                let (mut a, mut t) = (0.0f32, 0u8);
                let dx = (x as f32 - 160.0) / 115.0;
                let dy = (y as f32 - 205.0) / 38.0;
                let d = (dx * dx + dy * dy).sqrt();
                if d < 1.0 {
                    // 等比变暗 22%：真实投影
                    let k = 1.0 - 0.22 * (1.0 - d).powf(0.7);
                    c = [(255.0 * k) as u8, (255.0 * k) as u8, (255.0 * k) as u8];
                    t = 3;
                }
                if x >= 130 && x < 190 && y >= 80 && y < 150 {
                    c = [40, 70, 160];
                    a = 1.0;
                    t = 1;
                }
                if x >= 200 && x < 240 && y >= 85 && y < 125 {
                    c = [138, 138, 138];
                    a = 1.0;
                    t = 1;
                }
                s.set(x, y, c, a, t);
            }
        }
        s
    }

    /// D：主体压边（底边全占 + 左右下段）
    pub(crate) fn synth_press_edge() -> Synth {
        let (w, h) = (320usize, 240usize);
        let bg = [0.0f32, 177.0, 64.0];
        let mut s = Synth::new(w, h, FxBgType::Green);
        for y in 0..h {
            for x in 0..w {
                let (a, mut c, t) = if y >= 120 {
                    (1.0f32, [200u8, 90, 40], 1u8)
                } else if (x < 60 || x >= 260) && y >= 60 {
                    (1.0f32, [30u8, 60, 170], 1u8)
                } else {
                    (0.0f32, [bg[0] as u8, bg[1] as u8, bg[2] as u8], 0u8)
                };
                s.set(x, y, c, a, t);
                let _ = &mut c;
            }
        }
        s
    }

    /// E：暗底 (10,10,12) + 橙色径向光效
    pub(crate) fn synth_black_glow() -> Synth {
        let (w, h) = (280usize, 200usize);
        let bg = [10.0f32, 10.0, 12.0];
        let mut s = Synth::new(w, h, FxBgType::Black);
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 - 140.0, y as f32 - 100.0);
                let r = (dx * dx + dy * dy).sqrt();
                let mut a = if r < 70.0 { (1.0 - r / 70.0).powf(1.2) * 0.95 } else { 0.0 };
                let mut fg = [255u8, 120, 40];
                if r < 26.0 {
                    a = 0.95;
                    fg = [255, 200, 60];
                }
                s.set(x, y, syn_px(bg, fg, a), a, if a > 0.0 { 2 } else { 0 });
            }
        }
        s
    }

    #[test]
    pub(crate) fn synth_green_smoke_alpha_accurate_and_solid_kept() {
        let s = synth_green_smoke();
        let (out, det) = s.run_auto();
        let (c, model) = det.expect("绿底应识别成功");
        assert!(
            (c[1] as i32 - 177).abs() <= 4 && (c[2] as i32 - 64).abs() <= 6,
            "底色识别 {:?}",
            c
        );
        assert_eq!(model, FxModel::Color);
        println!(
            "[A] MAE={:.4} 底漏={:.4} 实心={:.4} 烟MAE={:.4} 硬边率={:.4}",
            s.mae(&out),
            s.mean_alpha(&out, 0),
            s.mean_alpha(&out, 1),
            s.mae_of(&out, 2),
            s.hard_ratio(&out)
        );
        assert!(s.mae(&out) < 0.02, "整图 alpha MAE 应 < 0.02");
        assert!(s.mean_alpha(&out, 0) < 0.01, "纯底色区不得漏出 alpha");
        assert!(s.mean_alpha(&out, 1) > 0.97, "不透明前景应基本实心");
        assert!(s.mae_of(&out, 2) < 0.03, "半透明区 alpha 应贴合真值");
        assert!(s.hard_ratio(&out) < 0.02, "烟雾不能被削成硬边");
    }

    #[test]
    pub(crate) fn synth_gradient_bg_field_accurate() {
        let s = synth_gradient();
        let f = BgField::estimate(&s.buf, s.w, s.h).expect("渐变底应建场");
        // 场 vs 真值：中心与四角邻域各取点
        // 解析真值（不能从 s.buf 读：那里可能被烟雾盖住）
        let (tl, tr) = ([0.0f32, 120.0, 32.0], [30.0f32, 190.0, 70.0]);
        let (bl, br) = ([20.0f32, 150.0, 50.0], [80.0f32, 220.0, 120.0]);
        let mut max_err = 0.0f32;
        for (xi, yi) in [(0.5, 0.5), (0.1, 0.1), (0.9, 0.1), (0.1, 0.9), (0.9, 0.9), (0.5, 0.02)] {
            let x = (xi * (s.w - 1) as f32) as usize;
            let y = (yi * (s.h - 1) as f32) as usize;
            let est = f.raw_at(x, y);
            for v in 0..3 {
                let top = tl[v] + (tr[v] - tl[v]) * xi;
                let bot = bl[v] + (br[v] - bl[v]) * xi;
                let truth = top + (bot - top) * yi;
                max_err = max_err.max((est[v] * 255.0 - truth).abs());
            }
        }
        let bg_type = s.bg_type;
        let samp = auto_bg_color(&s.img(), bg_type).expect("渐变底应能取样");
        let (out, det) = s.run_auto();
        println!(
            "[B] 场最大误差={:.1} 级 · 取样=#{} 识别={:?} MAE={:.4} 底漏={:.4} 烟MAE={:.4}",
            max_err,
            samp.iter().map(|v| format!("{v:02X}")).collect::<String>(),
            det,
            s.mae(&out),
            s.mean_alpha(&out, 0),
            s.mae_of(&out, 2)
        );
        assert!(max_err < 8.0, "渐变底色场最大误差应 < 8 级，实际 {max_err}");
        assert!(s.mean_alpha(&out, 0) < 0.02, "渐变底不得漏 alpha");
        assert!(s.mae(&out) < 0.03, "渐变底整图 MAE 应 < 0.03");
        assert!(s.mae_of(&out, 2) < 0.04);
    }

    #[test]
    pub(crate) fn synth_white_shadow_suppressed() {
        let s = synth_white_shadow();
        let (out, _) = s.run_auto();
        let shadow = s.mean_alpha(&out, 3);
        let blue = s.mean_alpha(&out, 1);
        println!(
            "[C] MAE={:.4} 底漏={:.4} 影漏={:.4} 物体={:.4}",
            s.mae(&out),
            s.mean_alpha(&out, 0),
            shadow,
            blue
        );
        assert!(shadow < 0.03, "浅投影应被抑制，实际平均 alpha {shadow}");
        assert!(blue > 0.6, "白底上的蓝色物体不应被误删");
        assert!(s.mean_alpha(&out, 0) < 0.01, "白底不得漏 alpha");
    }

    #[test]
    pub(crate) fn synth_subject_on_border_does_not_pollute_bg() {
        let s = synth_press_edge();
        let (out, det) = s.run_auto();
        let (c, _) = det.expect("主体压边时仍应识别出底色");
        let err = (c[0] as i32).abs() + (c[1] as i32 - 177).abs() + (c[2] as i32 - 64).abs();
        println!("[D] 识别={:?} 三通道总误差={} 底漏={:.4}", c, err, s.mean_alpha(&out, 0));
        assert!(err <= 18, "压边主体不得污染底色，识别 {:?}", c);
        assert!(s.mean_alpha(&out, 0) < 0.02, "剩余底色区应干净");
    }

    #[test]
    pub(crate) fn synth_black_glow_no_fog() {
        let s = synth_black_glow();
        let (out, det) = s.run_auto();
        let (c, model) = det.expect("黑底应识别成功");
        assert_eq!(model, FxModel::Black, "近黑底应推荐 Black 模型");
        assert!(c[0] < 40 && c[2] < 40, "黑底识别 {:?}", c);
        println!(
            "[E] MAE={:.4} 底漏={:.4} 光效={:.4}",
            s.mae(&out),
            s.mean_alpha(&out, 0),
            s.mean_alpha(&out, 2)
        );
        assert!(s.mean_alpha(&out, 0) < 0.01, "黑底不应浮起灰雾");
        assert!(s.mae(&out) < 0.02);
    }

    #[test]
    pub(crate) fn detect_rejects_two_tone_background() {
        // 左绿右蓝的硬分界：没有单一底色可键 → 必须返回 None（提示用户手动取色）
        let (w, h) = (320usize, 240usize);
        let mut s = Synth::new(w, h, FxBgType::Green);
        for y in 0..h {
            for x in 0..w {
                let c = if x < w / 2 { [0u8, 177, 64] } else { [40, 70, 200] };
                s.set(x, y, c, 0.0, 0);
            }
        }
        let f = BgField::estimate(&s.buf, s.w, s.h);
        if let Some(f) = &f {
            println!("[双色] conf={:.2} core={:.2} edge_ok={} spread={:.2} valid={:?}", f.conf, f.core, f.edge_ok, f.spread, f.valid);
        }
        println!("[双色] detect={:?}", detect_bg_auto(&s.img()));
        // 单色模型无法表达双色底：必须拒绝（UI 会提示改用色键/手动取色），
        // 而不是硬给一个中间混色把两边各删一半。
        assert!(
            detect_bg_auto(&s.img()).is_none(),
            "双色底不应硬给一个底色"
        );
    }

    #[test]
    pub(crate) fn detect_handles_vignette_and_noise() {
        // 绿底 + 径向暗角（四角 −35 级）+ 白烟边缘
        let (w, h) = (320usize, 240usize);
        let mut s = Synth::new(w, h, FxBgType::Green);
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 / (w - 1) as f32 - 0.5;
                let dy = y as f32 / (h - 1) as f32 - 0.5;
                let k = 1.0 - 0.18 * (dx * dx + dy * dy).sqrt() * 2.0;
                let c = [0u8, (177.0 * k) as u8, (64.0 * k) as u8];
                // 底部一条 40% 白烟贴着边框（考验"被污染的边"）
                let t = if y > 220 { 2 } else { 0 };
                let cc = if t == 2 { syn_px([c[0] as f32, c[1] as f32, c[2] as f32], [255, 255, 255], 0.4) } else { c };
                s.set(x, y, cc, if t == 2 { 0.4 } else { 0.0 }, t);
            }
        }
        let (c, _) = detect_bg_auto(&s.img()).expect("暗角底应识别成功");
        let f = BgField::estimate(&s.buf, s.w, s.h).expect("暗角应建场");
        let mut out = s.buf.clone();
        let mut p = fx_preset_for(s.bg_type);
        p.bg = c;
        fx_matte(&mut out, s.w as u32, s.h as u32, &p);
        println!(
            "[暗角] 识别={:?} spread={:.3} conf={:.2} 烟MAE={:.4} 底漏={:.4}",
            c,
            f.spread,
            f.conf,
            s.mae_of(&out, 2),
            s.mean_alpha(&out, 0)
        );
        // 中位数落在边框（暗角处）而不是图像中心，属于预期；关键看场是否跟随暗角
        assert!(c[1] > 120 && c[1] < 190, "识别绿通道应在暗角范围内，识别 {:?}", c);
        assert!(f.spread > 0.02, "应检测到空间变化（暗角）");
        assert!(s.mae_of(&out, 2) < 0.08, "暗角 + 压边污染下烟雾 alpha 仍应贴合");
        assert!(s.mean_alpha(&out, 0) < 0.02, "暗角底不得漏 alpha");
    }

    #[test]
    pub(crate) fn fx_flat_background_keeps_legacy_exact_values() {
        // 常色底快路径必须与老版公式逐位一致（防止优化改动破坏既有观感）
        let bg = FxBgType::Green;
        let mut buf = vec![0u8; 64 * 64 * 4];
        for px in buf.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 255, 0, 255]);
        }
        // 中间一块半透明白烟：C = 0.25*255 + 0.75*(0,255,0)
        let c = syn_px([0.0, 255.0, 0.0], [255, 255, 255], 0.25);
        for y in 20..40 {
            for x in 20..40 {
                let o = (y * 64 + x) * 4;
                buf[o..o + 2].copy_from_slice(&[c[0], c[1]]);
                buf[o + 2] = c[2];
            }
        }
        let mut out = buf.clone();
        fx_matte(&mut out, 64, 64, &fx_preset_for(bg));
        let o = (30 * 64 + 30) * 4;
        // 相对偏离：r/b 通道 0.25，g 通道向下不偏离 → A=0.25
        // alpha = (0.25-0.01)/0.99*255 ≈ 62
        assert_eq!(out[o + 3], 62, "半透明烟雾 alpha 应与老公式一致");
        assert_eq!([out[o], out[o + 1], out[o + 2]], [255, 255, 255]);
        // 远离烟雾的底色必须全透明
        assert_eq!(out[0 + 3], 0);
    }

    #[test]
    #[ignore = "性能基准，手动 cargo test --release -- --ignored bench --nocapture"]
    pub(crate) fn bench_2mp_preview_and_detect() {
        use std::time::Instant;
        let (w, h) = (1636usize, 1224usize);
        let mut buf = vec![0u8; w * h * 4];
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.55;
        let rr = w.min(h) as f32 * 0.25;
        for y in 0..h {
            for x in 0..w {
                let tx = x as f32 / (w - 1) as f32;
                let ty = y as f32 / (h - 1) as f32;
                let tl = [0.0f32, 120.0, 32.0];
                let tr = [30.0f32, 190.0, 70.0];
                let bl = [20.0f32, 150.0, 50.0];
                let br = [80.0f32, 220.0, 120.0];
                let mut bg = [0f32; 3];
                for v in 0..3 {
                    let topv = tl[v] + (tr[v] - tl[v]) * tx;
                    let botv = bl[v] + (br[v] - bl[v]) * tx;
                    bg[v] = topv + (botv - topv) * ty;
                }
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                let a = if d < rr * 0.55 { 0.6 } else if d < rr { 0.25 } else { 0.0 };
                let c = syn_px(bg, [255, 255, 255], a);
                let o = (y * w + x) * 4;
                buf[o] = c[0];
                buf[o + 1] = c[1];
                buf[o + 2] = c[2];
                buf[o + 3] = 255;
            }
        }
        let img = Img::from_raw(w, h, buf.clone());
        let mut best_det = f64::MAX;
        for _ in 0..5 {
            let s = Instant::now();
            let _ = detect_bg_auto(&img);
            best_det = best_det.min(s.elapsed().as_secs_f64() * 1000.0);
        }
        let det = detect_bg_auto(&img).unwrap();
        let mut best_fx = f64::MAX;
        for _ in 0..4 {
            let mut b2 = buf.clone();
            let mut p = fx_preset_for(FxBgType::Green);
            p.bg = det.0;
            let s = Instant::now();
            fx_matte(&mut b2, w as u32, h as u32, &p);
            best_fx = best_fx.min(s.elapsed().as_secs_f64() * 1000.0);
        }
        let mut b3 = buf.clone();
        let mut p3 = fx_preset_for(FxBgType::Green);
        p3.bg = det.0;
        fx_matte(&mut b3, w as u32, h as u32, &p3);
        let mut leak = 0f64;
        let mut n_bg = 0u32;
        for y in 0..80 {
            for x in 0..(w / 4) {
                leak += b3[(y * w + x) * 4 + 3] as f64 / 255.0;
                n_bg += 1;
            }
        }
        println!(
            "[2MP] 识别 {best_det:.2} ms · 预览 {best_fx:.2} ms · 底色 {:?} · 角落底漏 {:.4}",
            det.0,
            leak / n_bg as f64
        );
        assert!(leak / (n_bg as f64) < 0.02, "2MP 渐变底不应漏 alpha");
        assert!(best_fx < 500.0, "预览耗时异常");
    }

    #[test]
    #[ignore = "真实样本验收：tools/test_green.png（需先 cargo test -- --ignored --nocapture）"]
    pub(crate) fn real_sample_green_screen_acceptance() {
        use std::time::Instant;
        let bytes = match std::fs::read("tools/test_green.png") {
            Ok(b) => b,
            Err(_) => return,
        };
        let img = Img::from_bytes(&bytes).expect("样本解码失败");
        let t0 = Instant::now();
        let det = detect_bg_auto(&img);
        let t_det = t0.elapsed().as_secs_f64() * 1000.0;
        let (c, model) = det.expect("样本应识别出绿底");
        let mut p = fx_preset_for(FxBgType::Green);
        p.model = model;
        p.bg = c;
        let mut out = img.buf.clone();
        let t1 = Instant::now();
        fx_matte(&mut out, img.w as u32, img.h as u32, &p);
        let t_fx = t1.elapsed().as_secs_f64() * 1000.0;
        let n = img.w * img.h;
        let (mut clear, mut semi, mut opaque) = (0u32, 0u32, 0u32);
        let mut semi_mean = 0f64;
        for i in 0..n {
            let a = out[i * 4 + 3];
            if a < 6 {
                clear += 1;
            } else if a > 250 {
                opaque += 1;
            } else {
                semi += 1;
                semi_mean += a as f64 / 255.0;
            }
        }
        let corner = |x: usize, y: usize| out[(y * img.w + x) * 4 + 3];
        println!(
            "[真实样本] {}x{} 识别=#{:02X}{:02X}{:02X} 模型={:?}
               识别 {:.2} ms · 预览 {:.2} ms
               透明 {:.1}% · 半透明 {:.1}%（均值 alpha {:.2}）· 实心 {:.1}%
               四角 alpha = {} {} {} {}",
            img.w,
            img.h,
            c[0],
            c[1],
            c[2],
            model,
            t_det,
            t_fx,
            clear as f64 * 100.0 / n as f64,
            semi as f64 * 100.0 / n as f64,
            if semi > 0 { semi_mean / semi as f64 } else { 0.0 },
            opaque as f64 * 100.0 / n as f64,
            corner(2, 2),
            corner(img.w - 3, 2),
            corner(2, img.h - 3),
            corner(img.w - 3, img.h - 3)
        );
        assert_eq!(corner(2, 2), 0, "四角应为透明");
        assert_eq!(corner(img.w - 3, img.h - 3), 0);
        assert!(
            clear as f64 / n as f64 > 0.35,
            "纯底色区应被抠掉（样本底色约占一半）"
        );
        assert!(
            semi as f64 / n as f64 > 0.20 && opaque as f64 / n as f64 > 0.05,
            "烟雾半透明与实心物体都应保留"
        );
    }


    #[test]
    pub(crate) fn chroma_accepts_custom_key_color() {
        // 背景是不纯的绿 #4F9B5C，主体橙色方块
        let (w, h) = (32usize, 32usize);
        let mut buf = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let c = if (10..22).contains(&x) && (10..22).contains(&y) {
                    [232u8, 92, 0, 255]
                } else {
                    [79, 155, 92, 255]
                };
                buf[i..i + 4].copy_from_slice(&c);
            }
        }
        // 默认纯绿键色：不纯的绿底删不掉（预期行为，提示用户取色）
        let mut a = buf.clone();
        chroma_key(&mut a, w as u32, h as u32, 0, 20, 10, 0, [0, 255, 0], [0, 0, 255]).unwrap();
        assert!(a[3] > 200, "纯绿键色不应误删不纯绿底");
        // 用实际底色取色：底色应全删，主体保留
        let mut b = buf.clone();
        chroma_key(&mut b, w as u32, h as u32, 0, 20, 10, 0, [79, 155, 92], [0, 0, 255]).unwrap();
        assert_eq!(b[3], 0, "取色后底色应删干净");
        let center = (16 * w + 16) * 4;
        assert!(b[center + 3] > 200, "主体必须保留");
    }

}
