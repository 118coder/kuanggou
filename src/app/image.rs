//! 图片工作区：载入、预览代理、算法调度、全分辨率导出。
//!
//! 由 1.1.1 单文件版机械拆分而来；行为保持不变。

#![allow(unused_imports)]
use eframe::egui;
use egui::{Align2, Color32, ColorImage, CornerRadius, FontId, Margin, Pos2, Rect, Sense,
           Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2, pos2, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use crate::algo;
use crate::algo::*;
use crate::app::*;
use crate::ui;
use crate::ui::*;
use crate::ui::theme::*;
use crate::ui::util::*;
use crate::ui::widgets::*;
use crate::ui::command_bar::*;
use crate::ui::stage::*;
use crate::update::*;
use crate::video::*;


impl App {
    pub(crate) fn set_main(&mut self, img: Img) {
        // 预览代理：>2.2MP 降到长边 1600（约 1.7MP）；交互代理：>0.55MP 降到长边 760
        let mp = img.megapixels();
        self.proxy_main = if mp > 2.2 { Some(img.downscale_to_long(1600)) } else { None };
        self.fast_main = if mp > 0.55 {
            Some(match self.proxy_main.as_ref() {
                Some(p) => p.downscale_to_long(760),
                None => img.downscale_to_long(760),
            })
        } else {
            None
        };
        self.fast_done = false;
        self.pending_fast = true;
        self.orig = Some(img);
        self.result = None;
        self.tex_result = None;
        self.tex_main = None; // 在 ui 阶段用代理重建
        self.auto_bg = None;
        self.auto_tried = false;
        self.view_scale = 1.0;
        self.view_offset = Vec2::ZERO;
        self.dirty = true;
    }


    pub(crate) fn set_slot(&mut self, slot: Slot, img: Img) -> Result<(), String> {
        let other = match slot {
            Slot::Black => &self.white,
            Slot::White => &self.black,
        };
        if let Some(o) = other {
            if o.w != img.w || o.h != img.h {
                return Err("两张图尺寸不一致".into());
            }
        }
        match slot {
            Slot::Black => {
                let mp = img.megapixels();
                self.proxy_black = if mp > 2.2 { Some(img.downscale_to_long(1600)) } else { None };
                self.fast_black = if mp > 0.55 {
                    Some(match self.proxy_black.as_ref() {
                        Some(p) => p.downscale_to_long(760),
                        None => img.downscale_to_long(760),
                    })
                } else {
                    None
                };
                self.black = Some(img);
                self.tex_black = None;
            }
            Slot::White => {
                let mp = img.megapixels();
                self.proxy_white = if mp > 2.2 { Some(img.downscale_to_long(1600)) } else { None };
                self.fast_white = if mp > 0.55 {
                    Some(match self.proxy_white.as_ref() {
                        Some(p) => p.downscale_to_long(760),
                        None => img.downscale_to_long(760),
                    })
                } else {
                    None
                };
                self.white = Some(img);
                self.tex_white = None;
            }
        }
        self.dirty = true;
        Ok(())
    }


    pub(crate) fn clear_all(&mut self) {
        self.orig = None;
        self.black = None;
        self.white = None;
        self.proxy_main = None;
        self.proxy_black = None;
        self.proxy_white = None;
        self.fast_main = None;
        self.fast_black = None;
        self.fast_white = None;
        self.fast_done = false;
        self.result = None;
        self.tex_main = None;
        self.tex_black = None;
        self.tex_white = None;
        self.tex_result = None;
        self.auto_bg = None;
        self.view_scale = 1.0;
        self.view_offset = Vec2::ZERO;
        self.dirty = true;
    }


    pub(crate) fn fx_reset_params(&mut self) {
        // 原站 Xe(bg)：按底色类型复位参数，保留当前模型（黑底强制 Black）
        let preset = fx_preset_for(self.fx_bg_type);
        self.fx_ctol = preset.color_tolerance;
        self.fx_bpoint = preset.black_point;
        self.fx_white = preset.white_point;
        self.fx_density = preset.density;
        self.fx_despill = preset.despill;
        if preset.model == FxModel::Black {
            self.fx_model = FxModel::Black;
        }
        self.dirty = true;
    }

    /// 底色预设：自动取样 + 参数复位（原站 j(u)）

    pub(crate) fn fx_apply_bg(&mut self, bg: FxBgType) {
        let src = self.proxy_main.as_ref().or(self.orig.as_ref());
        let sampled = src.and_then(|m| auto_bg_color(m, bg));
        self.fx_bg_type = bg;
        self.fx_bg = match sampled {
            Some(c) => Color32::from_rgb(c[0], c[1], c[2]),
            None => Color32::from_rgb(bg.pure_color()[0], bg.pure_color()[1], bg.pure_color()[2]),
        };
        let preset = fx_preset_for(bg);
        self.fx_model = preset.model;
        self.fx_ctol = preset.color_tolerance;
        self.fx_bpoint = preset.black_point;
        self.fx_white = preset.white_point;
        self.fx_density = preset.density;
        self.fx_despill = preset.despill;
        self.dirty = true;
        let [r, g, b] = match sampled {
            Some(c) => c,
            None => bg.pure_color(),
        };
        self.toast(
            format!("{} · 已取样底色 #{r:02X}{g:02X}{b:02X}", bg.label()),
            false,
        );
    }


    pub(crate) fn run_chroma_on(&self, buf: &mut [u8], w: u32, h: u32) -> Result<(), String> {
        let c1 = rgb_of(match self.mode {
            Mode::Green => self.key_green,
            Mode::Blue => self.key_blue,
            _ => self.c1,
        });
        let c2 = rgb_of(self.c2);
        // 显示量纲 → 算法量纲：容差 0..150 → 0..100，羽化 0..50 → 0..100
        let tol_i = ((self.tol * 2.0 / 3.0) as u32).clamp(0, 100);
        let fea_i = ((self.feather * 2.0) as u32).clamp(0, 100);
        algo::chroma_key(buf, w, h, self.mode.id(), tol_i, fea_i, self.suppress as u32, c1, c2)?;
        algo::erode_alpha(buf, w, h, self.ck_erode);
        algo::smooth_alpha(buf, w, h, self.ck_smooth);
        Ok(())
    }


    pub(crate) fn run_fx_on(&self, buf: &mut [u8], w: u32, h: u32) -> Result<(), String> {
        let bg: [u8; 3] = if self.mode == Mode::Auto {
            self.auto_bg.map(|c| rgb_of(c)).unwrap_or([255, 255, 255])
        } else {
            rgb_of(self.fx_bg)
        };
        let params = FxParams {
            model: self.fx_model,
            bg,
            color_tolerance: self.fx_ctol,
            black_point: self.fx_bpoint,
            white_point: self.fx_white,
            density: self.fx_density,
            despill: self.fx_despill,
        };
        algo::fx_matte(buf, w, h, &params);
        Ok(())
    }


    /// Unmult 去黑底（参考 AE 的 Unmult 插件：亮度反解 alpha + 反预乘还原颜色）
    pub(crate) fn run_unmult_on(&self, buf: &mut [u8], w: u32, h: u32) -> Result<(), String> {
        let p = algo::UnmultParams {
            black_point: self.un_black,
            white_point: self.un_white,
            density: self.un_density,
            noise_floor: self.un_floor,
            gain_limit: self.un_gain,
            use_luma: self.un_luma,
        };
        algo::unmult_black(buf, w, h, &p);
        algo::erode_alpha(buf, w, h, self.ck_erode);
        algo::smooth_alpha(buf, w, h, self.ck_smooth);
        Ok(())
    }

    pub(crate) fn run_dualbg_on(&self, blk: &mut [u8], wht: &[u8], w: u32, h: u32) -> Result<(), String> {
        algo::double_bg(blk, wht, w, h, self.db_tol as u32, self.db_edge as u32, self.db_erode as u32)
    }

    // ---------------- 处理 ----------------


    pub(crate) fn refresh(&mut self, ctx: &egui::Context, fast: bool) {
        self.dirty = false;
        self.last_run = Instant::now();
        // 自动识别只跑一次，必须用完整代理，不能吃快代理
        let auto_pending = self.mode == Mode::Auto && self.auto_bg.is_none() && !self.auto_tried;
        let fast = fast && !auto_pending;
        self.pending_fast = false;
        let t0 = Instant::now();

        match self.mode {
            Mode::DualBg => {
                let src_b = if fast { self.fast_black.as_ref() } else { None }
                    .or(self.proxy_black.as_ref())
                    .or(self.black.as_ref());
                let src_w = if fast { self.fast_white.as_ref() } else { None }
                    .or(self.proxy_white.as_ref())
                    .or(self.white.as_ref());
                let (Some(b), Some(w)) = (src_b, src_w) else {
                    self.result = None;
                    self.tex_result = None;
                    return;
                };
                if b.w != w.w || b.h != w.h {
                    self.status = "黑底图与白底图尺寸不一致，无法差分".into();
                    self.result = None;
                    return;
                }
                let mut out = b.buf.clone();
                if let Err(e) = self.run_dualbg_on(&mut out, &w.buf, b.w as u32, b.h as u32) {
                    self.status = e;
                    self.result = None;
                    return;
                }
                self.result = Some(Img::from_raw(b.w, b.h, out));
            }
            Mode::Fx | Mode::Auto => {
                // 智能抠底：首次处理时从边框环识别底色与推荐模型
                if self.mode == Mode::Auto && self.auto_bg.is_none() && !self.auto_tried {
                    self.auto_tried = true; // 只识别一次，避免每次刷新重采样
                    if let Some(src) = self.proxy_main.as_ref().or(self.orig.as_ref()) {
                        let probe = src.clone();
                        if let Some((c, model)) = algo::detect_bg_auto(&probe) {
                            self.auto_bg = Some(Color32::from_rgb(c[0], c[1], c[2]));
                            self.fx_model = model;
                            let [r, g, b] = c;
                            self.status = format!("已识别底色 #{r:02X}{g:02X}{b:02X}");
                        } else {
                            self.status = "未识别到纯色底，可改用色键模式或点击原图取色".into();
                        }
                    }
                }
                let src = if fast { self.fast_main.as_ref() } else { None }
                    .or(self.proxy_main.as_ref())
                    .or(self.orig.as_ref());
                let Some(o) = src else {
                    self.result = None;
                    self.tex_result = None;
                    return;
                };
                let mut buf = o.buf.clone();
                if let Err(e) = self.run_fx_on(&mut buf, o.w as u32, o.h as u32) {
                    self.status = e;
                    self.result = None;
                    return;
                }
                self.result = Some(Img::from_raw(o.w, o.h, buf));
            }
            _ => {
                let src = if fast { self.fast_main.as_ref() } else { None }
                    .or(self.proxy_main.as_ref())
                    .or(self.orig.as_ref());
                let Some(o) = src else {
                    self.result = None;
                    self.tex_result = None;
                    return;
                };
                let mut buf = o.buf.clone();
                let run = if self.mode == Mode::Unmult {
                    self.run_unmult_on(&mut buf, o.w as u32, o.h as u32)
                } else {
                    self.run_chroma_on(&mut buf, o.w as u32, o.h as u32)
                };
                if let Err(e) = run {
                    self.status = e;
                    self.result = None;
                    return;
                }
                self.result = Some(Img::from_raw(o.w, o.h, buf));
            }
        }

        self.last_ms = t0.elapsed().as_secs_f32() * 1000.0;
        if self.status.trim().is_empty() {
            self.status = "参数已应用".into();
        }
        self.fast_done = fast;
        self.update_result_tex(ctx);
    }


    pub(crate) fn update_result_tex(&mut self, ctx: &egui::Context) {
        let Some(res) = &self.result else {
            self.tex_result = None;
            return;
        };
        let ci = image_to_color_image(res, self.view_mask);
        let opts = if self.view_scale >= 3.0 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
        match &mut self.tex_result {
            Some(h) => h.set(ci, opts),
            None => self.tex_result = Some(ctx.load_texture("result", ci, opts)),
        }
    }

    // ---------------- 导出 / 复制 / 定位 ----------------

    /// 全分辨率处理（导出用），返回 (w, h, rgba)

    pub(crate) fn export_to_buffer(&self) -> Result<(usize, usize, Vec<u8>), String> {
        match self.mode {
            Mode::DualBg => {
                let (Some(b), Some(w)) = (&self.black, &self.white) else {
                    return Err("双背景差分需要黑底和白底两张图".into());
                };
                if b.w != w.w || b.h != w.h {
                    return Err("黑底图与白底图尺寸不一致".into());
                }
                let mut out = b.buf.clone();
                self.run_dualbg_on(&mut out, &w.buf, b.w as u32, b.h as u32)?;
                Ok((b.w, b.h, out))
            }
            Mode::Fx | Mode::Auto => {
                let o = self.orig.as_ref().ok_or("请先载入原图")?;
                let mut buf = o.buf.clone();
                self.run_fx_on(&mut buf, o.w as u32, o.h as u32)?;
                Ok((o.w, o.h, buf))
            }
            _ => {
                let o = self.orig.as_ref().ok_or("请先载入原图")?;
                let mut buf = o.buf.clone();
                if self.mode == Mode::Unmult {
                    self.run_unmult_on(&mut buf, o.w as u32, o.h as u32)?;
                } else {
                    self.run_chroma_on(&mut buf, o.w as u32, o.h as u32)?;
                }
                Ok((o.w, o.h, buf))
            }
        }
    }


    pub(crate) fn export(&mut self) {
        let (w, h, buf) = match self.export_to_buffer() {
            Ok(v) => v,
            Err(e) => {
                self.toast(e, true);
                return;
            }
        };
        // 自动命名 cutout_20261112_153000.png + 记住上次导出目录
        let (y, mo, d, hh, mm, ss) = civil_now();
        let default_name = format!("cutout_{y:04}{mo:02}{d:02}_{hh:02}{mm:02}{ss:02}.png");
        let mut dialog =
            rfd::FileDialog::new().set_file_name(&default_name).add_filter("PNG 图片", &["png"]);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        let path = ensure_png_ext(path);
        match ::image::RgbaImage::from_raw(w as u32, h as u32, buf)
            .ok_or("缓冲区尺寸不符".to_string())
            .and_then(|im| im.save(&path).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.last_dir = path.parent().map(|p| p.to_path_buf());
                self.last_export = Some(path.clone());
                self.toast(format!("已导出 {}", file_name_of(&path)), false);
                self.status = format!("已导出 {}", path.display());
            }
            Err(e) => {
                self.toast(format!("导出失败：{e}"), true);
                self.status = format!("导出失败：{e}");
            }
        }
    }


    pub(crate) fn copy_result(&mut self) {
        let Some(res) = &self.result else {
            self.toast("还没有结果可复制", true);
            return;
        };
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                let data = arboard::ImageData {
                    width: res.w,
                    height: res.h,
                    bytes: res.buf.clone().into(),
                };
                match cb.set_image(data) {
                    Ok(()) => self.toast(format!("结果已复制到剪贴板（{}×{}）", res.w, res.h), false),
                    Err(e) => self.toast(format!("复制失败：{e}"), true),
                }
            }
            Err(e) => self.toast(format!("无法访问剪贴板：{e}"), true),
        }
    }


    pub(crate) fn reveal_last_export(&mut self) {
        let Some(p) = self.last_export.clone() else { return };
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn();
    }

    // ---------------- 输入事件 ----------------

}
