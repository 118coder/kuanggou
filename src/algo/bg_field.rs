//! 智能底色识别：边框分段采样 → 二次曲面「底色场」拟合 → 逐点底色反解
//! 的预处理；同时给出自动识别底色与推荐模型。

#![allow(unused_imports)]
use crate::algo::Img;
use crate::algo::chroma::*;
use crate::algo::dual_bg::*;
use crate::algo::fx::*;
use crate::algo::fx_matte::*;
use crate::algo::unmult::*;


#[derive(Clone, Copy)]
pub(crate) struct SegSample {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) u: f32,
    pub(crate) c: [f32; 3],
    pub(crate) w: f32,
    pub(crate) side: u8, // 0 上 1 下 2 左 3 右
}

#[inline]
pub(crate) fn max_dev(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())
}

#[inline]
pub(crate) fn to_u8(c: [f32; 3]) -> [u8; 3] {
    [
        (c[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[2] * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// 中位数（就地排序；空切片返回 0）
pub(crate) fn median_f32(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let m = v.len() / 2;
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[m]
}

/// 3x3 系数矩阵 + 3 组右端项 → 高斯-约当消元（带主元），返回 [基函数][通道]
pub(crate) fn solve3(m: [[f64; 3]; 3], r: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
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
pub(crate) fn fit_basis<F>(pts: &[(f32, f32, f32, [f32; 3])], phi: F) -> Option<[[f64; 3]; 3]>
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
pub(crate) fn coef_at(c: &[[f64; 3]; 3], p: [f64; 3]) -> [f32; 3] {
    [
        (c[0][0] * p[0] + c[1][0] * p[1] + c[2][0] * p[2]) as f32,
        (c[0][1] * p[0] + c[1][1] * p[1] + c[2][1] * p[2]) as f32,
        (c[0][2] * p[0] + c[1][2] * p[1] + c[2][2] * p[2]) as f32,
    ]
}

/// 边框环分段采样：每边 SEGS 段，每段取逐通道中位色（跳过透明像素）
pub(crate) fn sample_border(buf: &[u8], w: usize, h: usize) -> Vec<SegSample> {
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
pub(crate) struct BgField {
    pub(crate) w: usize,
    pub(crate) h: usize,
    /// 四角色值：0=左上 1=右上 2=左下 3=右下
    pub(crate) c: [[f32; 3]; 4],
    /// 四条边相对「角点连线」的曲率修正
    pub(crate) et: Vec<[f32; 3]>,
    pub(crate) eb: Vec<[f32; 3]>,
    pub(crate) el: Vec<[f32; 3]>,
    pub(crate) er: Vec<[f32; 3]>,
    /// 内部「碗形」项（四条边中点曲率的平均，专治径向暗角）
    pub(crate) bowl: [f32; 3],
    pub(crate) mean: [f32; 3],
    /// 场的空间变化幅度（0..1）
    pub(crate) spread: f32,
    /// 置信度：场能解释边框采样的加权比例
    pub(crate) conf: f32,
    /// 边框相对场的中位残差（≈ 噪声量级，0..1）
    pub(crate) noise: f32,
    /// 连通有效边占边框的权重比
    pub(crate) core: f32,
    /// 「一条边被完整解释 + 其余边是主体」也算可信（主体压边场景）
    pub(crate) edge_ok: bool,
    /// 边框横跨两种底色（某条边两端都贴合场、中段明显背离）
    pub(crate) dual: bool,
    pub(crate) valid: [bool; 4],
}

impl BgField {
    pub(crate) fn estimate(buf: &[u8], w: usize, h: usize) -> Option<BgField> {
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
    pub(crate) fn raw_at(&self, x: usize, y: usize) -> [f32; 3] {
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
    pub(crate) fn reliable(&self) -> bool {
        !self.dual
            && self.valid.iter().any(|v| *v)
            && (self.conf >= 0.55 || self.edge_ok)
    }
}

/// 底色像素分类（原站 Ut）：判断单像素是否"像"指定类型的底色
pub(crate) fn looks_like_bg(c: [u8; 3], bg: FxBgType) -> bool {
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
pub(crate) fn bg_type_plausible(c: [f32; 3], bg: FxBgType) -> bool {
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
pub(crate) fn typed_border_color(segs: &[SegSample], bg: FxBgType) -> Option<[u8; 3]> {
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
pub(crate) fn legacy_auto_bg_color(img: &Img, bg: FxBgType) -> Option<[u8; 3]> {
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
pub(crate) fn legacy_detect_bg_auto(img: &Img) -> Option<([u8; 3], FxModel)> {
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
