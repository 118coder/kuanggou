//! 自动更新的 UI 部分：轮询后台消息、手动检查、更新弹窗。
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
    pub(crate) fn poll_updates(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.upd_rx.try_recv() {
            match m {
                update::Msg::Found(i) => {
                    self.upd_state = UpdState::Idle;
                    if self.skip_version.as_deref() != Some(i.version.as_str()) {
                        self.toast(format!("发现新版本 v{}", i.version), false);
                        self.upd_info = Some(i);
                    }
                }
                update::Msg::NoUpdate(v) => {
                    self.upd_state = UpdState::Latest(v);
                    if self.upd_manual {
                        self.toast("已是最新版本", false);
                    }
                    self.upd_manual = false;
                }
                update::Msg::Failed(e) => {
                    self.upd_state = UpdState::Failed(e.clone());
                    if self.upd_manual {
                        self.toast(format!("检查更新失败：{e}"), true);
                    }
                    self.upd_manual = false;
                }
                update::Msg::Ready(p) => {
                    self.upd_state = UpdState::Idle;
                    match update::apply(&p) {
                        Ok(()) => {
                            self.toast("正在替换并重启…", false);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(e) => {
                            self.upd_busy = false;
                            self.toast(format!("替换失败：{e}"), true);
                        }
                    }
                }
            }
        }
    }


    pub(crate) fn check_updates(&mut self, manual: bool) {
        if self.upd_state == UpdState::Checking {
            return;
        }
        self.upd_state = UpdState::Checking;
        self.upd_manual = manual;
        update::check(self.upd_tx.clone());
    }


    pub(crate) fn update_window(&mut self, ctx: &egui::Context) {
        if !self.upd_open {
            return;
        }
        let Some(info) = self.upd_info.clone() else {
            self.upd_open = false;
            return;
        };
        let busy = self.upd_busy;
        let mut open = true;
        let (mut go, mut later, mut skip) = (false, false, false);
        egui::Window::new("发现有新版本 // UPDATE")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .frame(
                egui::Frame::new()
                    .fill(C_PANEL)
                    .stroke(Stroke::new(1.0, C_INK))
                    .inner_margin(18.0),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!("当前 v{}", update::current_version()))
                        .size(11.0)
                        .color(C_INK2),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(format!("最新 v{}", info.version))
                        .size(19.0)
                        .strong()
                        .color(C_INK),
                );
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(170.0).show(ui, |ui| {
                    let notes = if info.notes.trim().is_empty() {
                        "这个版本没有写更新说明。".to_string()
                    } else {
                        info.notes.clone()
                    };
                    ui.label(egui::RichText::new(notes).size(11.0).color(C_INK2));
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if busy {
                        ui.label(
                            egui::RichText::new("正在下载新版本…")
                                .size(12.0)
                                .strong()
                                .color(C_INK),
                        );
                    } else if primary_block(ui, "下载并安装", vec2(120.0, 32.0)).clicked() {
                        go = true;
                    }
                    ui.add_space(6.0);
                    if cut_block(ui, "以后再说", false, vec2(78.0, 32.0), CUT_SMALL).clicked() {
                        later = true;
                    }
                    if cut_block(ui, "忽略此版本", false, vec2(90.0, 32.0), CUT_SMALL).clicked() {
                        skip = true;
                    }
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("不会强制更新。下载完成后程序自动重启，旧版本留备份，失败会回滚。")
                        .size(10.0)
                        .color(C_INK2),
                );
            });
        if go {
            self.upd_busy = true;
            update::download(info.clone(), self.upd_tx.clone());
        }
        if skip {
            self.skip_version = Some(info.version.clone());
            self.upd_info = None;
            self.upd_open = false;
            self.toast(format!("已忽略 v{}", info.version), false);
        }
        if later || !open {
            self.upd_open = false;
        }
    }

}
