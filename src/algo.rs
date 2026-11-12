//! 抠图去背算法 —— 从 FrameForge `wasm/src/lib.rs` 移植的 Rust 原生版本。
//!
//! 公式与原 WASM 实现逐行一致（`ff_chroma_key` / `ff_double_bg`），
//! 保证处理结果与网页版抠图模块完全相同。
//!
//! 约定：图像缓冲区一律为 RGBA8（每像素 4 字节，行优先）。

/// RGBA8 图像缓冲（含大图预览代理工具）
#[derive(Clone)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    pub buf: Vec<u8>,
}

impl Img {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| format!("图片解码失败: {e}"))?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(Self { w: w as usize, h: h as usize, buf: rgba.into_raw() })
    }

    pub fn from_raw(w: usize, h: usize, buf: Vec<u8>) -> Self {
        Self { w, h, buf }
    }

    pub fn megapixels(&self) -> f32 {
        (self.w * self.h) as f32 / 1_000_000.0
    }

    /// 大图降采样代理（长边 ≤2048，仅用于预览；导出仍走全分辨率）
    pub fn downscale_proxy(&self) -> Img {
        self.downscale_to_long(2048)
    }

    /// 按长边降采样（交互预览用；长边已小于目标则原样克隆）
    pub fn downscale_to_long(&self, long: u32) -> Img {
        let (w, h) = (self.w as u32, self.h as u32);
        if w.max(h) <= long {
            return self.clone();
        }
        let s = long as f32 / w.max(h) as f32;
        let nw = ((w as f32 * s) as u32).max(1);
        let nh = ((h as f32 * s) as u32).max(1);
        let full = image::RgbaImage::from_raw(w, h, self.buf.clone()).expect("缓冲区尺寸正确");
        let resized = image::imageops::resize(&full, nw, nh, image::imageops::FilterType::Triangle);
        Img::from_raw(nw as usize, nh as usize, resized.into_raw())
    }
}

#[inline]
fn dist(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> f32 {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxModel {
    Color,
    Smoke,
    Black,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxBgType {
    Blue,
    Green,
    Black,
    White,
    Magenta,
}

impl FxBgType {
    pub fn pure_color(self) -> [u8; 3] {
        match self {
            FxBgType::Blue => [0, 0, 255],
            FxBgType::Green => [0, 255, 0],
            FxBgType::Black => [0, 0, 0],
            FxBgType::White => [255, 255, 255],
            FxBgType::Magenta => [255, 0, 255],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FxBgType::Blue => "蓝底",
            FxBgType::Green => "绿底",
            FxBgType::Black => "黑底",
            FxBgType::White => "白底",
            FxBgType::Magenta => "洋红底",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FxParams {
    pub model: FxModel,
    pub bg: [u8; 3],
    pub color_tolerance: f32,
    pub black_point: f32,
    pub white_point: f32,
    pub density: f32,
    pub despill: f32,
}

#[inline]
fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// 原站 Xe(bg) 预设：切底色类型时的参数复位（黑底强制 Black 模型，其余 Color）
pub fn fx_preset_for(bg: FxBgType) -> FxParams {
    let black = bg == FxBgType::Black;
    FxParams {
        model: if black { FxModel::Black } else { FxModel::Color },
        bg: bg.pure_color(),
        color_tolerance: if black { 0.0 } else { 3.0 },
        black_point: if black { 0.0 } else { 1.0 },
        white_point: 100.0,
        density: 1.0,
        despill: if matches!(bg, FxBgType::Blue | FxBgType::Green | FxBgType::Magenta) { 20.0 } else { 0.0 },
    }
}

// ---------------------------------------------------------------------------
// 底色场估计（智能识别 + 半透明反解共用）
//
// 老问题：整幅只认「一个底色」。渐变底、暗角、双色底、主体压边、脏边都会让单色
// 失真；底色偏差 10 级，半透明烟雾的浓度就能偏 0.2 以上。
//
// 做法（纯手写、无依赖、O(边框长)）：
//   1) 边框环每条边切 16 段，每段取逐通道中位色 —— 抗噪点 / JPEG 脏边 / 局部异物；
//   2) 每条边用「本边中位色」做种子，剔除离群段（主体压边）后拟合二次曲线
//      c0 + c1·u + c2·u² —— 线性项管渐变，二次项管暗角 / 光照衰减；
//   3) 四条边互相在四角对齐校验，对不上的边判为不可靠（主体压住）；不可靠边用
//      相邻两条垂直边的角点值线性重建，仍不行才退成常数；
//   4) 内部取值 =「上下插值」与「左右插值」的平均 —— 径向暗角在四角自然收敛；
//   5) 置信度 = 场能解释边框像素的加权比例（双色底、杂乱背景会显著掉分）。
//
// 场只描述「形状」：使用时电平锚定到调用方给的底色，因此用户手选取色不会被
// 自动估计悄悄覆盖。
// ---------------------------------------------------------------------------

/// 每条边切分段数
const SEGS: usize = 16;

/// 一段边框的中位色样本（坐标 / 归一化坐标 / 0..1 色值 / 权重）
#[derive(Clone, Copy)]
struct SegSample {
    x: f32,
    y: f32,
    u: f32,
    c: [f32; 3],
    w: f32,
    side: u8, // 0 上 1 下 2 左 3 右
}

#[inline]
fn max_dev(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())
}

#[inline]
fn to_u8(c: [f32; 3]) -> [u8; 3] {
    [
        (c[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[2] * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// 中位数（就地排序；空切片返回 0）
fn median_f32(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let m = v.len() / 2;
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[m]
}

/// 3x3 系数矩阵 + 3 组右端项 → 高斯-约当消元（带主元），返回 [基函数][通道]
fn solve3(m: [[f64; 3]; 3], r: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let mut a = [[0.0f64; 6]; 3];
    for i in 0..3 {
        for j in 0..3 {
            a[i][j] = m[i][j];
        }
        for ch in 0..3 {
            a[i][3 + ch] = r[i][ch];
        }
    }
    for col in 0..3 {
        let mut p = col;
        for row in (col + 1)..3 {
            if a[row][col].abs() > a[p][col].abs() {
                p = row;
            }
        }
        if a[p][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, p);
        let inv = 1.0 / a[col][col];
        for k in col..6 {
            a[col][k] *= inv;
        }
        for row in 0..3 {
            if row == col {
                continue;
            }
            let f = a[row][col];
            if f.abs() < 1e-18 {
                continue;
            }
            for k in col..6 {
                a[row][k] -= f * a[col][k];
            }
        }
    }
    let mut out = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for ch in 0..3 {
            out[i][ch] = a[i][3 + ch];
        }
    }
    Some(out)
}

/// 加权最小二乘：pts = (x, y, 权重, 0..1 色值)，phi 给出 3 个基函数值
fn fit_basis<F>(pts: &[(f32, f32, f32, [f32; 3])], phi: F) -> Option<[[f64; 3]; 3]>
where
    F: Fn(f32, f32) -> [f64; 3],
{
    let mut m = [[0.0f64; 3]; 3];
    let mut r = [[0.0f64; 3]; 3];
    let mut sw = 0.0f64;
    for &(a, b, w, c) in pts {
        let p = phi(a, b);
        let w = w as f64;
        sw += w;
        for j in 0..3 {
            for k in 0..3 {
                m[j][k] += w * p[j] * p[k];
            }
            for ch in 0..3 {
                r[j][ch] += w * p[j] * c[ch] as f64;
            }
        }
    }
    if sw <= 0.0 {
        return None;
    }
    solve3(m, r)
}

#[inline]
fn coef_at(c: &[[f64; 3]; 3], p: [f64; 3]) -> [f32; 3] {
    [
        (c[0][0] * p[0] + c[1][0] * p[1] + c[2][0] * p[2]) as f32,
        (c[0][1] * p[0] + c[1][1] * p[1] + c[2][1] * p[2]) as f32,
        (c[0][2] * p[0] + c[1][2] * p[1] + c[2][2] * p[2]) as f32,
    ]
}

/// 边框环分段采样：每边 SEGS 段，每段取逐通道中位色（跳过透明像素）
fn sample_border(buf: &[u8], w: usize, h: usize) -> Vec<SegSample> {
    let mut out: Vec<SegSample> = Vec::with_capacity(SEGS * 4);
    if w < 24 || h < 24 || buf.len() < w * h * 4 {
        return out;
    }
    // 采样带宽度：短边的 3.5%，2..28 像素，且不超短边 1/3
    let band = ((w.min(h) as f32 * 0.035) as usize)
        .clamp(2, 28)
        .min(w.min(h) / 3)
        .max(1);
    for side in 0..4u8 {
        let along_n = if side < 2 { w } else { h };
        for k in 0..SEGS {
            let a0 = k * along_n / SEGS;
            let a1 = (((k + 1) * along_n) / SEGS).max(a0 + 1).min(along_n);
            let span = a1.saturating_sub(a0);
            if span == 0 {
                continue;
            }
            let total = span * band;
            let stride = (total / 40).max(1);
            let mut ch: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            let mut i = 0usize;
            while i < total {
                let along = a0 + (i % span);
                let depth = (i / span).min(band - 1);
                i += stride;
                let (x, y) = match side {
                    0 => (along, depth),
                    1 => (along, h - 1 - depth),
                    2 => (depth, along),
                    _ => (w - 1 - depth, along),
                };
                let o = (y * w + x) * 4;
                if buf[o + 3] < 240 {
                    continue;
                }
                ch[0].push(buf[o]);
                ch[1].push(buf[o + 1]);
                ch[2].push(buf[o + 2]);
            }
            if ch[0].len() < 4 {
                continue;
            }
            let mut c = [0f32; 3];
            for v in 0..3 {
                let n = ch[v].len();
                ch[v].sort_unstable();
                c[v] = ch[v][n / 2] as f32 / 255.0;
            }
            let ca = (a0 + a1) as f32 * 0.5;
            let cd = band as f32 * 0.5;
            let (x, y) = match side {
                0 => (ca, cd),
                1 => (ca, h as f32 - 1.0 - cd),
                2 => (cd, ca),
                _ => (w as f32 - 1.0 - cd, ca),
            };
            out.push(SegSample {
                x,
                y,
                u: ca / along_n as f32,
                c,
                w: ch[0].len() as f32,
                side,
            });
        }
    }
    out
}

/// 边框底色场：四角双线性底座 + 四条边的曲率修正（暗角 / 光照衰减）
///
/// 取值：f(x,y) = 四角双线性 + (1-s)·eT[x] + s·eB[x] + (1-t)·eL[y] + t·eR[y]
/// 四条边上严格等于各自拟合出的二次曲线，内部平滑过渡；纯双线性底色可精确还原，
/// 因此不会出现「上/下 + 左/右 简单平均」在四角约 1/4 梯度幅度的系统偏差。
struct BgField {
    w: usize,
    h: usize,
    /// 四角色值：0=左上 1=右上 2=左下 3=右下
    c: [[f32; 3]; 4],
    /// 四条边相对「角点连线」的曲率修正
    et: Vec<[f32; 3]>,
    eb: Vec<[f32; 3]>,
    el: Vec<[f32; 3]>,
    er: Vec<[f32; 3]>,
    /// 内部「碗形」项（四条边中点曲率的平均，专治径向暗角）
    bowl: [f32; 3],
    mean: [f32; 3],
    /// 场的空间变化幅度（0..1）
    spread: f32,
    /// 置信度：场能解释边框采样的加权比例
    conf: f32,
    /// 边框相对场的中位残差（≈ 噪声量级，0..1）
    noise: f32,
    /// 连通有效边占边框的权重比
    core: f32,
    /// 「一条边被完整解释 + 其余边是主体」也算可信（主体压边场景）
    edge_ok: bool,
    /// 边框横跨两种底色（某条边两端都贴合场、中段明显背离）
    dual: bool,
    valid: [bool; 4],
}

impl BgField {
    fn estimate(buf: &[u8], w: usize, h: usize) -> Option<BgField> {
        let segs = sample_border(buf, w, h);
        if segs.len() < 10 {
            return None;
        }
        let all_w: f32 = segs.iter().map(|s| s.w).sum();
        if all_w <= 0.0 {
            return None;
        }

        // ---------------- 1) 逐边二次拟合 ----------------
        // 种子用「本边中位色」，对 <50% 的压边主体稳健；离群段（主体/异物）剔除后拟合。
        let mut quads = [[[0f64; 3]; 3]; 4];
        let mut side_ok = [false; 4];
        let mut side_med = [[0f32; 3]; 4];
        let mut side_q = [0f32; 4]; // 该边被拟合曲线解释的加权比例
        for side in 0..4usize {
            let ss: Vec<SegSample> =
                segs.iter().filter(|s| s.side as usize == side).copied().collect();
            if ss.len() < 3 {
                continue;
            }
            let sw: f32 = ss.iter().map(|s| s.w).sum();
            if sw <= 0.0 {
                continue;
            }
            let mut seed = [0f32; 3];
            for chn in 0..3 {
                let mut v: Vec<(f32, f32)> = ss.iter().map(|s| (s.c[chn], s.w)).collect();
                v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let mut acc = 0.0f32;
                let mut pick = v.first().map(|x| x.0).unwrap_or(0.0);
                for (val, ww) in &v {
                    acc += *ww;
                    if acc >= sw * 0.5 {
                        pick = *val;
                        break;
                    }
                }
                seed[chn] = pick;
            }
            side_med[side] = seed;

            // 离群段剔除（上限 0.20，防止双色底被 MAD 放大后全部当内点）
            let res: Vec<f32> = ss.iter().map(|s| max_dev(s.c, seed)).collect();
            let mut tmp = res.clone();
            let mad = median_f32(&mut tmp);
            let tol = 0.09f32.max((3.0 * mad).min(0.20));
            let mut cur: Vec<(f32, f32, f32, [f32; 3])> = ss
                .iter()
                .zip(res.iter())
                .filter(|(_, r)| **r <= tol)
                .map(|(s, _)| (s.u, 0.0, s.w, s.c))
                .collect();
            if cur.len() < 3 {
                cur = ss.iter().map(|s| (s.u, 0.0, s.w, s.c)).collect();
            }
            let mut q = match fit_basis(&cur, |u, _| [1.0, u as f64, u as f64 * u as f64]) {
                Some(q) => q,
                None => continue,
            };
            // 对二次曲线再做一轮残差剔除
            let res2: Vec<f32> = cur
                .iter()
                .map(|(u, _, _, c)| {
                    let uu = *u as f64;
                    max_dev(*c, coef_at(&q, [1.0, uu, uu * uu]))
                })
                .collect();
            let mut tmp2 = res2.clone();
            let mad2 = median_f32(&mut tmp2);
            let tol2 = 0.045f32.max((3.0 * mad2).min(0.13));
            let kept: Vec<(f32, f32, f32, [f32; 3])> = cur
                .iter()
                .zip(res2.iter())
                .filter(|(_, r)| **r <= tol2)
                .map(|(c, _)| *c)
                .collect();
            // 内点太少就退回粗筛结果，避免空拟合
            let src: &[(f32, f32, f32, [f32; 3])] = if kept.len() >= 3 { &kept } else { &cur };
            let (mut lo, mut hi) = (1.0f32, 0.0f32);
            for (u, _, _, _) in src {
                lo = lo.min(*u);
                hi = hi.max(*u);
            }
            let span = (hi - lo).max(0.0);
            // 跨度不足 0.70 时降阶为线性：二次曲线在半条边上外推会发散
            if span < 0.70 {
                if let Some(ql) = fit_basis(src, |u, _| [1.0, u as f64, 0.0]) {
                    q = ql;
                }
            } else if let Some(q2) = fit_basis(src, |u, _| [1.0, u as f64, u as f64 * u as f64]) {
                q = q2;
            }
            // 解释率：整条边（含被剔除段）有多少权重落在这条曲线上
            let hit_tol = tol2.max(0.075);
            let mut hit = 0.0f32;
            for s in &ss {
                let uu = s.u as f64;
                if max_dev(s.c, coef_at(&q, [1.0, uu, uu * uu])) <= hit_tol {
                    hit += s.w;
                }
            }
            side_q[side] = hit / sw;
            // 外推幅度合理性：端点离任何已知段都不应超过 0.5（约 128 级）
            let e0 = coef_at(&q, [1.0, 0.0, 0.0]);
            let e1 = coef_at(&q, [1.0, 1.0, 1.0]);
            let dev0 = src.iter().map(|(_, _, _, c)| max_dev(e0, *c)).fold(1.0f32, f32::min);
            let dev1 = src.iter().map(|(_, _, _, c)| max_dev(e1, *c)).fold(1.0f32, f32::min);
            // 有效条件：段数够 + 跨度够 + 外推不离谱
            if src.len() >= 3 && span >= 0.35 && dev0 <= 0.5 && dev1 <= 0.5 {
                quads[side] = q;
                side_ok[side] = true;
            }
        }
        if !side_ok.iter().any(|v| *v) {
            return None;
        }

        // ---------------- 2) 四角连通：只保留能互相接上的边 ----------------
        // 四种「边-边」在角点的对应：上左 / 上右 / 下右 / 下左
        const CORNERS: [(usize, usize, usize, usize); 4] = [
            (0, 0, 2, 0),
            (0, 1, 3, 0),
            (1, 1, 3, 1),
            (1, 0, 2, 1),
        ];
        let mut ends = [[[0f32; 3]; 2]; 4];
        for side in 0..4 {
            if side_ok[side] {
                ends[side][0] = coef_at(&quads[side], [1.0, 0.0, 0.0]);
                ends[side][1] = coef_at(&quads[side], [1.0, 1.0, 1.0]);
            }
        }
        let mut best = 0usize;
        let mut bq = -1.0f32;
        for s in 0..4 {
            if side_ok[s] && side_q[s] > bq {
                bq = side_q[s];
                best = s;
            }
        }
        let mut conn = [false; 4];
        conn[best] = true;
        let mut queue = vec![best];
        while let Some(a) = queue.pop() {
            for &(sa, ea, sb, eb) in &CORNERS {
                if !(sa == a || sb == a) {
                    continue;
                }
                if side_ok[sa]
                    && side_ok[sb]
                    && max_dev(ends[sa][ea], ends[sb][eb]) <= 0.13
                {
                    for &s in &[sa, sb] {
                        if !conn[s] {
                            conn[s] = true;
                            queue.push(s);
                        }
                    }
                }
            }
        }
        for s in 0..4 {
            if !conn[s] {
                side_ok[s] = false;
            }
        }
        if !side_ok.iter().any(|v| *v) {
            return None;
        }

        // ---------------- 3) 生成四条边函数（不可靠边用相邻边角点重建） ----------------
        let ux = |x: usize| if w > 1 { x as f32 / (w - 1) as f32 } else { 0.5 };
        let uy = |y: usize| if h > 1 { y as f32 / (h - 1) as f32 } else { 0.5 };
        let qv = |side: usize, u: f32| {
            if side_ok[side] {
                let uu = u as f64;
                coef_at(&quads[side], [1.0, uu, uu * uu])
            } else {
                side_med[side]
            }
        };
        let mut top = vec![[0f32; 3]; w];
        let mut bottom = vec![[0f32; 3]; w];
        let mut left = vec![[0f32; 3]; h];
        let mut right = vec![[0f32; 3]; h];
        for x in 0..w {
            top[x] = qv(0, ux(x));
            bottom[x] = qv(1, ux(x));
        }
        for y in 0..h {
            left[y] = qv(2, uy(y));
            right[y] = qv(3, uy(y));
        }
        let lerp_arr = |a: [f32; 3], b: [f32; 3], n: usize| -> Vec<[f32; 3]> {
            (0..n)
                .map(|i| {
                    let t = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.5 };
                    [
                        a[0] + (b[0] - a[0]) * t,
                        a[1] + (b[1] - a[1]) * t,
                        a[2] + (b[2] - a[2]) * t,
                    ]
                })
                .collect()
        };
        let mut mean_all = [0f32; 3];
        let mut mw = 0.0f32;
        for side in 0..4 {
            if side_ok[side] {
                for v in 0..3 {
                    mean_all[v] += side_med[side][v];
                }
                mw += 1.0;
            }
        }
        for v in 0..3 {
            mean_all[v] = if mw > 0.0 { mean_all[v] / mw } else { 0.0 };
        }
        if !side_ok[0] {
            top = if side_ok[2] && side_ok[3] {
                lerp_arr(left[0], right[0], w)
            } else if side_ok[1] {
                bottom.clone()
            } else if side_ok[2] {
                vec![left[0]; w]
            } else if side_ok[3] {
                vec![right[0]; w]
            } else {
                vec![mean_all; w]
            };
        }
        if !side_ok[1] {
            bottom = if side_ok[2] && side_ok[3] {
                lerp_arr(left[h - 1], right[h - 1], w)
            } else if side_ok[0] {
                top.clone()
            } else if side_ok[2] {
                vec![left[h - 1]; w]
            } else if side_ok[3] {
                vec![right[h - 1]; w]
            } else {
                vec![mean_all; w]
            };
        }
        if !side_ok[2] {
            left = if side_ok[0] && side_ok[1] {
                lerp_arr(top[0], bottom[0], h)
            } else if side_ok[3] {
                right.clone()
            } else if side_ok[0] {
                vec![top[0]; h]
            } else if side_ok[1] {
                vec![bottom[0]; h]
            } else {
                vec![mean_all; h]
            };
        }
        if !side_ok[3] {
            right = if side_ok[0] && side_ok[1] {
                lerp_arr(top[w - 1], bottom[w - 1], h)
            } else if side_ok[2] {
                left.clone()
            } else if side_ok[0] {
                vec![top[w - 1]; h]
            } else if side_ok[1] {
                vec![bottom[w - 1]; h]
            } else {
                vec![mean_all; h]
            };
        }
        for arr in [&mut top, &mut bottom, &mut left, &mut right] {
            for c in arr.iter_mut() {
                for v in 0..3 {
                    c[v] = c[v].clamp(0.0, 1.0);
                }
            }
        }

        // ---------------- 4) 拆成「四角 + 四条边曲率」 ----------------
        let c = [top[0], top[w - 1], bottom[0], bottom[w - 1]];
        let mut et = vec![[0f32; 3]; w];
        let mut eb = vec![[0f32; 3]; w];
        let mut el = vec![[0f32; 3]; h];
        let mut er = vec![[0f32; 3]; h];
        for x in 0..w {
            let t = ux(x);
            for v in 0..3 {
                et[x][v] = top[x][v] - (c[0][v] + (c[1][v] - c[0][v]) * t);
                eb[x][v] = bottom[x][v] - (c[2][v] + (c[3][v] - c[2][v]) * t);
            }
        }
        for y in 0..h {
            let s = uy(y);
            for v in 0..3 {
                el[y][v] = left[y][v] - (c[0][v] + (c[2][v] - c[0][v]) * s);
                er[y][v] = right[y][v] - (c[1][v] + (c[3][v] - c[1][v]) * s);
            }
        }

        // 碗形项：四条边中点曲率的平均。纯线性渐变时为 0；径向暗角时为负/正，
        // 用来补足「四角双线性 + 边曲率」在内部对碗形场欠修正的那部分。
        let mut bowl = [0f32; 3];
        for v in 0..3 {
            bowl[v] = 0.25
                * (et[w / 2][v] + eb[w / 2][v] + el[h / 2][v] + er[h / 2][v]);
        }

        // 变化幅度（四角 + 四条边上的极值）
        let mut mn = [1f32; 3];
        let mut mx = [0f32; 3];
        for arr in [&top, &bottom, &left, &right] {
            for p in arr {
                for v in 0..3 {
                    mn[v] = mn[v].min(p[v]);
                    mx[v] = mx[v].max(p[v]);
                }
            }
        }
        let spread = (0..3).map(|v| mx[v] - mn[v]).fold(0.0f32, f32::max);

        // 电平（代表色）：取「场在有效边采样点处预测值」的加权中位数。
        // 与场自洽（电平与场差几级就会在底色边界上产生一圈假 alpha），
        // 而且中位数不会被某一条边的投影 / 主体压住拉灰 —— 白底拉灰几级就足以
        // 把整片背景判成前景（相对偏离在近白处被放大）。
        let field0 = BgField {
            w,
            h,
            c,
            et,
            eb,
            el,
            er,
            bowl: [0.0; 3],
            mean: [0.0; 3],
            spread,
            conf: 0.0,
            noise: 0.0,
            core: 0.0,
            edge_ok: false,
            dual: false,
            valid: side_ok,
        };
        let mut mean = [0f32; 3];
        for v in 0..3 {
            let mut vals: Vec<(f32, f32)> = segs
                .iter()
                .filter(|s| side_ok[s.side as usize] && conn[s.side as usize])
                .map(|s| {
                    let x = (s.x as usize).min(w - 1);
                    let y = (s.y as usize).min(h - 1);
                    (field0.raw_at(x, y)[v], s.w)
                })
                .collect();
            if vals.is_empty() {
                vals = segs.iter().map(|s| (s.c[v], s.w)).collect();
            }
            vals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let tot: f32 = vals.iter().map(|x| x.1).sum();
            let mut acc = 0.0f32;
            let mut pick = vals.first().map(|x| x.0).unwrap_or(0.0);
            for (val, ww) in &vals {
                acc += *ww;
                if acc >= tot * 0.5 {
                    pick = *val;
                    break;
                }
            }
            mean[v] = pick.clamp(0.0, 1.0);
        }
        // 近白/近中性底色：改用 0.90 分位。中位数会把噪声下的白底压到 250 上下，
        // 而 (255-254)/(255-254)=1 —— 一级之差就能把整片白底判成前景。
        {
            let mn = mean[0].min(mean[1]).min(mean[2]);
            let mx = mean[0].max(mean[1]).max(mean[2]);
            if mn > 0.85 && mx - mn < 0.08 {
                for v in 0..3 {
                    let mut vals: Vec<(f32, f32)> = segs
                        .iter()
                        .filter(|s| side_ok[s.side as usize] && conn[s.side as usize])
                        .map(|s| {
                            let x = (s.x as usize).min(w - 1);
                            let y = (s.y as usize).min(h - 1);
                            (field0.raw_at(x, y)[v], s.w)
                        })
                        .collect();
                    if vals.is_empty() {
                        continue;
                    }
                    vals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    let tot: f32 = vals.iter().map(|x| x.1).sum();
                    let mut acc = 0.0f32;
                    let _ = (&mut acc, tot);
                    let mut pick = 0.0f32;
                    for (val, _) in &vals {
                        pick = pick.max(*val);
                    }
                    mean[v] = pick.clamp(0.0, 1.0);
                }
            }
        }
        let field = BgField { mean, bowl, ..field0 };
        // 置信度：场在采样点处的解释率（加权）；同时统计「连通有效边」的权重与解释率
        let mut hit = 0.0f32;
        let mut w_core = 0.0f32;
        let mut hit_core = 0.0f32;
        for s in &segs {
            let x = (s.x as usize).min(w - 1);
            let y = (s.y as usize).min(h - 1);
            let ok = max_dev(s.c, field.raw_at(x, y)) <= 0.075;
            if ok {
                hit += s.w;
            }
            if conn[s.side as usize] {
                w_core += s.w;
                if ok {
                    hit_core += s.w;
                }
            }
        }
        let conf = hit / all_w;
        let core = w_core / all_w;
        let core_agree = if w_core > 0.0 { hit_core / w_core } else { 0.0 };
        // 一条边被完整解释、其余边是压边主体 → 也认为底色可信
        let edge_ok = core >= 0.20 && core_agree >= 0.90;
        let mut res: Vec<f32> = segs
            .iter()
            .map(|s| {
                let x = (s.x as usize).min(w - 1);
                let y = (s.y as usize).min(h - 1);
                max_dev(s.c, field.raw_at(x, y))
            })
            .collect();
        // 注意：median_f32 会就地排序，必须用副本，否则 res 与 segs 的对应关系被破坏
        let noise = {
            let mut tmp = res.clone();
            median_f32(&mut tmp)
        };
        // 双色底判定：某条边「两端贴合场、中段明显背离」→ 这条边横跨两种底色。
        // 主体压边时残差是单侧的（一端高），不会被误判；平滑渐变的中段残差也很小。
        let mut dual = false;
        for side in 0..4usize {
            let mut prof: Vec<(f32, f32, [f32; 3])> = segs
                .iter()
                .zip(res.iter())
                .filter(|(sg, _)| sg.side as usize == side)
                .map(|(sg, r)| (sg.u, *r, sg.c))
                .collect();
            if prof.len() < 8 {
                continue;
            }
            prof.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let n = prof.len();
            // 两端必须颜色明显不同（双色底的两种底色），否则中段背离只是
            // 「贴着边框中段的主体/烟雾」——那种情况拟合已经把它们当离群点剔掉了。
            let ends_far = prof
                .first()
                .zip(prof.last())
                .map(|(a, b)| max_dev(a.2, b.2) > 0.30)
                .unwrap_or(false);
            if ends_far && prof[0].1 < 0.09 && prof[n - 1].1 < 0.09 {
                let mid_max = prof[n / 4..=3 * n / 4]
                    .iter()
                    .map(|x| x.1)
                    .fold(0.0f32, f32::max);
                if mid_max > 0.18 {
                    dual = true;
                }
            }
        }
        Some(BgField { conf, noise, core, edge_ok, dual, ..field })
    }

    /// 逐点原始底色（不锚定，0..1）
    #[inline]
    fn raw_at(&self, x: usize, y: usize) -> [f32; 3] {
        let t = if self.w > 1 { x as f32 / (self.w - 1) as f32 } else { 0.5 };
        let s = if self.h > 1 { y as f32 / (self.h - 1) as f32 } else { 0.5 };
        let it = 1.0 - t;
        let is = 1.0 - s;
        let c = &self.c;
        let et = self.et[x];
        let eb = self.eb[x];
        let el = self.el[y];
        let er = self.er[y];
        let bowl_w = (4.0 * t * it) * (4.0 * s * is);
        let mut o = [0f32; 3];
        for v in 0..3 {
            o[v] = c[0][v] * it * is
                + c[1][v] * t * is
                + c[2][v] * it * s
                + c[3][v] * t * s
                + et[v] * is
                + eb[v] * s
                + el[v] * it
                + er[v] * t
                + self.bowl[v] * bowl_w;
        }
        o
    }

    /// 场是否足够可靠（识别 / 反解共用）
    ///
    /// 两条路：①场能解释大部分边框；②至少一条边被完整解释、其余边像主体压边。
    /// 双色底 / 杂乱边框两条都不满足 → 交由调用方提示用户手动取色。
    fn reliable(&self) -> bool {
        !self.dual
            && self.valid.iter().any(|v| *v)
            && (self.conf >= 0.55 || self.edge_ok)
    }
}

/// 底色像素分类（原站 Ut）：判断单像素是否"像"指定类型的底色
fn looks_like_bg(c: [u8; 3], bg: FxBgType) -> bool {
    let (r, g, b) = (c[0] as i32, c[1] as i32, c[2] as i32);
    let mn = c[0].min(c[1]).min(c[2]);
    let mx = c[0].max(c[1]).max(c[2]);
    match bg {
        FxBgType::Blue => b > r + 30 && b > g + 20,
        FxBgType::Green => g > r + 30 && g > b + 20,
        FxBgType::Magenta => r > g + 25 && b > g + 25,
        FxBgType::White => mn > 210 && mx - mn < 25,
        FxBgType::Black => mx < 32,
    }
}

/// 底色类型「像不像」（比 looks_like_bg 宽松一档，用于渐变/暗角下的场均值判定）
fn bg_type_plausible(c: [f32; 3], bg: FxBgType) -> bool {
    let (r, g, b) = (c[0] * 255.0, c[1] * 255.0, c[2] * 255.0);
    let mn = r.min(g).min(b);
    let mx = r.max(g).max(b);
    match bg {
        FxBgType::Blue => b > r + 20.0 && b > g + 15.0,
        FxBgType::Green => g > r + 20.0 && g > b + 15.0,
        FxBgType::Magenta => r > g + 18.0 && b > g + 18.0,
        FxBgType::White => mn > 195.0 && mx - mn < 30.0,
        FxBgType::Black => mx < 40.0,
    }
}

/// 把边框段样本按底色类型过滤 → 加权中位数 → ±0.12 内加权均值（去掉桶量化误差）
fn typed_border_color(segs: &[SegSample], bg: FxBgType) -> Option<[u8; 3]> {
    let pts: Vec<([f32; 3], f32)> = segs
        .iter()
        .filter(|s| looks_like_bg(to_u8(s.c), bg))
        .map(|s| (s.c, s.w))
        .collect();
    let tw: f32 = pts.iter().map(|p| p.1).sum();
    if pts.len() < 3 || tw <= 0.0 {
        return None;
    }
    let mut seed = [0f32; 3];
    for chn in 0..3 {
        let mut v: Vec<(f32, f32)> = pts.iter().map(|p| (p.0[chn], p.1)).collect();
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut acc = 0.0f32;
        let mut pick = v.first().map(|x| x.0).unwrap_or(0.0);
        for (val, ww) in &v {
            acc += *ww;
            if acc >= tw * 0.5 {
                pick = *val;
                break;
            }
        }
        seed[chn] = pick;
    }
    let mut sum = [0f32; 3];
    let mut wsum = 0.0f32;
    for (c, ww) in &pts {
        if max_dev(*c, seed) <= 0.12 {
            for chn in 0..3 {
                sum[chn] += c[chn] * *ww;
            }
            wsum += *ww;
        }
    }
    if wsum <= 0.0 {
        return None;
    }
    Some(to_u8([sum[0] / wsum, sum[1] / wsum, sum[2] / wsum]))
}

/// 老版全图取样（兜底）：降采样 → 类型分类 → 5-bit 主色桶 → 逐通道中位数
fn legacy_auto_bg_color(img: &Img, bg: FxBgType) -> Option<[u8; 3]> {
    let scale = (512.0 / img.w.max(img.h) as f32).min(1.0);
    let nw = ((img.w as f32 * scale).round() as u32).max(1);
    let nh = ((img.h as f32 * scale).round() as u32).max(1);
    let small = if nw == img.w as u32 && nh == img.h as u32 {
        img.clone()
    } else {
        let full = image::RgbaImage::from_raw(img.w as u32, img.h as u32, img.buf.clone())?;
        let r = image::imageops::resize(&full, nw, nh, image::imageops::FilterType::Nearest);
        Img::from_raw(nw as usize, nh as usize, r.into_raw())
    };
    let n = small.w * small.h;
    let stride = ((n as f32 / 20_000.0).ceil() as usize).max(1);
    let mut buckets: std::collections::HashMap<u32, Vec<[u8; 3]>> = std::collections::HashMap::new();
    let mut counted = 0usize;
    let mut i = 0usize;
    while i < n {
        let o = i * 4;
        i += stride;
        if small.buf[o + 3] < 240 {
            continue;
        }
        counted += 1;
        let c = [small.buf[o], small.buf[o + 1], small.buf[o + 2]];
        if !looks_like_bg(c, bg) {
            continue;
        }
        let key = ((c[0] >> 3) as u32) << 10 | ((c[1] >> 3) as u32) << 5 | (c[2] >> 3) as u32;
        buckets.entry(key).or_default().push(c);
    }
    let best = buckets.values_mut().max_by_key(|v| v.len())?;
    if best.len() < ((counted as f32 * 0.05) as usize).max(8) {
        return None;
    }
    let len = best.len();
    let mut out = [0u8; 3];
    for ch in 0..3 {
        best.sort_by_key(|c| c[ch]);
        out[ch] = best[len / 2][ch];
    }
    Some(out)
}

/// 从原图自动取样底色（原站 Ct/kt 的强化版）
///
/// 顺序：
///   1) 边框底色场的面积均值（抗渐变 / 暗角 / 主体压边）——类型吻合时直接用；
///   2) 边框段按类型过滤后做加权中位数 + 邻域均值（比 5-bit 桶中位数少一级量化误差）；
///   3) 兜底：老逻辑（全图降采样 + 类型分类 + 主色桶中位数）。
pub fn auto_bg_color(img: &Img, bg: FxBgType) -> Option<[u8; 3]> {
    if bg == FxBgType::Black {
        return Some([0, 0, 0]);
    }
    let (w, h) = (img.w, img.h);
    if w == 0 || h == 0 || img.buf.len() < w * h * 4 {
        return None;
    }
    if let Some(f) = BgField::estimate(&img.buf, w, h) {
        if f.reliable() && bg_type_plausible(f.mean, bg) {
            return Some(to_u8(f.mean));
        }
    }
    let segs = sample_border(&img.buf, w, h);
    if let Some(c) = typed_border_color(&segs, bg) {
        return Some(c);
    }
    legacy_auto_bg_color(img, bg)
}

/// 智能识别底色：边框环鲁棒底色场 → 面积均值 + 推荐模型
///
/// 相比老版（外圈 4% + 5-bit 主色桶 + 中位数）：
///   · 渐变底按二次曲面建模，不再被「桶」切碎；
///   · 主体压边段被当离群点剔除，且四角一致性可校验；
///   · 双色底 / 杂乱边框拟合率不足 → 返回 None（提示改色键），不再硬给一个错色；
///   · 顺带给出模型建议：近黑 → Black、中性灰 → Smoke、其余 → Color。
pub fn detect_bg_auto(img: &Img) -> Option<([u8; 3], FxModel)> {
    let (w, h) = (img.w, img.h);
    if w < 24 || h < 24 || img.buf.len() < w * h * 4 {
        // 极小的图没有可靠边框：退回老逻辑，保证行为不倒退
        return legacy_detect_bg_auto(img);
    }
    let f = BgField::estimate(&img.buf, w, h)?;
    if !f.reliable() {
        return None;
    }
    let c = to_u8(f.mean);
    let (r, g, b) = (c[0] as f32, c[1] as f32, c[2] as f32);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let chroma = mx - mn;
    let luma = 0.299 * r + 0.587 * g + 0.114 * b;
    let model = if luma < 38.0 && chroma < 42.0 {
        FxModel::Black
    } else if chroma < 18.0 && (60.0..215.0).contains(&luma) {
        // 中性灰底：光效与烟雾在此等价，推荐更贴合「烟雾/薄纱」的模型
        FxModel::Smoke
    } else {
        FxModel::Color
    };
    Some((c, model))
}

/// 老版识别（仅用于 <24px 的极小图兜底）
fn legacy_detect_bg_auto(img: &Img) -> Option<([u8; 3], FxModel)> {
    let bw = ((img.w as f32 * 0.04) as usize).clamp(2, 24);
    let bh = ((img.h as f32 * 0.04) as usize).clamp(2, 24);
    let mut buckets: std::collections::HashMap<u32, Vec<[u8; 3]>> = std::collections::HashMap::new();
    let mut count = 0usize;
    for y in 0..img.h {
        for x in 0..img.w {
            if !(x < bw || y < bh || x + bw >= img.w || y + bh >= img.h) {
                continue;
            }
            let o = (y * img.w + x) * 4;
            if img.buf[o + 3] < 240 {
                continue;
            }
            count += 1;
            let c = [img.buf[o], img.buf[o + 1], img.buf[o + 2]];
            let key = ((c[0] >> 3) as u32) << 10 | ((c[1] >> 3) as u32) << 5 | (c[2] >> 3) as u32;
            buckets.entry(key).or_default().push(c);
        }
    }
    let best = buckets.values_mut().max_by_key(|v| v.len())?;
    if best.len() < ((count as f32 * 0.10) as usize).max(8) {
        return None;
    }
    let len = best.len();
    let mut out = [0u8; 3];
    for ch in 0..3 {
        best.sort_by_key(|c| c[ch]);
        out[ch] = best[len / 2][ch];
    }
    let mn = out[0].min(out[1]).min(out[2]);
    let mx = out[0].max(out[1]).max(out[2]);
    let model = if mn > 210 && mx - mn < 25 {
        FxModel::Color
    } else if mx < 32 {
        FxModel::Black
    } else {
        FxModel::Color
    };
    Some((out, model))
}

/// 半透明特效抠图主循环（原站 wt 的升级版）
///
/// 对原实现的改动（公开行为兼容；底色为纯色主键时结果与老版一致）：
///   1) 底色参考从「单一色值」升级为「底色场」：边框渐变 / 暗角逐点建模，
///      电平仍锚定调用方给的 bg（用户手选取色优先），因此平坦底色结果逐位一致；
///   2) 覆盖度 A 增加「主导通道盈余」项（原站仅烟雾模型启用）：
///      彩色光效模型下按 a_cov 的平滑权重并入，用于把绿幕上的实心物体（橙圆、黑环）
///      从 0.78~0.88 抬回 1.0；白/中性前景该项与相对偏离严格相等，烟雾浓度不受影响；
///   3) 浅投影抑制：白/灰底上「等比变暗」（色度不变、亮度 0.60~0.94）判为投影 → alpha=0；
///   4) 黑屏光效：用底色场的局部亮度做覆盖率基准并参与反解，暗底噪点不再浮起一层灰雾；
///   5) 半透明保留：不做形态学收缩/二值化，只清 alpha<0.5/255 的尾巴，烟雾梯度完整；
///   6) 性能：常色底走快路径；density=1 时跳过 powf（预览大头）。
pub fn fx_matte(buf: &mut [u8], w: u32, h: u32, p: &FxParams) {
    let (wu, hu) = (w as usize, h as usize);
    if wu == 0 || hu == 0 || buf.len() < wu * hu * 4 {
        return;
    }
    let black_model = p.model == FxModel::Black;
    let o: [f32; 3] = if black_model {
        [0.0, 0.0, 0.0]
    } else {
        [
            clamp01(p.bg[0] as f32 / 255.0),
            clamp01(p.bg[1] as f32 / 255.0),
            clamp01(p.bg[2] as f32 / 255.0),
        ]
    };

    // 边框底色场：可靠才用；黑屏光效只接受「确实是暗底」的场
    let raw_field = BgField::estimate(buf, wu, hu).filter(|f| f.reliable());
    let fld = match raw_field.as_ref() {
        Some(f) if black_model => {
            let mx = f.mean[0].max(f.mean[1]).max(f.mean[2]);
            if mx < 0.35 {
                Some(f)
            } else {
                None
            }
        }
        other => other,
    };
    // 近白底：不做逐点场。底色接近 255 时 (ch-b)/(1-b) 会把几级误差放大成 alpha=1，
    // 投影已经由等比变暗规则单独处理，这里保持常色最稳。
    let near_white = if black_model {
        false
    } else {
        let e = fld.map(|f| f.mean).unwrap_or(o);
        let mn = e[0].min(e[1]).min(e[2]);
        let mx = e[0].max(e[1]).max(e[2]);
        mn > 0.85 && mx - mn < 0.08
    };
    // 场有空间变化才走逐点分支；平坦底色保持老版快路径（结果逐位一致）
    let use_field = !near_white && fld.map(|f| f.spread >= 0.012).unwrap_or(false);
    let tiny = |v: f32| if v < 0.05 { 0.0 } else { v };
    let b_const: [f32; 3] = if black_model {
        match fld {
            Some(f) => f.mean,
            None => [0.0, 0.0, 0.0],
        }
    } else {
        [tiny(o[0]), tiny(o[1]), tiny(o[2])]
    };
    // 白/灰底才启用浅投影抑制
    let bg_bright = if black_model {
        false
    } else {
        let e = if use_field {
            fld.map(|f| f.mean).unwrap_or(o)
        } else {
            b_const
        };
        let mn = e[0].min(e[1]).min(e[2]);
        let mx = e[0].max(e[1]).max(e[2]);
        mn > 0.55 && mx - mn < 0.10
    };

    // 底色场电平锚定到调用方底色：B = 场 + (bg - 场均值)
    // 这样用户手动取色 / 预设不会被自动估计覆盖，但渐变形状仍然生效。
    let off: [f32; 3] = if use_field {
        match fld {
            Some(f) if !black_model => {
                [o[0] - f.mean[0], o[1] - f.mean[1], o[2] - f.mean[2]]
            }
            _ => [0.0; 3],
        }
    } else {
        [0.0; 3]
    };
    // 逐行底色缓冲（每行重算一次，行内像素只做一次读取）
    let mut row: Vec<[f32; 3]> = vec![[0.0f32; 3]; wu];

    let r_lo = clamp01(p.black_point / 100.0);
    let r_hi = (r_lo + 0.001).max(clamp01(p.white_point / 100.0));
    let span_inv = 1.0 / (r_hi - r_lo);
    let density = p.density.clamp(0.2, 3.0);
    let inv_u = 1.0 / density;
    let no_pow = (inv_u - 1.0).abs() < 1e-4;
    let desp = clamp01(p.despill / 100.0);
    let ytol = p.color_tolerance.clamp(0.0, 20.0);
    let ytol_inv = if ytol > 0.0 { 1.0 / ytol } else { 0.0 };

    let mut idx = 0usize;
    for y in 0..hu {
        if use_field {
            if let Some(f) = fld {
                let s = if hu > 1 { y as f32 / (hu - 1) as f32 } else { 0.5 };
                let is = 1.0 - s;
                let el = f.el[y];
                let er = f.er[y];
                let bowl = f.bowl;
                let s4 = 4.0 * s * is;
                let w1 = (wu - 1).max(1) as f32;
                let c0c = f.c[0];
                let c1c = f.c[1];
                let c2c = f.c[2];
                let c3c = f.c[3];
                // 迭代器 + zip：省掉每像素 6 次 Vec 越界检查
                for (x, (r, (e1, e2))) in
                    row.iter_mut().zip(f.et.iter().zip(f.eb.iter())).enumerate()
                {
                    let t = x as f32 / w1;
                    let it = 1.0 - t;
                    let bw = s4 * (4.0 * t * it);
                    for ch in 0..3 {
                        let tv = c0c[ch] * it + c1c[ch] * t;
                        let bv = c2c[ch] * it + c3c[ch] * t;
                        let val = (tv * is
                            + bv * s
                            + e1[ch] * is
                            + e2[ch] * s
                            + el[ch] * it
                            + er[ch] * t
                            + bowl[ch] * bw
                            + off[ch])
                            .clamp(0.0, 1.0);
                        // 底色该通道接近 0 时直接归零：否则 (b-ch)/b 会把几级噪声
                        // 放大成 alpha=1（纯绿底 r=0、纯蓝底 r/g=0 的经典坑）
                        r[ch] = if val < 0.05 { 0.0 } else { val };
                    }
                }
            }
        }
        for x in 0..wu {
            let o4 = idx * 4;
            idx += 1;
            let b: [f32; 3] = if use_field { row[x] } else { b_const };
            let k = buf[o4 + 3] as f32 / 255.0;
            if k <= 0.0 {
                buf[o4] = 0;
                buf[o4 + 1] = 0;
                buf[o4 + 2] = 0;
                buf[o4 + 3] = 0;
                continue;
            }
            let c0 = buf[o4] as f32;
            let c1 = buf[o4 + 1] as f32;
            let c2 = buf[o4 + 2] as f32;
            // 与底色逐通道差都在容差内 → 过渡带系数为 0 → 结果必为全透明，直接跳过
            if ytol > 0.0
                && (c0 - b[0] * 255.0).abs().max((c1 - b[1] * 255.0).abs()).max(
                    (c2 - b[2] * 255.0).abs(),
                ) <= ytol
            {
                buf[o4] = 0;
                buf[o4 + 1] = 0;
                buf[o4 + 2] = 0;
                buf[o4 + 3] = 0;
                continue;
            }

            let mut a_cov = 0.0f32;
            if black_model {
                // 黑屏光效：亮度覆盖度，底色亮度做基准（暗底不浮灰雾）
                let mx = c0.max(c1).max(c2) / 255.0;
                let bl = b[0].max(b[1]).max(b[2]);
                a_cov = if bl < 0.999 {
                    ((mx - bl) / (1.0 - bl)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
            } else {
                for v in 0..3 {
                    let ch = if v == 0 {
                        c0 / 255.0
                    } else if v == 1 {
                        c1 / 255.0
                    } else {
                        c2 / 255.0
                    };
                    let bv = b[v];
                    // 近白处给分母加 10 级地板：底色 254 时 (255-254)/(255-254)=1，
                    // 一级噪声就能把整片白底翻成前景
                    let dev = if ch < bv {
                        (bv - ch) / bv.max(1e-6)
                    } else if ch > bv {
                        (ch - bv) / (1.0 - bv).max(0.04)
                    } else {
                        0.0
                    };
                    a_cov = a_cov.max(dev);
                }
                // 主导通道盈余比：底色纯度越高，这个量越能反映「被前景覆盖了多少」
                let m = if b[1] > b[0] {
                    if b[2] > b[1] { 2 } else { 1 }
                } else if b[2] > b[0] {
                    2
                } else {
                    0
                };
                let d = (m + 1) % 3;
                let xx = (m + 2) % 3;
                let e = b[m] - b[d].max(b[xx]);
                if e > 0.05 {
                    let cc = [c0, c1, c2];
                    let v = (cc[m] - cc[d].max(cc[xx])) / 255.0;
                    let ch_a = clamp01(1.0 - v / e);
                    if ch_a > a_cov {
                        if p.model == FxModel::Smoke {
                            // 烟雾/薄纱：中性前景在该项上与既有公式严格相等，
                            // 非中性薄纱按色度盈余保守抬升（原站行为）
                            a_cov = ch_a;
                        } else {
                            // 彩色光效：只对「已经接近实心」的像素做补正，
                            // 平滑权重避免把半透明彩色光效抬成实心
                            let t = ((a_cov - 0.35) / 0.40).clamp(0.0, 1.0);
                            let wgt = t * t * (3.0 - 2.0 * t);
                            a_cov += (ch_a - a_cov) * wgt;
                        }
                    }
                }
            }

            // 浅投影抑制：像素 = 底色等比变暗（色度不变、亮度 0.60~0.94）
            if bg_bright && a_cov > 0.0 && a_cov < 0.999 {
                let (mut kmax, mut kmin, mut ksum) = (0.0f32, 1.0f32, 0.0f32);
                let mut kn = 0;
                for v in 0..3 {
                    let bv = b[v];
                    if bv > 0.10 {
                        let cv = if v == 0 {
                            c0
                        } else if v == 1 {
                            c1
                        } else {
                            c2
                        };
                        let kk = cv / 255.0 / bv;
                        kmax = kmax.max(kk);
                        kmin = kmin.min(kk);
                        ksum += kk;
                        kn += 1;
                    }
                }
                if kn == 3 {
                    let km = ksum / 3.0;
                    let spread_tol = (0.06 + 2.0 * fld.map(|f| f.noise).unwrap_or(0.0))
                        .min(0.22);
                    if km >= 0.60 && km <= 0.94 && (kmax - kmin) < spread_tol {
                        a_cov = 0.0;
                    }
                }
            }

            // 黑点/白点归一 + 浓度指数
            let mut alpha = ((a_cov - r_lo) * span_inv).clamp(0.0, 1.0);
            if !no_pow && alpha > 0.0 {
                alpha = alpha.powf(inv_u);
            }
            // 底色容差（绝对通道差，带同宽过渡带）
            if ytol > 0.0 {
                let l_dev = (c0 - b[0] * 255.0)
                    .abs()
                    .max((c1 - b[1] * 255.0).abs())
                    .max((c2 - b[2] * 255.0).abs());
                alpha *= clamp01((l_dev - ytol) * ytol_inv);
            }
            alpha *= k;
            if a_cov <= 0.0 || alpha < 0.00196 {
                buf[o4] = 0;
                buf[o4 + 1] = 0;
                buf[o4 + 2] = 0;
                buf[o4 + 3] = 0;
                continue;
            }

            // 颜色分离（un-premultiply）
            let inv_a = 1.0 / a_cov;
            let mut fo = [0u8; 3];
            for v in 0..3 {
                let cv = if v == 0 {
                    c0
                } else if v == 1 {
                    c1
                } else {
                    c2
                } / 255.0;
                fo[v] = (255.0 * clamp01((cv - (1.0 - a_cov) * b[v]) * inv_a))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            // 溢色抑制：主导通道超出非主导最大值的部分 × despill × (1-A)
            if desp > 0.0 {
                let s_min = b[0].min(b[1]).min(b[2]);
                let s_max = b[0].max(b[1]).max(b[2]);
                if s_max - s_min > 0.05 {
                    let mut dmask = 0u8;
                    for v in 0..3 {
                        if (b[v] - s_min) / (s_max - s_min) > 0.65 {
                            dmask |= 1 << v;
                        }
                    }
                    let ndom = dmask.count_ones();
                    if ndom > 0 && ndom < 3 {
                        let mut other_max = 0u8;
                        let mut dom_min = 255u8;
                        for v in 0..3 {
                            if dmask & (1 << v) != 0 {
                                dom_min = dom_min.min(fo[v]);
                            } else {
                                other_max = other_max.max(fo[v]);
                            }
                        }
                        let excess = dom_min.saturating_sub(other_max);
                        if excess > 0 {
                            let cut = (excess as f32 * desp * (1.0 - a_cov))
                                .round()
                                .clamp(0.0, 255.0) as u8;
                            if cut > 0 {
                                for v in 0..3 {
                                    if dmask & (1 << v) != 0 {
                                        fo[v] = fo[v].saturating_sub(cut);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            buf[o4] = fo[0];
            buf[o4 + 1] = fo[1];
            buf[o4 + 2] = fo[2];
            buf[o4 + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

// ---------------------------------------------------------------------------
// 测试：与原 WASM 版已验证用例对齐
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 简单构造纯色图
    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h { v.extend_from_slice(&rgba); }
        v
    }

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * w + x) * 4) as usize;
        [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
    }

    #[test]
    fn green_screen_removes_green_and_suppresses_spill() {
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
    fn blue_screen_removes_blue() {
        let mut buf = vec![
            0, 0, 255, 255,
            255, 0, 0, 255,
        ];
        chroma_key(&mut buf, 2, 1, 1, 35, 25, 60, [0, 0, 255], [0, 0, 0]).unwrap();
        assert_eq!(px(&buf, 2, 0, 0)[3], 0, "纯蓝应全透明");
        assert_eq!(px(&buf, 2, 1, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn white_background_flood_from_edges() {
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
    fn corner_flood_keeps_isolated_same_color() {
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
    fn dual_color_key_uses_nearest_of_two_keys() {
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
    fn dual_bg_exact_alpha_and_color_recovery() {
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
    fn dual_bg_removes_tiny_fragments() {
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
    fn dual_bg_erode_shaves_border() {
        // 5x5 全不透明，erode=25 → 14 层削边：边界削到 0，内圈被削 2 层（211），中心保留
        let mut black = solid(5, 5, [180, 60, 40, 255]);
        let white = solid(5, 5, [180, 60, 40, 255]);
        double_bg(&mut black, &white, 5, 5, 100, 0, 25).unwrap();
        assert_eq!(px(&black, 5, 0, 0)[3], 0, "角点应被削边削到透明");
        assert_eq!(px(&black, 5, 1, 1)[3], 211, "内圈被削 2 层（每层 22）");
        assert_eq!(px(&black, 5, 2, 2)[3], 255, "中心不受削边影响");
    }

    #[test]
    fn reject_bad_args() {
        let mut buf = vec![0u8; 4];
        assert!(chroma_key(&mut buf, 0, 0, 0, 0, 0, 0, [0; 3], [0; 3]).is_err());
        assert!(chroma_key(&mut buf, 4, 4, 0, 0, 0, 0, [0; 3], [0; 3]).is_err(), "缓冲区过小");
        let b = vec![0u8; 16];
        let mut c = vec![0u8; 16];
        assert!(double_bg(&mut c, &b, 0, 1, 0, 0, 0).is_err());
    }

    fn fx_params(model: FxModel, bg: FxBgType) -> FxParams {
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
    fn fx_green_bg_pixel_fully_transparent() {
        // 纯底色像素：逐通道偏离为 0 → 全透明
        let mut buf = vec![0, 255, 0, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Color, FxBgType::Green));
        assert_eq!(buf, vec![0, 0, 0, 0]);
    }

    #[test]
    fn fx_smoke_neutral_gray_on_green_kept_opaque() {
        // 绿幕上的中性烟雾：烟雾模型覆盖度=1，灰色完整保留
        let mut buf = vec![128, 128, 128, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Smoke, FxBgType::Green));
        assert_eq!(buf, vec![128, 128, 128, 255]);
    }

    #[test]
    fn fx_smoke_greenish_mix_separates_to_white() {
        // 绿幕 75% + 前景 25% 的混合像素 → 25% 不透明白色（un-premultiply 分离）
        let mut buf = vec![64, 255, 64, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Smoke, FxBgType::Green));
        assert_eq!(buf[3], 62, "alpha = (A-r)/(1-r) ≈ 0.243");
        assert_eq!([buf[0], buf[1], buf[2]], [255, 255, 255]);
    }

    #[test]
    fn fx_white_bg_separates_pale_purple() {
        // 白底淡紫 (200,180,220)：分离背景混色 → 饱和紫 (68,0,136) @ ~29%
        let mut buf = vec![200, 180, 220, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Color, FxBgType::White));
        assert_eq!(buf[3], 73, "覆盖度 A = max 通道偏离 ≈ 0.294");
        assert_eq!([buf[0], buf[1], buf[2]], [68, 0, 136]);
    }

    #[test]
    fn fx_white_bg_pure_white_transparent() {
        let mut buf = vec![255, 255, 255, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Color, FxBgType::White));
        assert_eq!(buf, vec![0, 0, 0, 0]);
    }

    #[test]
    fn fx_black_glow_compensates_color() {
        // 黑屏光效：亮度=覆盖度，颜色反解补偿 (60,40,80) → (191,128,255) @ 80
        let mut buf = vec![60, 40, 80, 255];
        fx_matte(&mut buf, 1, 1, &fx_params(FxModel::Black, FxBgType::Black));
        assert_eq!(buf[3], 80);
        assert_eq!([buf[0], buf[1], buf[2]], [191, 128, 255]);
    }

    #[test]
    fn fx_despill_trims_dominant_excess_on_edges() {
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
    fn fx_auto_bg_samples_green_screen() {
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
    fn erode_alpha_expands_hole() {
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
    fn smooth_alpha_blurs_edge() {
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

    fn syn_px(bg: [f32; 3], fg: [u8; 3], a: f32) -> [u8; 3] {
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
    fn synth_green_smoke() -> Synth {
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
    fn synth_gradient() -> Synth {
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
    fn synth_white_shadow() -> Synth {
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
    fn synth_press_edge() -> Synth {
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
    fn synth_black_glow() -> Synth {
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
    fn synth_green_smoke_alpha_accurate_and_solid_kept() {
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
    fn synth_gradient_bg_field_accurate() {
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
    fn synth_white_shadow_suppressed() {
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
    fn synth_subject_on_border_does_not_pollute_bg() {
        let s = synth_press_edge();
        let (out, det) = s.run_auto();
        let (c, _) = det.expect("主体压边时仍应识别出底色");
        let err = (c[0] as i32).abs() + (c[1] as i32 - 177).abs() + (c[2] as i32 - 64).abs();
        println!("[D] 识别={:?} 三通道总误差={} 底漏={:.4}", c, err, s.mean_alpha(&out, 0));
        assert!(err <= 18, "压边主体不得污染底色，识别 {:?}", c);
        assert!(s.mean_alpha(&out, 0) < 0.02, "剩余底色区应干净");
    }

    #[test]
    fn synth_black_glow_no_fog() {
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
    fn detect_rejects_two_tone_background() {
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
    fn detect_handles_vignette_and_noise() {
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
    fn fx_flat_background_keeps_legacy_exact_values() {
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
    fn bench_2mp_preview_and_detect() {
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
    fn real_sample_green_screen_acceptance() {
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
    fn chroma_accepts_custom_key_color() {
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
