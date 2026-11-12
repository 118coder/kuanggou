//! 图片预览舞台：画布、对比分割线、缩放平移、取色、双底槽位、空状态。

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
use crate::update::*;
use crate::video::*;


pub(crate) struct StageStat {
    pub(crate) hover: Option<String>,
    pub(crate) zoom: f32,
    pub(crate) fit: Vec2,
    pub(crate) img_w: usize,
    pub(crate) canvas: Rect,
    pub(crate) has_img: bool,
}


impl App {
    pub(crate) fn stage(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, C_PANEL);
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
        let inner = rect.shrink(1.0);
        let two_row = inner.width() < 760.0 && self.result.is_some();
        let head_h = if two_row { STAGE_HEAD + 32.0 } else { STAGE_HEAD };
        let head = Rect::from_min_size(inner.min, vec2(inner.width(), head_h));
        let foot = Rect::from_min_size(pos2(inner.left(), inner.bottom() - STAGE_FOOT), vec2(inner.width(), STAGE_FOOT));
        let canvas = Rect::from_min_max(pos2(inner.left(), head.bottom()), pos2(inner.right(), foot.top()));
        p.line_segment([pos2(inner.left(), head.bottom()), pos2(inner.right(), head.bottom())], Stroke::new(1.0, C_RULE));
        p.line_segment([pos2(inner.left(), foot.top()), pos2(inner.right(), foot.top())], Stroke::new(1.0, C_RULE));
        self.stage_head(ui, head, two_row);
        let st = self.stage_canvas(ui, canvas);
        self.stage_foot(ui, foot, &st);
    }


    pub(crate) fn stage_head(&mut self, ui: &mut egui::Ui, rect: Rect, two_row: bool) {
        let mut flip = false;
        let has = self.result.is_some();
        let row1 = Rect::from_min_size(rect.min, vec2(rect.width(), STAGE_HEAD));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(row1.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                draw_tag_chip_widget(ui, if self.mode == Mode::DualBg { "01 / 差分结果" } else { "01 / 结果" });
                ui.add_space(10.0);
                let mut info = match &self.result {
                    Some(r) => format!("{} × {}", r.w, r.h),
                    None => "— × —".to_string(),
                };
                if self.mode != Mode::DualBg {
                    if let Some(o) = self.orig.as_ref() {
                        info = format!("{info}   │   {:.2} MP", o.w as f32 * o.h as f32 / 1_000_000.0);
                    }
                }
                ui.label(egui::RichText::new(info).size(12.0).color(C_INK2));
                if has && !two_row {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.ctl_compare(ui).clicked() {
                            flip = true;
                        }
                        ui.add_space(12.0);
                        rule_v(ui);
                        ui.add_space(12.0);
                        self.ctl_views(ui);
                        ui.add_space(12.0);
                        rule_v(ui);
                        ui.add_space(12.0);
                        self.ctl_backdrop(ui);
                    });
                }
            },
        );
        if two_row {
            let row2 = Rect::from_min_max(pos2(rect.left(), row1.bottom()), rect.max);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(row2.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    if self.ctl_compare(ui).clicked() {
                        flip = true;
                    }
                    ui.add_space(12.0);
                    rule_v(ui);
                    ui.add_space(12.0);
                    self.ctl_views(ui);
                    ui.add_space(12.0);
                    rule_v(ui);
                    ui.add_space(12.0);
                    self.ctl_backdrop(ui);
                },
            );
        }
        if flip {
            self.compare = !self.compare;
        }
    }


    pub(crate) fn ctl_compare(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let can = self.orig.is_some() && self.mode != Mode::DualBg && self.result.is_some();
        check_chip(ui, "对比", self.compare && can)
    }


    pub(crate) fn ctl_views(&mut self, ui: &mut egui::Ui) {
        if seg_block(ui, "蒙版", self.view_mask, vec2(44.0, 24.0)).clicked() {
            self.view_mask = true;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "彩色", !self.view_mask, vec2(44.0, 24.0)).clicked() {
            self.view_mask = false;
            self.update_result_tex(ui.ctx());
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("视图").size(10.0).color(C_INK2));
    }


    pub(crate) fn ctl_backdrop(&mut self, ui: &mut egui::Ui) {
        let mut custom = self.preview_custom;
        let changed = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size = vec2(24.0, 24.0);
                ui.color_edit_button_srgba(&mut custom)
            })
            .inner;
        if ui.ctx().cumulative_pass_nr() > 4 && changed.changed() {
            self.preview_custom = custom;
            self.preview_bg = PreviewBg::Custom;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "黑", self.preview_bg == PreviewBg::Black, vec2(30.0, 24.0)).clicked() {
            self.preview_bg = PreviewBg::Black;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "白", self.preview_bg == PreviewBg::White, vec2(30.0, 24.0)).clicked() {
            self.preview_bg = PreviewBg::White;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "棋盘", self.preview_bg == PreviewBg::Checker, vec2(44.0, 24.0)).clicked() {
            self.preview_bg = PreviewBg::Checker;
            self.update_result_tex(ui.ctx());
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("底色").size(10.0).color(C_INK2));
    }

    /// 绿/蓝幕：图片外圈颜色与当前键色的平均色差（越大越可能取错色）

    pub(crate) fn key_color_gap(&self) -> Option<f32> {
        let img = self.proxy_main.as_ref().or(self.orig.as_ref())?;
        let key = match self.mode {
            Mode::Green => self.key_green,
            Mode::Blue => self.key_blue,
            _ => return None,
        };
        let (w, h) = (img.w, img.h);
        if w < 16 || h < 16 {
            return None;
        }
        let (kr, kg, kb) = (key.r() as f32, key.g() as f32, key.b() as f32);
        let spots = [
            (0, 0),
            (w / 2, 0),
            (w - 8, 0),
            (0, h / 2),
            (w - 8, h / 2),
            (0, h - 8),
            (w / 2, h - 8),
            (w - 8, h - 8),
        ];
        let mut sum = 0.0f32;
        let mut n = 0.0f32;
        for (sx, sy) in spots {
            for y in sy..(sy + 8).min(h) {
                for x in sx..(sx + 8).min(w) {
                    let o = (y * w + x) * 4;
                    let dr = img.buf[o] as f32 - kr;
                    let dg = img.buf[o + 1] as f32 - kg;
                    let db = img.buf[o + 2] as f32 - kb;
                    sum += (dr * dr + dg * dg + db * db).sqrt();
                    n += 1.0;
                }
            }
        }
        Some(sum / n.max(1.0))
    }


    pub(crate) fn stage_foot(&mut self, ui: &mut egui::Ui, rect: Rect, st: &StageStat) {
        let mismatch = self.key_color_gap().map(|g| g > 55.0).unwrap_or(false);
        let hint = if !st.has_img {
            String::new()
        } else if mismatch && st.hover.is_none() && !self.peek {
            if self.mode == Mode::Green {
                "背景没去干净？点一下画布里的绿幕重新取色".to_string()
            } else {
                "背景没去干净？点一下画布里的蓝幕重新取色".to_string()
            }
        } else if let Some(h) = &st.hover {
            h.clone()
        } else if self.peek {
            "松开空格返回结果".to_string()
        } else if self.compare && self.orig.is_some() {
            "拖动分割线对比原图 · 空格速览".to_string()
        } else if matches!(
            self.mode,
            Mode::Green | Mode::Blue | Mode::Custom | Mode::Dual | Mode::Fx | Mode::Auto
        ) {
            "点击画布取色 · 滚轮缩放 · 双击 1:1 · 空格速览原图".to_string()
        } else {
            "滚轮缩放 · 拖动平移 · 双击 1:1 · 空格速览原图".to_string()
        };
        let tx = if mismatch && st.has_img {
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - 6.0), vec2(3.0, 12.0)),
                0.0,
                C_SIGNAL,
            );
            rect.left() + 25.0
        } else {
            rect.left() + 14.0
        };
        ui.painter().text(
            pos2(tx, rect.center().y),
            Align2::LEFT_CENTER,
            hint,
            FontId::proportional(10.5),
            if mismatch { C_INK } else { C_INK2 },
        );
        let fit = st.fit;
        let canvas = st.canvas;
        let img_w = st.img_w;
        let zoom = st.zoom;
        if !st.has_img {
            return;
        }
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 2.0))).layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                if cut_block(ui, "1:1", false, vec2(40.0, 22.0), 6.0).clicked() && fit.x > 0.0 && img_w > 0 {
                    self.view_scale = (img_w as f32 / fit.x).clamp(0.3, 16.0);
                    self.view_offset = Vec2::ZERO;
                }
                if cut_block(ui, "适应", false, vec2(46.0, 22.0), 6.0).clicked() {
                    self.view_scale = 1.0;
                    self.view_offset = Vec2::ZERO;
                }
                if cut_block(ui, "+", false, vec2(28.0, 22.0), 6.0).clicked() {
                    self.view_scale = (self.view_scale * 1.25).clamp(0.3, 16.0);
                    self.clamp_offset(fit * self.view_scale, canvas);
                }
                let zl = format!("{:.0}%", zoom);
                if cut_block(ui, &zl, false, vec2(52.0, 22.0), 6.0).clicked() {
                    self.view_scale = 1.0;
                    self.view_offset = Vec2::ZERO;
                }
                if cut_block(ui, "−", false, vec2(28.0, 22.0), 6.0).clicked() {
                    self.view_scale = (self.view_scale / 1.25).clamp(0.3, 16.0);
                    self.clamp_offset(fit * self.view_scale, canvas);
                }
            },
        );
    }


    pub(crate) fn stage_canvas(&mut self, ui: &mut egui::Ui, rect: Rect) -> StageStat {
        let p = ui.painter().clone().with_clip_rect(rect);
        p.rect_filled(rect, 0.0, C_PAPER);

        if self.orig.is_some() && self.tex_main.is_none() {
            let src = self.proxy_main.as_ref().or(self.orig.as_ref()).unwrap().clone();
            update_tex(ui.ctx(), &mut self.tex_main, "main", &src);
        }
        if self.mode == Mode::DualBg {
            if let Some(b) = self.proxy_black.as_ref().or(self.black.as_ref()) {
                if self.tex_black.is_none() {
                    update_tex(ui.ctx(), &mut self.tex_black, "black", b);
                }
            }
            if let Some(w) = self.proxy_white.as_ref().or(self.white.as_ref()) {
                if self.tex_white.is_none() {
                    update_tex(ui.ctx(), &mut self.tex_white, "white", w);
                }
            }
        }

        if self.mode == Mode::DualBg && self.result.is_none() {
            self.stage_dual_slots(ui, rect);
            return StageStat { hover: None, zoom: 100.0, fit: Vec2::ZERO, img_w: 0, canvas: rect, has_img: false };
        }
        let dims = if self.mode == Mode::DualBg {
            self.result.as_ref().map(|m| (m.w, m.h))
        } else {
            self.proxy_main.as_ref().or(self.orig.as_ref()).map(|m| (m.w, m.h))
        };
        let Some((mw, mh)) = dims else {
            self.stage_empty(ui, rect);
            return StageStat { hover: None, zoom: 100.0, fit: Vec2::ZERO, img_w: 0, canvas: rect, has_img: false };
        };

        let fit = fit_size(rect.shrink(18.0).size(), mw, mh);
        let resp = ui.interact(rect, egui::Id::new("stage_canvas"), Sense::click_and_drag());
        let show_cmp = self.compare && self.orig.is_some() && self.mode != Mode::DualBg && self.result.is_some();
        let div_x = rect.left() + rect.width() * self.split_x;
        let mut block_pan = false;
        let mut div_hover = false;
        if show_cmp {
            let hit = Rect::from_min_max(pos2(div_x - 11.0, rect.top()), pos2(div_x + 11.0, rect.bottom()));
            let d = ui.interact(hit, egui::Id::new("stage_divider"), Sense::click_and_drag());
            div_hover = d.hovered() || d.dragged();
            block_pan = div_hover;
            if d.dragged() {
                if let Some(pp) = d.interact_pointer_pos() {
                    self.split_x = ((pp.x - rect.left()) / rect.width().max(1.0)).clamp(0.04, 0.96);
                }
            }
        }

        let picking = matches!(
            self.mode,
            Mode::Green | Mode::Blue | Mode::Custom | Mode::Dual | Mode::Fx | Mode::Auto
        );
        self.viewport_interaction(&resp, fit, rect, mw, !block_pan);
        if div_hover {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        } else if picking && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }

        let disp = fit * self.view_scale;
        let ir = Rect::from_center_size(rect.center() + self.view_offset, disp);
        let mut hover: Option<String> = None;
        let mut picked: Option<Color32> = None;
        let peeking = self.peek && self.tex_main.is_some();

        if peeking {
            paint_backdrop(&p, ir, self.preview_bg, self.preview_custom);
            if let Some(t) = &self.tex_main {
                p.image(t.id(), ir, uv01(), Color32::WHITE);
                p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
            }
            p.rect_filled(Rect::from_min_size(pos2(rect.left() + 14.0, rect.top() + 13.0), vec2(4.0, 16.0)), 0.0, C_SIGNAL);
            p.text(
                pos2(rect.left() + 26.0, rect.top() + 12.0),
                Align2::LEFT_TOP,
                "原图速览",
                FontId::proportional(11.5),
                C_INK,
            );
        } else if let (Some(_r), Some(t)) = (self.result.as_ref(), self.tex_result.as_ref()) {
            if self.view_mask {
                p.rect_filled(ir, 0.0, C_INK);
            } else {
                paint_backdrop(&p, ir, self.preview_bg, self.preview_custom);
            }
            p.image(t.id(), ir, uv01(), Color32::WHITE);
            p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
            if show_cmp {
                if let Some(tm) = self.tex_main.as_ref() {
                    let clip = Rect::from_min_max(rect.min, pos2(div_x, rect.max.y)).intersect(rect);
                    let pc = p.with_clip_rect(clip);
                    pc.image(tm.id(), ir, uv01(), Color32::WHITE);
                    pc.line_segment([pos2(div_x, rect.top()), pos2(div_x, rect.bottom())], Stroke::new(1.0, C_INK));
                }
                draw_tag_chip(&p, pos2(rect.left() + 14.0, rect.top() + 13.0), "原图");
                draw_tag_chip_right(&p, pos2(rect.right() - 14.0, rect.top() + 13.0), "结果");
                draw_divider_handle(&p, pos2(div_x, rect.center().y));
            }
            if self.view_mask {
                p.text(
                    pos2(rect.right() - 14.0, rect.bottom() - 14.0),
                    Align2::RIGHT_BOTTOM,
                    "蒙版中灰 = 半透明；导出始终为彩色透明 PNG",
                    FontId::proportional(10.5),
                    C_INK2,
                );
            }
        } else if let (Some(_o), Some(tm)) = (self.orig.as_ref(), self.tex_main.as_ref()) {
            paint_backdrop(&p, ir, self.preview_bg, self.preview_custom);
            p.image(tm.id(), ir, uv01(), Color32::WHITE);
            p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
            p.text(
                pos2(rect.center().x, rect.bottom() - 16.0),
                Align2::CENTER_BOTTOM,
                "原图已就绪，等待处理结果",
                FontId::proportional(11.0),
                C_INK2,
            );
        }

        // 取色放大镜
        if picking && !div_hover {
            if let Some(pp) = resp.hover_pos() {
                if rect.contains(pp) && ir.contains(pp) {
                    if let Some(m) = self.proxy_main.as_ref().or(self.orig.as_ref()) {
                        if let Some([x, y]) = pixel_at(ir, pp, m) {
                            draw_loupe(&p, rect, pp, m, [x, y]);
                        }
                    }
                }
            }
        }

        if let Some(pp) = resp.hover_pos() {
            if rect.contains(pp) && ir.contains(pp) {
                let under = if show_cmp && pp.x < div_x {
                    self.proxy_main.as_ref().or(self.orig.as_ref())
                } else {
                    self.result.as_ref()
                };
                if let Some(m) = under {
                    if let Some([x, y]) = pixel_at(ir, pp, m) {
                        let i = (y * m.w + x) * 4;
                        if i + 3 < m.buf.len() {
                            let (r, g, b, a) = (m.buf[i], m.buf[i + 1], m.buf[i + 2], m.buf[i + 3]);
                            hover = Some(format!("{x} , {y}   │   {r}, {g}, {b}, {a}   │   #{r:02X}{g:02X}{b:02X}"));
                        }
                    }
                }
            }
        }

        if picking && resp.clicked() && !block_pan {
            if let Some(pp) = resp.interact_pointer_pos() {
                if ir.contains(pp) {
                    if let Some(m) = self.proxy_main.as_ref().or(self.orig.as_ref()) {
                        if let Some([x, y]) = pixel_at(ir, pp, m) {
                            let i = (y * m.w + x) * 4;
                            if i + 2 < m.buf.len() {
                                picked = Some(Color32::from_rgb(m.buf[i], m.buf[i + 1], m.buf[i + 2]));
                            }
                        }
                    }
                }
            }
        }
        if let Some(c) = picked {
            self.push_undo();
            match self.mode {
                Mode::Fx => self.fx_bg = c,
                Mode::Auto => {
                    self.auto_bg = Some(c);
                    self.auto_tried = true;
                }
                Mode::Green => self.key_green = c,
                Mode::Blue => self.key_blue = c,
                _ => self.c1 = c,
            }
            self.dirty = true;
            self.toast(format!("已取色 #{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()), false);
        }

        let zoom = if fit.x > 0.0 { fit.x / mw as f32 * self.view_scale * 100.0 } else { 100.0 };
        StageStat { hover, zoom, fit, img_w: mw, canvas: rect, has_img: true }
    }

    /// 双背景：画布即两个落区（点击 / 拖入）

    pub(crate) fn stage_dual_slots(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter().clone().with_clip_rect(rect);
        p.rect_filled(rect, 0.0, C_PAPER);
        let pad = 18.0;
        p.text(
            pos2(rect.left() + pad, rect.top() + 14.0),
            Align2::LEFT_TOP,
            "// 需要两张同尺寸照片：黑底一张、白底一张",
            FontId::proportional(11.0),
            C_INK2,
        );
        let top = rect.top() + 42.0;
        let gap = 12.0;
        let zh = ((rect.bottom() - 16.0 - top - gap) / 2.0).max(56.0);
        let zw = rect.width() - pad * 2.0;
        let slots = [
            ("黑底图", Slot::Black, self.black.as_ref().map(|m| (m.w, m.h)), self.tex_black.as_ref().map(|t| t.id())),
            ("白底图", Slot::White, self.white.as_ref().map(|m| (m.w, m.h)), self.tex_white.as_ref().map(|t| t.id())),
        ];
        let mut pick: Option<Slot> = None;
        for (i, (label, slot, dims, tex)) in slots.into_iter().enumerate() {
            let z = Rect::from_min_size(pos2(rect.left() + pad, top + i as f32 * (zh + gap)), vec2(zw, zh));
            let resp = ui.interact(z, egui::Id::new(("dual_zone", i)), Sense::click());
            match (dims, tex) {
                (Some((iw, ih)), Some(t)) => {
                    p.rect_filled(z, 0.0, C_PANEL);
                    p.rect_stroke(z, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
                    let fit = fit_size(z.shrink(52.0).size(), iw, ih);
                    let ir = Rect::from_center_size(z.center(), fit);
                    p.image(t, ir, uv01(), Color32::WHITE);
                    p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_RULE), StrokeKind::Inside);
                    p.text(
                        pos2(z.left() + 12.0, z.top() + 10.0),
                        Align2::LEFT_TOP,
                        label,
                        FontId::proportional(11.0),
                        C_INK2,
                    );
                    p.text(
                        pos2(z.right() - 12.0, z.top() + 10.0),
                        Align2::RIGHT_TOP,
                        format!("{iw} × {ih} · 点击替换"),
                        FontId::proportional(10.5),
                        C_INK2,
                    );
                }
                _ => {
                    hatch(&p, z);
                    p.rect_stroke(z, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
                    p.rect_filled(
                        Rect::from_min_size(pos2(z.left(), z.center().y - 26.0), vec2(3.0, 52.0)),
                        0.0,
                        C_RULE,
                    );
                    p.text(
                        pos2(z.left() + 18.0, z.center().y - 12.0),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(15.0),
                        C_INK,
                    );
                    p.text(
                        pos2(z.left() + 18.0, z.center().y + 12.0),
                        Align2::LEFT_CENTER,
                        "点击载入，或把图片拖到窗口",
                        FontId::proportional(10.5),
                        C_INK2,
                    );
                }
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                p.rect_stroke(z, 0.0, Stroke::new(2.0, C_INK), StrokeKind::Inside);
            }
            if resp.clicked() {
                pick = Some(slot);
            }
        }
        if let Some(slot) = pick {
            self.pick_slot_image(slot);
        }
    }


    pub(crate) fn stage_empty(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter().clone().with_clip_rect(rect);
        p.rect_filled(rect, 0.0, C_PAPER);
        hatch(&p, rect);
        let bw = (rect.width() - 80.0).clamp(240.0, 430.0);
        let band = Rect::from_center_size(rect.center(), vec2(bw, 210.0));
        let plate = band.shrink2(vec2(-14.0, -6.0));
        p.rect_filled(plate, 0.0, C_PAPER);
        p.rect_stroke(plate, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
        let dual = self.mode == Mode::DualBg;
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(band).layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                ui.add_space(18.0);
                ui.label(egui::RichText::new("// 等待输入").size(10.0).color(C_INK2));
                ui.add_space(5.0);
                ui.label(
                    egui::RichText::new(if dual { "载入黑底 + 白底两张图" } else { "把图片拖进窗口" })
                        .size(21.0)
                        .strong()
                        .color(C_INK),
                );
                ui.add_space(7.0);
                ui.label(
                    egui::RichText::new(if dual {
                        "同一主体、同一尺寸，一次拖入两张会自动填槽"
                    } else {
                        "Ctrl+V 粘贴 · 按 O 键打开 · 支持 PNG / JPG / WEBP / BMP"
                    })
                    .size(11.0)
                    .color(C_INK2),
                );
                ui.add_space(18.0);
                if dual {
                    ui.horizontal(|ui| {
                        if primary_block(ui, "载入黑底", vec2(102.0, 32.0)).clicked() {
                            self.pick_slot_image(Slot::Black);
                        }
                        ui.add_space(6.0);
                        if primary_block(ui, "载入白底", vec2(102.0, 32.0)).clicked() {
                            self.pick_slot_image(Slot::White);
                        }
                    });
                } else if primary_block(ui, "打开图片", vec2(126.0, 34.0)).clicked() {
                    self.open_file_dialog();
                }
            },
        );
    }

    /// 视口公共交互：滚轮缩放（鼠标中心）/ 拖动平移 / 双击 1:1 ↔ 适应窗口

    pub(crate) fn viewport_interaction(&mut self, resp: &egui::Response, fit: Vec2, vp: Rect, img_w: usize, allow_pan: bool) {
        if resp.hovered() {
            let scroll = resp.ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old_s = self.view_scale;
                let new_s = (old_s * (1.0 + scroll * 0.0018)).clamp(0.3, 16.0);
                if let Some(ptr) = resp.hover_pos() {
                    let c0 = vp.center();
                    let u = (ptr - c0 - self.view_offset) / (fit * old_s);
                    self.view_offset = ptr - c0 - u * (fit * new_s);
                }
                self.view_scale = new_s;
                let disp = fit * new_s;
                self.clamp_offset(disp, vp);
            }
            if allow_pan && self.view_scale > 1.0 && !resp.dragged() {
                resp.ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
        if allow_pan && self.view_scale > 1.0 && resp.dragged() {
            self.view_offset += resp.drag_delta();
            self.clamp_offset(fit * self.view_scale, vp);
            resp.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        // 中键：任何缩放级别都能拖
        if allow_pan && resp.dragged_by(egui::PointerButton::Middle) {
            self.view_offset += resp.drag_delta();
            self.clamp_offset(fit * self.view_scale, vp);
            resp.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.double_clicked() {
            if (self.view_scale - 1.0).abs() < 0.01 {
                self.view_scale = (img_w as f32 / fit.x).clamp(1.0, 16.0);
                self.view_offset = Vec2::ZERO;
            } else {
                self.view_scale = 1.0;
                self.view_offset = Vec2::ZERO;
            }
        }
    }


    pub(crate) fn clamp_offset(&mut self, disp: Vec2, vp: Rect) {
        let max_x = ((disp.x - vp.width()) / 2.0 + 24.0).max(0.0);
        let max_y = ((disp.y - vp.height()) / 2.0 + 24.0).max(0.0);
        self.view_offset.x = self.view_offset.x.clamp(-max_x, max_x);
        self.view_offset.y = self.view_offset.y.clamp(-max_y, max_y);
    }

    // ---------------- 帮助弹窗 ----------------

}
