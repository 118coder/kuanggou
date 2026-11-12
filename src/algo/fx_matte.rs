//! 半透明抠底主算法：逐点底色相对反解 + trimap 式投影剔除 + 溢色抑制，
//! 覆盖彩色光效 / 烟雾薄纱 / 黑屏光效三种模型。

#![allow(unused_imports)]
use crate::algo::Img;
use crate::algo::chroma::*;
use crate::algo::dual_bg::*;
use crate::algo::fx::*;
use crate::algo::bg_field::*;
use crate::algo::unmult::*;


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
