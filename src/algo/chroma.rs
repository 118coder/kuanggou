//! 色度键控：绿幕 / 蓝幕 / 白底泛洪 / 自定义色 / 双色键 / 左上角连通域，
//! 以及 alpha 腐蚀与平滑两种后处理。

#![allow(unused_imports)]
use crate::algo::Img;
use crate::algo::dual_bg::*;
use crate::algo::fx::*;
use crate::algo::bg_field::*;
use crate::algo::fx_matte::*;
use crate::algo::unmult::*;


#[inline]
pub(crate) fn dist(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
    let dr = r1 as f32 - r2 as f32;
    let dg = g1 as f32 - g2 as f32;
    let db = b1 as f32 - b2 as f32;
    (dr * dr + dg * dg + db * db).sqrt()
}

// ---------------------------------------------------------------------------
// 色键去背（对应 ff_chroma_key）
// mode: 0=绿幕 1=蓝幕 2=白底(边缘泛洪) 3=自定义色1 4=双重色键(c1+c2)
//       5=左上角连通域（AI 生成图专用）
// tol/feather/suppress: 0..=100
// ---------------------------------------------------------------------------

pub fn chroma_key(
    buf: &mut [u8], w: u32, h: u32,
    mode: u32, tol: u32, feather: u32, suppress: u32,
    c1: [u8; 3], c2: [u8; 3],
) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("图像尺寸为 0".into());
    }
    if buf.len() < (w as usize) * (h as usize) * 4 {
        return Err("缓冲区长度与图像尺寸不符".into());
    }
    let t = (tol as f32 / 100.0) * 180.0 + 8.0;          // 主容差
    let f = (feather as f32 / 100.0) * 120.0 + 1.0;       // 羽化带宽
    let sup = suppress as f32 / 100.0;

    match mode {
        0 | 1 => {
            // 绿/蓝幕：全局色度距离 + 抑色。键色由调用方传入（默认纯绿/纯蓝，
            // 底色不纯时可用取色器给实际颜色），抑色通道取键色里最亮的通道。
            let (kr, kg, kb) = (c1[0], c1[1], c1[2]);
            let dom = if kr >= kg && kr >= kb {
                0
            } else if kg >= kb {
                1
            } else {
                2
            };
            for px in buf.chunks_exact_mut(4) {
                let (r, g, b, _) = (px[0], px[1], px[2], px[3]);
                let d = dist(r, g, b, kr, kg, kb);
                let a = ((d - t) / f).clamp(0.0, 1.0);
                // 抑色：不论是否半透明都压（去绿边=0 时不动）
                let c = [r, g, b];
                let (o1, o2) = match dom {
                    0 => (1, 2),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                let m = c[o1].max(c[o2]) as f32; // 非主导通道上限
                if c[dom] as f32 > m && sup > 0.0 {
                    let spill = ((c[dom] as f32 - m) * sup) as u8;
                    px[dom] = c[dom].saturating_sub(spill);
                }
                px[3] = (a * 255.0) as u8;
            }
        }
        2 => {
            // 白底：从四边泛洪，只抠外部连通的白色区域
            let (wu, hu) = (w as usize, h as usize);
            let mut removed = vec![false; wu * hu];
            let mut stack: Vec<usize> = Vec::with_capacity(1024);
            for x in 0..wu {
                stack.push(x);
                stack.push((hu - 1) * wu + x);
            }
            for y in 0..hu {
                stack.push(y * wu);
                stack.push(y * wu + wu - 1);
            }
            while let Some(i) = stack.pop() {
                if removed[i] { continue; }
                let o = i * 4;
                if dist(buf[o], buf[o + 1], buf[o + 2], 255, 255, 255) > t { continue; }
                removed[i] = true;
                let x = i % wu;
                if x > 0 { stack.push(i - 1); }
                if x + 1 < wu { stack.push(i + 1); }
                if i >= wu { stack.push(i - wu); }
                if i + wu < wu * hu { stack.push(i + wu); }
            }
            for (i, rm) in removed.iter().enumerate() {
                let o = i * 4;
                if *rm {
                    buf[o + 3] = 0;
                } else {
                    // 边缘羽化：靠近白色按比例降 alpha
                    let d = dist(buf[o], buf[o + 1], buf[o + 2], 255, 255, 255);
                    if d < t + f {
                        let a = ((d - t) / f).clamp(0.0, 1.0);
                        buf[o + 3] = (a * 255.0) as u8;
                    }
                }
            }
            let _ = sup;
        }
        3 | 4 => {
            let keys = if mode == 3 {
                vec![c1]
            } else {
                vec![c1, c2]
            };
            for px in buf.chunks_exact_mut(4) {
                let (r, g, b) = (px[0], px[1], px[2]);
                let mut d = f32::MAX;
                for k in &keys {
                    d = d.min(dist(r, g, b, k[0], k[1], k[2]));
                }
                let a = ((d - t) / f).clamp(0.0, 1.0);
                if a < 1.0 { px[3] = (a * 255.0) as u8; }
            }
        }
        5 => {
            // 左上角连通域：以左上角像素为键色从该点泛洪（AI 生成图的纯色背景）
            let (wu, hu) = (w as usize, h as usize);
            let key = (buf[0], buf[1], buf[2]);
            let mut removed = vec![false; wu * hu];
            let mut stack: Vec<usize> = vec![0];
            while let Some(i) = stack.pop() {
                if removed[i] { continue; }
                let o = i * 4;
                if dist(buf[o], buf[o + 1], buf[o + 2], key.0, key.1, key.2) > t { continue; }
                removed[i] = true;
                let x = i % wu;
                if x > 0 { stack.push(i - 1); }
                if x + 1 < wu { stack.push(i + 1); }
                if i >= wu { stack.push(i - wu); }
                if i + wu < wu * hu { stack.push(i + wu); }
            }
            for (i, rm) in removed.iter().enumerate() {
                let o = i * 4;
                if *rm {
                    buf[o + 3] = 0;
                } else {
                    // 羽化仅作用于与被移除区域相邻的像素：内部同色区（不连通）必须保留
                    let x = i % wu;
                    let y = i / wu;
                    let near = (x > 0 && removed[i - 1])
                        || (x + 1 < wu && removed[i + 1])
                        || (y > 0 && removed[i - wu])
                        || (y + 1 < hu && removed[i + wu]);
                    if near {
                        let d = dist(buf[o], buf[o + 1], buf[o + 2], key.0, key.1, key.2);
                        if d < t + f {
                            let a = ((d - t) / f).clamp(0.0, 1.0);
                            buf[o + 3] = (a * 255.0) as u8;
                        }
                    }
                }
            }
            let _ = sup;
        }
        _ => return Err(format!("未知模式: {mode}")),
    }
    Ok(())
}

pub fn erode_alpha(buf: &mut [u8], w: u32, h: u32, radius_px: f32) {
    let r = radius_px.round().clamp(0.0, 10.0) as usize;
    if r == 0 || w == 0 || h == 0 { return; }
    let (wu, hu) = (w as usize, h as usize);
    if buf.len() < wu * hu * 4 { return; }
    let snap: Vec<u8> = (0..wu * hu).map(|i| buf[i * 4 + 3]).collect();
    for y in 0..hu {
        for x in 0..wu {
            let mut m = 255u8;
            for dy in -(r as isize)..=(r as isize) {
                let yy = y as isize + dy;
                if yy < 0 || yy >= hu as isize { continue; }
                for dx in -(r as isize)..=(r as isize) {
                    let xx = x as isize + dx;
                    if xx < 0 || xx >= wu as isize { continue; }
                    m = m.min(snap[yy as usize * wu + xx as usize]);
                }
            }
            buf[(y * wu + x) * 4 + 3] = m;
        }
    }
}

pub fn smooth_alpha(buf: &mut [u8], w: u32, h: u32, radius_px: f32) {
    let r = radius_px.round().clamp(0.0, 10.0) as usize;
    if r == 0 || w == 0 || h == 0 { return; }
    let (wu, hu) = (w as usize, h as usize);
    if buf.len() < wu * hu * 4 { return; }
    let snap: Vec<u8> = (0..wu * hu).map(|i| buf[i * 4 + 3]).collect();
    // 水平盒式模糊 → 垂直盒式模糊（可分离，窗口 = 自身 ± r）
    let mut tmp = vec![0u8; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            let x0 = x.saturating_sub(r);
            let x1 = (x + r + 1).min(wu);
            let cnt = (x1 - x0) as u32;
            let sum: u32 = (x0..x1).map(|xx| snap[y * wu + xx] as u32).sum();
            tmp[y * wu + x] = (sum / cnt) as u8;
        }
    }
    for y in 0..hu {
        for x in 0..wu {
            let y0 = y.saturating_sub(r);
            let y1 = (y + r + 1).min(hu);
            let cnt = (y1 - y0) as u32;
            let sum: u32 = (y0..y1).map(|yy| tmp[yy * wu + x] as u32).sum();
            buf[(y * wu + x) * 4 + 3] = (sum / cnt) as u8;
        }
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// 半透明特效抠图（忠实移植 FrameRonin ImageMatte 线上版 wt 函数）
// 模型：彩色光效 / 烟雾／薄纱（绿蓝幕中性烟）/ 黑屏光效（黑底专用）
// alpha：逐通道相对底色的偏离度取最大；烟雾模式叠加「主通道盈余比」
//   （底色为白/灰等无主导通道时 e<0.05，自动退化为光效估算——与原站一致）
// 颜色分离：F = (C - B*(1-A)) / A（un-premultiply）
// 参数（与原站同量纲）：color_tolerance 0..20 / black_point 0..20 /
//   white_point 50..100 / density 0.2..3 / despill 0..100
// ---------------------------------------------------------------------------
