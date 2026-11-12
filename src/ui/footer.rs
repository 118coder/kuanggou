//! 底部状态栏：就绪状态、消息、耗时、引擎、更新入口、清空与显示文件。

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
    pub(crate)     fn footer(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        // 视频工作区显示自己的状态，图片工作区显示图片处理状态，互不串味。
        let status = if self.workspace == Workspace::Video {
            self.video.status.clone()
        } else {
            self.status.clone()
        };
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, C_BAR);
        let proxy = match self.mode {
            Mode::DualBg => self.proxy_black.is_some() || self.proxy_white.is_some(),
            _ => self.proxy_main.is_some(),
        };
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                // 视频工作区没有「脏标记」概念，状态由导出 / 估算 / 播放三个开关决定。
                let (busy, label) = if self.workspace == Workspace::Video {
                    if self.video.exporting {
                        (true, "导出中")
                    } else if self.video.sampling {
                        (true, "估算中")
                    } else if self.video.playing {
                        (true, "播放中")
                    } else {
                        (false, "就绪")
                    }
                } else if self.dirty {
                    (true, "处理中")
                } else {
                    (false, "就绪")
                };
                let (dot, _) = ui.allocate_exact_size(vec2(4.0, 4.0), Sense::hover());
                ui.painter().rect_filled(dot, 0.0, if busy { C_SIGNAL } else { C_INK3_DARK });
                ui.add_space(7.0);
                ui.label(
                    egui::RichText::new(label)
                        .size(11.5)
                        .strong()
                        .color(C_BAR_TXT),
                );
                ui.add_space(11.0);
                rule_v_dark(ui);
                ui.add_space(11.0);
                ui.allocate_ui_with_layout(
                    vec2((rect.width() * 0.42).clamp(120.0, 620.0), STATUS_H),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if status.contains("失败") || status.contains("错误") || status.contains("不一致") {
                            let (bar, _) = ui.allocate_exact_size(vec2(3.0, 12.0), Sense::hover());
                            ui.painter().rect_filled(bar, 0.0, C_SIGNAL);
                            ui.add_space(5.0);
                        }
                        ui.add(egui::Label::new(egui::RichText::new(&status).size(11.0).color(C_INK3_DARK)).truncate());
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if link_btn(ui, "快捷键 F1").clicked() {
                        self.help_open = true;
                    }
                    ui.add_space(12.0);
                    if let Some(info) = &self.upd_info {
                        let label = format!("新版本 v{}", info.version);
                        if upd_chip(ui, &label).clicked() {
                            self.upd_open = true;
                        }
                    } else if self.upd_state == UpdState::Checking {
                        ui.label(
                            egui::RichText::new("检查更新…")
                                .size(10.0)
                                .color(C_INK3_DARK),
                        );
                    } else if link_btn(ui, "检查更新").clicked() {
                        self.check_updates(true);
                    }
                    if self.last_export.is_some() {
                        ui.add_space(12.0);
                        if link_btn(ui, "显示文件").clicked() {
                            self.reveal_last_export();
                        }
                    }
                    if self.workspace != Workspace::Video && (self.orig.is_some() || self.black.is_some() || self.white.is_some()) {
                        ui.add_space(12.0);
                        if link_btn(ui, "清空").clicked() {
                            self.clear_all();
                            self.status = "已清空".into();
                            self.toast("已清空", false);
                        }
                    }
                    ui.add_space(12.0);
                    field_dark(ui, "耗时", &format!("{:.1} ms", self.last_ms));
                    ui.add_space(12.0);
                    field_dark(ui, "引擎", "RUST");
                    if proxy {
                        ui.add_space(12.0);
                        field_dark(ui, "预览", "代理");
                    }
                });
            },
        );
    }

    // ---------------- 属性栏 ----------------

}
