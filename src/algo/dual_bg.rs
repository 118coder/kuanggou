//! 双背景差分：同主体黑底图 + 白底图逐像素反解 alpha。

#![allow(unused_imports)]
use crate::algo::Img;
use crate::algo::chroma::*;
use crate::algo::fx::*;
use crate::algo::bg_field::*;
use crate::algo::fx_matte::*;
use crate::algo::unmult::*;


// ---------------------------------------------------------------------------
// 双背景差分抠图（对应 ff_double_bg）：同一主体的黑底图 + 白底图 → 逐像素求解 alpha
// diff = ((rw-rb)+(gw-gb)+(bw-bb))/3；alpha = clamp(255 - diff*tolScale)
// 之后 gamma 曲线调边缘、阈值清底、去小连通域、可选逐层削边
// 结果写入 black 缓冲区（与原实现一致）
// tolerance/edge_contrast/erode 均为 0..=100
// ---------------------------------------------------------------------------

pub fn double_bg(
    black: &mut [u8], white: &[u8], w: u32, h: u32,
    tolerance: u32, edge_contrast: u32, erode: u32,
) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("图像尺寸为 0".into());
    }
    let n = (w as usize) * (h as usize) * 4;
    if black.len() < n || white.len() < n {
        return Err("缓冲区长度与图像尺寸不符".into());
    }
    let tol_scale = 0.5 + tolerance as f32 / 100.0;
    let gamma = 0.5 + edge_contrast as f32 / 100.0;

    // 1) 逐像素求 alpha 并 un-premultiply 恢复前景色
    for i in 0..(w * h) as usize {
        let o = i * 4;
        let (rb, gb, bb) = (black[o] as f32, black[o + 1] as f32, black[o + 2] as f32);
        let (rw, gw, bw) = (white[o] as f32, white[o + 1] as f32, white[o + 2] as f32);
        let diff = ((rw - rb) + (gw - gb) + (bw - bb)) / 3.0;
        let mut alpha = (255.0 - diff * tol_scale).clamp(0.0, 255.0);
        alpha = 255.0 * (alpha / 255.0).powf(gamma);
        let a = alpha.round() as u32;
        if a > 0 {
            black[o] = ((rb * 255.0) / a as f32).round().clamp(0.0, 255.0) as u8;
            black[o + 1] = ((gb * 255.0) / a as f32).round().clamp(0.0, 255.0) as u8;
            black[o + 2] = ((bb * 255.0) / a as f32).round().clamp(0.0, 255.0) as u8;
            black[o + 3] = a as u8;
        } else {
            black[o] = 0; black[o + 1] = 0; black[o + 2] = 0; black[o + 3] = 0;
        }
    }

    // 2) 低 alpha 置零
    for px in black.chunks_exact_mut(4) {
        if px[3] < 51 { px[3] = 0; }
    }

    // 3) 去小连通域（8 连通，min area 16）
    let (wu, hu) = (w as usize, h as usize);
    let total = wu * hu;
    let mut label = vec![0u32; total];
    let mut areas: Vec<u32> = vec![0];
    for i in 0..total {
        if black[i * 4 + 3] == 0 || label[i] != 0 { continue; }
        let id = areas.len() as u32;
        let mut stack = vec![i];
        label[i] = id;
        let mut area = 0u32;
        while let Some(j) = stack.pop() {
            area += 1;
            let x = j % wu;
            let y = j / wu;
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    if dx == 0 && dy == 0 { continue; }
                    let nx = x as isize + dx;
                    let ny = y as isize + dy;
                    if nx < 0 || ny < 0 || nx >= wu as isize || ny >= hu as isize { continue; }
                    let k = ny as usize * wu + nx as usize;
                    if black[k * 4 + 3] != 0 && label[k] == 0 {
                        label[k] = id;
                        stack.push(k);
                    }
                }
            }
        }
        areas.push(area);
    }
    for i in 0..total {
        if label[i] != 0 && areas[label[i] as usize] < 16 {
            black[i * 4 + 3] = 0;
        }
    }

    // 4) 边缘逐层削边（frontier shave 22/层）
    let passes = (erode as usize * 56) / 100;
    for _ in 0..passes {
        let alpha_snap: Vec<u8> = (0..total).map(|i| black[i * 4 + 3]).collect();
        let mut changed = false;
        for y in 0..hu {
            for x in 0..wu {
                let i = y * wu + x;
                let a = alpha_snap[i];
                if a == 0 { continue; }
                let mut frontier = x == 0 || y == 0 || x + 1 == wu || y + 1 == hu;
                if !frontier {
                    if (x > 0 && alpha_snap[i - 1] <= 10)
                        || (x + 1 < wu && alpha_snap[i + 1] <= 10)
                        || (y > 0 && alpha_snap[i - wu] <= 10)
                        || (y + 1 < hu && alpha_snap[i + wu] <= 10)
                    {
                        frontier = true;
                    }
                }
                if frontier {
                    let na = a.saturating_sub(22);
                    black[i * 4 + 3] = na;
                    // RGB 同比例收缩避免光晕
                    if na == 0 {
                        black[i * 4] = 0; black[i * 4 + 1] = 0; black[i * 4 + 2] = 0;
                    }
                    changed = true;
                }
            }
        }
        if !changed { break; }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 后处理：alpha 侵蚀 / 边缘平滑（色键模式通用）
// radius_px：像素半径 0..=10（0 = 不处理）
// 侵蚀 = 最小值滤波（收缩不透明区，去绿边/白边），平滑 = 盒式模糊（柔化锯齿）
// 只作用于 alpha 通道，RGB 保持不变
// ---------------------------------------------------------------------------
