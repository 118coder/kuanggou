//! 右侧参数栏：按当前图片模式渲染对应参数组 + 最近文件列表。

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
    pub(crate) fn props_panel(&mut self, ui: &mut egui::Ui) {
        let mode = self.mode;
        ui.scope(|ui| {
            {
                let v = &mut ui.style_mut().visuals.widgets;
                v.inactive.bg_fill = C_INK;
                v.inactive.fg_stroke = Stroke::new(1.0, C_INK);
                v.hovered.bg_fill = C_INK;
                v.hovered.fg_stroke = Stroke::new(1.0, C_INK);
                v.active.bg_fill = C_SIGNAL;
                v.active.fg_stroke = Stroke::new(1.0, C_SIGNAL);
            }
            let reset = self.scroll_top;
            self.scroll_top = false;
            let mut sa = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
            if reset {
                sa = sa.vertical_scroll_offset(0.0);
            }
            sa.show(ui, |ui| {
                    // 纵向滚动条是浮层，会盖住滑杆右端：先按条宽收窄内容区。
                    let bar = ui.spacing().scroll.bar_width + 4.0;
                    ui.set_max_width((ui.available_width() - bar).max(80.0));
                    self.props_head(ui);
                    match mode {
                        Mode::DualBg => self.dualbg_params(ui),
                        Mode::Fx => self.fx_params(ui),
                        Mode::Auto => self.auto_params(ui),
                        Mode::Unmult => self.unmult_params(ui),
                        _ => self.chroma_params(ui),
                    }
                    ui.add_space(14.0);
                    self.recents(ui);
                    ui.add_space(10.0);
                });
        });
    }


    pub(crate) fn props_head(&mut self, ui: &mut egui::Ui) {
        let (en, zh, desc) = mode_meta(self.mode);
        let n = mode_idx(self.mode) + 1;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("// {en}")).size(10.0).color(C_INK2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("No.{:02}", n)).size(10.0).color(C_INK2));
            });
        });
        ui.add_space(1.0);
        ui.label(egui::RichText::new(zh).size(19.0).strong().color(C_INK));
        ui.add_space(3.0);
        ui.label(egui::RichText::new(desc).size(11.5).color(C_INK2));
        ui.add_space(9.0);
        ui.separator();
        if self.orig.is_some()
            && matches!(
                self.mode,
                Mode::Green | Mode::Blue | Mode::Custom | Mode::Dual | Mode::Fx | Mode::Auto
            )
        {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("点画布取色")
                        .size(11.0)
                        .strong()
                        .color(C_INK)
                        .background_color(C_SIGNAL),
                );
            });
        }
    }


    pub(crate) fn recents(&mut self, ui: &mut egui::Ui) {
        section(ui, "RECENTS", "最近文件", Some(format!("{} 条", self.recent.len())));
        if self.recent.is_empty() {
            ui.label(
                egui::RichText::new("还没有记录。打开或拖入图片后会出现在这里，点一下即可重新载入。")
                    .size(11.0)
                    .color(C_INK2),
            );
            return;
        }
        let recent = self.recent.clone();
        for (i, p) in recent.iter().enumerate() {
            self.recent_row(ui, i, p);
        }
    }


    pub(crate) fn recent_row(&mut self, ui: &mut egui::Ui, idx: usize, p: &Path) {
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
        let xr = Rect::from_min_size(pos2(rect.right() - 22.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        let del = ui.interact(xr, ui.id().with(("recent_del", idx)), Sense::click());
        {
            let painter = ui.painter();
            if resp.hovered() {
                painter.rect_filled(
                    Rect::from_min_size(pos2(rect.left(), rect.top() + 2.0), vec2(3.0, rect.height() - 4.0)),
                    0.0,
                    C_INK,
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let num = format!("{:02}", idx + 1);
            let ng = painter.layout_no_wrap(num, FontId::proportional(10.0), C_TAG_FG);
            let chip = Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.0), vec2(ng.size().x + 10.0, 16.0));
            painter.rect_filled(chip, 0.0, C_TAG_BG);
            painter.galley(pos2(chip.left() + 5.0, chip.center().y - ng.size().y / 2.0), ng, C_TAG_FG);
            let font = FontId::proportional(12.0);
            let text = ellipsize(painter, &file_name_of(p), font.clone(), w - chip.width() - 50.0);
            painter.text(
                pos2(chip.right() + 8.0, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                font,
                if resp.hovered() { C_INK } else { C_INK2 },
            );
            if resp.hovered() || del.hovered() {
                if del.hovered() {
                    painter.rect_filled(xr, 0.0, C_SIGNAL);
                }
                painter.text(
                    xr.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(12.0),
                    C_INK,
                );
                if del.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            painter.line_segment(
                [pos2(rect.left(), rect.bottom()), pos2(rect.right(), rect.bottom())],
                Stroke::new(1.0, C_RULE),
            );
        }
        if del.clicked() {
            self.recent.remove(idx);
            self.toast("已从最近文件移除", false);
        } else if resp.clicked() {
            self.load_recent(idx);
        }
    }

    // ---------------- 色键参数 ----------------


    pub(crate) fn chroma_params(&mut self, ui: &mut egui::Ui) {
        section(ui, "PARAMETERS", "去底参数", None);
        let s = slider_row(ui, "容差", &mut self.tol, 0.0..=150.0, 80.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "柔边", &mut self.feather, 0.0..=50.0, 5.0, 1.0, 0);
        self.slider_changed(&s);
        if matches!(self.mode, Mode::Green | Mode::Blue) {
            let s = slider_row(ui, "抑色", &mut self.suppress, 0.0..=100.0, 60.0, 1.0, 0);
            self.slider_changed(&s);
        }
        if matches!(self.mode, Mode::Green | Mode::Blue) {
            let is_green = self.mode == Mode::Green;
            let pure = if is_green {
                Color32::from_rgb(0, 255, 0)
            } else {
                Color32::from_rgb(0, 0, 255)
            };
            let mut key = if is_green { self.key_green } else { self.key_blue };
            section(ui, "KEY COLOR", if is_green { "绿幕颜色" } else { "蓝幕颜色" }, None);
            let mut changed = false;
            ui.horizontal(|ui| {
                if cut_block(
                    ui,
                    if is_green { "重置纯绿" } else { "重置纯蓝" },
                    false,
                    vec2(66.0, 25.0),
                    CUT_SMALL,
                )
                .clicked()
                {
                    key = pure;
                    changed = true;
                }
                if ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut key).changed() {
                    changed = true;
                }
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", key.r(), key.g(), key.b()))
                        .size(11.0)
                        .color(C_INK),
                );
            });
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(if is_green {
                    "背景绿不纯时，点一下画布里的绿幕就能取到实际颜色。"
                } else {
                    "背景蓝不纯时，点一下画布里的蓝幕就能取到实际颜色。"
                })
                .size(10.5)
                .color(C_INK2),
            );
            if changed {
                self.push_undo();
                if is_green {
                    self.key_green = key;
                } else {
                    self.key_blue = key;
                }
                self.dirty = true;
            }
        }
        if matches!(self.mode, Mode::Custom | Mode::Dual) {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("键色 1").size(11.5).color(C_INK2));
                ui.add_space(6.0);
                if ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut self.c1).changed() {
                    self.push_undo();
                    self.dirty = true;
                }
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", self.c1.r(), self.c1.g(), self.c1.b()))
                        .size(10.5)
                        .color(C_INK2),
                );
            });
        }
        if self.mode == Mode::Dual {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("键色 2").size(11.5).color(C_INK2));
                ui.add_space(6.0);
                if ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut self.c2).changed() {
                    self.push_undo();
                    self.dirty = true;
                }
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", self.c2.r(), self.c2.g(), self.c2.b()))
                        .size(10.5)
                        .color(C_INK2),
                );
            });
        }

        section(ui, "EDGE", "边缘处理", None);
        let s = slider_row(ui, "边缘内缩", &mut self.ck_erode, 0.0..=10.0, 0.0, 0.1, 1);
        self.slider_changed(&s);
        let s = slider_row(ui, "边缘平滑", &mut self.ck_smooth, 0.0..=10.0, 0.0, 0.1, 1);
        self.slider_changed(&s);
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("内缩让边缘更干净，去锯齿让边缘更柔和。")
                .size(10.5)
                .color(C_INK2),
        );
        ui.label(
            egui::RichText::new("预览和导出用同一套参数。")
                .size(10.5)
                .color(C_INK2),
        );
    }


    pub(crate) fn slider_changed(&mut self, s: &SliderOut) {
        if s.drag_started {
            self.push_undo();
        }
        if s.changed {
            self.dirty = true;
        }
    }

    // ---------------- 半透明参数 ----------------


    pub(crate) fn fx_params(&mut self, ui: &mut egui::Ui) {
        let black_bg = self.fx_bg_type == FxBgType::Black;
        section(ui, "METHOD", "处理方式", None);
        if black_bg {
            ui.horizontal(|ui| {
                swatch(ui, C_INK);
                ui.label(egui::RichText::new("黑屏光效").size(12.0).strong().color(C_INK));
            });
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new("纯黑背景直接变透明，并补回光晕颜色。适合闪光、火焰，不用取色。")
                    .size(10.5)
                    .color(C_INK2),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                if cut_block(ui, "彩色光效", self.fx_model == FxModel::Color, vec2(86.0, 26.0), CUT_SMALL).clicked() {
                    self.push_undo();
                    self.fx_model = FxModel::Color;
                    self.dirty = true;
                }
                if cut_block(ui, "烟雾 / 薄纱", self.fx_model == FxModel::Smoke, vec2(96.0, 26.0), CUT_SMALL).clicked() {
                    self.push_undo();
                    self.fx_model = FxModel::Smoke;
                    self.dirty = true;
                }
            });
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(match self.fx_model {
                    FxModel::Smoke => "保留灰白烟雾、薄纱的层次；白底会自动改用光效算法。",
                    _ => "优先保留弱光和彩色渐变。和背景同色的部分无法区分。",
                })
                .size(10.5)
                .color(C_INK2),
            );
        }

        section(ui, "KEY", "底色", None);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("底色").size(11.5).color(C_INK2));
            if !black_bg && ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut self.fx_bg).changed() {
                self.push_undo();
                self.dirty = true;
            }
            if black_bg {
                ui.label(egui::RichText::new("纯黑，无需取色").size(10.5).color(C_INK2));
            } else {
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", self.fx_bg.r(), self.fx_bg.g(), self.fx_bg.b()))
                        .size(10.5)
                        .color(C_INK2),
                );
            }
        });
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            for bg in [FxBgType::Blue, FxBgType::Green, FxBgType::Black, FxBgType::White, FxBgType::Magenta] {
                let selected = self.fx_bg_type == bg;
                if cut_block(ui, bg.label(), selected, vec2(56.0, 25.0), CUT_SMALL).clicked() {
                    self.push_undo();
                    self.fx_apply_bg(bg);
                }
            }
            if cut_block(ui, "重置", false, vec2(48.0, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_reset_params();
                self.toast("特效参数已按底色复位", false);
            }
        });

        section(ui, "FINE TUNE", "参数调节", Some("4–5 项".into()));
        if !black_bg {
            let s = slider_row(ui, "底色范围", &mut self.fx_ctol, 0.0..=20.0, 3.0, 1.0, 0);
            self.slider_changed(&s);
        }
        let s = slider_row(ui, "背景清理", &mut self.fx_bpoint, 0.0..=20.0, 1.0, 0.1, 1);
        self.slider_changed(&s);
        let s = slider_row(ui, "主体实心度", &mut self.fx_white, 50.0..=100.0, 100.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "浓淡", &mut self.fx_density, 0.2..=3.0, 1.0, 0.05, 2);
        self.slider_changed(&s);
        if !black_bg && self.fx_bg_type != FxBgType::White {
            let s = slider_row(ui, "去色边", &mut self.fx_despill, 0.0..=100.0, 20.0, 1.0, 0);
            self.slider_changed(&s);
        }
    }

    // ---------------- 智能抠底参数 ----------------


    pub(crate) fn auto_params(&mut self, ui: &mut egui::Ui) {
        section(ui, "KEY", "识别底色", None);
        ui.horizontal(|ui| {
            match self.auto_bg {
                Some(c) => {
                    swatch(ui, c);
                    ui.label(
                        egui::RichText::new(format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()))
                            .size(11.0)
                            .color(C_INK),
                    );
                }
                None => {
                    ui.label(egui::RichText::new("自动").size(10.5).color(C_INK2));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.orig.is_some() {
                    if cut_block(ui, "重新识别", false, vec2(72.0, 24.0), CUT_SMALL).clicked() {
                        self.auto_bg = None;
                        self.auto_tried = false;
                        self.dirty = true;
                    }
                }
            });
        });

        section(ui, "METHOD", "处理方式", None);
        let cw = ((ui.available_width() - 8.0) / 3.0).max(64.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if cut_block(ui, "彩色光效", self.fx_model == FxModel::Color, vec2(cw, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_model = FxModel::Color;
                self.dirty = true;
            }
            if cut_block(ui, "烟雾薄纱", self.fx_model == FxModel::Smoke, vec2(cw, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_model = FxModel::Smoke;
                self.dirty = true;
            }
            if cut_block(ui, "黑屏光效", self.fx_model == FxModel::Black, vec2(cw, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_model = FxModel::Black;
                self.dirty = true;
            }
        });

        section(ui, "FINE TUNE", "参数微调", None);
        if self.fx_model != FxModel::Black {
            let s = slider_row(ui, "底色范围", &mut self.fx_ctol, 0.0..=20.0, 3.0, 1.0, 0);
            self.slider_changed(&s);
        }
        let s = slider_row(ui, "背景清理", &mut self.fx_bpoint, 0.0..=20.0, 1.0, 0.1, 1);
        self.slider_changed(&s);
        let s = slider_row(ui, "主体实心度", &mut self.fx_white, 50.0..=100.0, 100.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "浓淡", &mut self.fx_density, 0.2..=3.0, 1.0, 0.05, 2);
        self.slider_changed(&s);
        if self.fx_model != FxModel::Black {
            let s = slider_row(ui, "去色边", &mut self.fx_despill, 0.0..=100.0, 20.0, 1.0, 0);
            self.slider_changed(&s);
        }
    }

    // ---------------- 双背景差分参数 ----------------


    pub(crate) fn dualbg_params(&mut self, ui: &mut egui::Ui) {
        section(ui, "SLOTS", "输入槽位", None);
        for (label, slot, loaded) in [
            ("黑底图", Slot::Black, self.black.is_some()),
            ("白底图", Slot::White, self.white.is_some()),
        ] {
            let size_text = match slot {
                Slot::Black => self.black.as_ref().map(|b| format!("{} × {}", b.w, b.h)),
                Slot::White => self.white.as_ref().map(|b| format!("{} × {}", b.w, b.h)),
            };
            ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(vec2(3.0, 14.0), Sense::hover());
                ui.painter().rect_filled(bar, 0.0, if loaded { C_INK } else { C_RULE });
                ui.label(egui::RichText::new(label).size(11.5).strong().color(if loaded { C_INK } else { C_INK2 }));
                ui.add_space(2.0);
                if let Some(s) = size_text {
                    ui.label(egui::RichText::new(s).size(10.5).color(C_INK2));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if loaded {
                        if cut_block(ui, "清除", false, vec2(48.0, 23.0), CUT_SMALL).clicked() {
                            match slot {
                                Slot::Black => {
                                    self.black = None;
                                    self.proxy_black = None;
                                    self.tex_black = None;
                                }
                                Slot::White => {
                                    self.white = None;
                                    self.proxy_white = None;
                                    self.tex_white = None;
                                }
                            }
                            self.dirty = true;
                        }
                    } else {
                        ui.label(egui::RichText::new("点画布槽位载入").size(10.0).color(C_INK2));
                    }
                });
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
        }
        ui.label(
            egui::RichText::new("两张要同尺寸；一次拖两张会自动填好，也可以点画布上的槽位。")
                .size(10.5)
                .color(C_INK2),
        );

        section(ui, "FINE TUNE", "差分参数", None);
        let s = slider_row(ui, "容差", &mut self.db_tol, 0.0..=100.0, 20.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "边缘对比", &mut self.db_edge, 0.0..=100.0, 30.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "边缘内缩", &mut self.db_erode, 0.0..=100.0, 0.0, 1.0, 0);
        self.slider_changed(&s);
    }


    pub(crate) fn pick_slot_image(&mut self, slot: Slot) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("图片文件", &["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "webp"]);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(p) = dialog.pick_file() else { return };
        match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| Img::from_bytes(&b)) {
            Ok(img) => match self.set_slot(slot, img) {
                Ok(()) => {
                    self.toast(
                        format!("已载入{}", if slot == Slot::Black { "黑底图" } else { "白底图" }),
                        false,
                    );
                    self.last_dir = p.parent().map(|x| x.to_path_buf());
                }
                Err(e) => self.toast(e, true),
            },
            Err(e) => self.toast(format!("载入失败：{e}"), true),
        }
    }

    // ---------------- 预览舞台 ----------------


    // ---------------- Unmult 去黑底 ----------------

    /// 参考 AE 插件 Unmult：α = 亮度（黑场/白场校正），RGB = C / α 反预乘。
    /// 专用于黑底光效、烟雾、火花；实拍黑底人像请改用色键或双底合成。
    pub(crate) fn unmult_params(&mut self, ui: &mut egui::Ui) {
        section(ui, "UNMULT", "去黑底参数", Some("AE Unmult 同款".into()));
        let s = slider_row(ui, "黑场", &mut self.un_black, 0.0..=64.0, 0.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "白场", &mut self.un_white, 128.0..=255.0, 255.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "浓淡", &mut self.un_density, 0.2..=3.0, 1.0, 0.05, 2);
        self.slider_changed(&s);
        let s = slider_row(ui, "清底", &mut self.un_floor, 0.0..=32.0, 2.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "颜色补偿", &mut self.un_gain, 1.0..=16.0, 12.0, 1.0, 0);
        self.slider_changed(&s);
        if ui.checkbox(&mut self.un_luma, "用亮度代替最大通道求透明度").changed() {
            self.push_undo();
            self.dirty = true;
        }
        section(ui, "EDGE", "边缘处理", None);
        let s = slider_row(ui, "边缘内缩", &mut self.ck_erode, 0.0..=10.0, 0.0, 0.5, 1);
        self.slider_changed(&s);
        let s = slider_row(ui, "边缘平滑", &mut self.ck_smooth, 0.0..=10.0, 0.0, 0.5, 1);
        self.slider_changed(&s);
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("提示：黑底越纯效果越好；有噪点就调高「黑场」和「清底」。")
                .size(10.0).color(C_INK2),
        );
    }

}
