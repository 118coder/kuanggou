//! UI 装配层：整窗精确布局（标题栏 / 三色条 / 工作区切换条 / 命令栏 /
//! 舞台 + 参数栏 / 状态栏），并按当前工作区把主体分发给图片或视频 UI。
//!
//! 布局尺寸令牌全部来自 `theme.rs`；两个工作区共用同一套窗口骨架，
//! 但命令栏、舞台、参数栏由各自的模块提供，互不耦合。

#![allow(unused_imports)]

pub mod command_bar;
pub mod footer;
pub mod help;
pub mod props;
pub mod stage;
pub mod theme;
pub mod title_bar;
pub mod util;
pub mod video;
pub mod video_props;
pub mod widgets;
pub mod workspace_bar;

use eframe::egui;
use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use crate::app::*;
use crate::ui::theme::*;
use crate::ui::widgets::*;
use crate::ui::command_bar::*;
use crate::ui::util::*;

/// 整窗布局入口（由 `eframe::App::ui` 调用）。
pub fn root(app: &mut App, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(0.0).fill(C_PAPER))
        .show(ui, |ui| {
            let area = ui.max_rect();
            let (l, t, w, h) = (area.left(), area.top(), area.width(), area.height());
            let is_video = app.workspace == Workspace::Video;
            // 图片命令栏会按窗口宽度换行；视频命令栏固定一行。
            let cmd = if is_video {
                56.0
            } else {
                cmd_layout(ui.ctx(), pos2(l, t + TITLE_H + TRI_H + workspace_bar::BAR_H), w).h
            };
            let tab_top = t + TITLE_H + TRI_H;
            let cmd_top = tab_top + workspace_bar::BAR_H;
            let body_top = cmd_top + cmd;
            let body_h = (h - TITLE_H - TRI_H - workspace_bar::BAR_H - cmd - STATUS_H).max(240.0);

            ui.painter().rect_filled(area, 0.0, C_PAPER);

            // 标题栏
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, t), vec2(w, TITLE_H))),
                |ui| app.title_bar(ui),
            );
            // 三色条
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, t + TITLE_H), vec2(w, TRI_H))),
                |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(w, TRI_H), Sense::hover());
                    let p = ui.painter().clone();
                    for (i, c) in [C_TRI_A, C_TRI_B, C_TRI_C].into_iter().enumerate() {
                        p.rect_filled(
                            Rect::from_min_max(
                                pos2(r.left() + w * i as f32 / 3.0, r.top()),
                                pos2(r.left() + w * (i + 1) as f32 / 3.0, r.bottom()),
                            ), 0.0, c,
                        );
                    }
                },
            );
            // 工作区切换条
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, tab_top), vec2(w, workspace_bar::BAR_H))),
                |ui| workspace_bar::bar(app, ui, w),
            );
            // 命令栏（按工作区切换）
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, cmd_top), vec2(w, cmd))),
                |ui| {
                    if is_video { video::command_bar(app, ui, w); }
                    else { app.command_bar(ui, w); }
                },
            );

            // 属性栏宽度（舞台至少留 520）
            let props_max = PROPS_MAX.min(w - 520.0).max(PROPS_MIN);
            let props_w = app.props_w.clamp(PROPS_MIN, props_max);
            let props_rect = Rect::from_min_size(pos2(l + w - props_w, body_top), vec2(props_w, body_h));
            let stage_rect = Rect::from_min_max(
                pos2(l + BODY_PAD, body_top + BODY_PAD),
                pos2(l + w - props_w - SPLITTER, body_top + body_h - BODY_PAD),
            );

            ui.painter().line_segment(
                [pos2(props_rect.left(), body_top), pos2(props_rect.left(), body_top + body_h)],
                Stroke::new(1.0, C_RULE),
            );

            if is_video {
                video::stage(app, ui, stage_rect);
                video_props::props(app, ui, props_rect);
            } else {
                app.stage(ui, stage_rect);
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(props_rect.shrink2(vec2(18.0, 14.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                    |ui| app.props_panel(ui),
                );
            }

            // 可拖分隔条（两个工作区共用）
            let sx = l + w - props_w;
            let hit = Rect::from_min_max(pos2(sx - SPLITTER, body_top), pos2(sx, body_top + body_h));
            let sp = ui.interact(hit, egui::Id::new("props_splitter"), Sense::click_and_drag());
            if sp.hovered() || sp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                let x = hit.center().x;
                let w_line = if sp.dragged() { 3.0 } else { 2.0 };
                ui.painter().rect_filled(
                    Rect::from_min_size(pos2(x - w_line / 2.0, body_top + 8.0), vec2(w_line, body_h - 16.0)),
                    0.0,
                    if sp.dragged() { C_SIGNAL } else { C_INK },
                );
            }
            if sp.dragged() {
                if let Some(pp) = sp.interact_pointer_pos() {
                    app.props_w = (l + w - pp.x).clamp(PROPS_MIN, props_max);
                }
            }
            if sp.double_clicked() { app.props_w = PROPS_DEF; }

            // 状态栏
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, t + h - STATUS_H), vec2(w, STATUS_H))),
                |ui| app.footer(ui),
            );

            // 浮层
            if app.help_open { app.help_window(ui.ctx()); }
            app.update_window(ui.ctx());
            draw_toasts(ui.ctx(), &app.toasts);
            if app.hovering > 0 && !is_video {
                draw_drop_hint(ui.ctx(), app.hovering, app.mode, &app.black, &app.white);
            }
        });
}
