//! 自绘标题栏：品牌、文件名、窗口拖拽 / 最小化 / 最大化 / 关闭。

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
    // ---------------- 标题栏 ----------------

    /// 自绘标题栏：品牌 + 文件名 + 窗口键
    pub(crate) fn title_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, C_BAR);
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.add_space(12.0);
                let (mark, _) = ui.allocate_exact_size(vec2(4.0, 14.0), Sense::hover());
                ui.painter().rect_filled(mark, 0.0, C_SIGNAL);
                ui.add_space(7.0);
                ui.label(egui::RichText::new("框构").size(14.0).strong().color(C_BAR_TXT));
                ui.add_space(7.0);
                ui.label(egui::RichText::new("// FRAMEFORGE").size(9.5).color(C_INK3_DARK));
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                        .size(9.0)
                        .color(C_INK3_DARK),
                );
                ui.add_space(13.0);
                rule_v_dark(ui);
                ui.add_space(13.0);
                let name = match self.recent.first() {
                    Some(p) if self.orig.is_some() => file_name_of(p),
                    _ => String::new(),
                };
                ui.allocate_ui_with_layout(
                    vec2((rect.width() * 0.38).clamp(110.0, 440.0), TITLE_H),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(name).size(12.5).color(C_BAR_TXT)).truncate());
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if window_btn(ui, "✕", C_SIGNAL, C_INK).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                    if window_btn(ui, if maximized { "❐" } else { "□" }, C_HOVER_DARK, C_BAR_TXT).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if window_btn(ui, "─", C_HOVER_DARK, C_BAR_TXT).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    let (_r, drag) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                    if drag.drag_started() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag.double_clicked() {
                        let m = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!m));
                    }
                });
            },
        );
    }

    // ---------------- 深色命令栏：模式 + 动作 ----------------

}
