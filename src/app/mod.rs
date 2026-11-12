//! 应用装配层：模块声明、eframe::App 实现（save / logic / ui）。
//!
//! 只做「接线」，具体逻辑分别住在 state.rs（状态）、image.rs（图片）、
//! input.rs（输入）、update_ui.rs（更新）与 ui/（绘制）。
#![allow(unused_imports)]

pub mod image;
pub mod input;
pub mod state;
pub mod types;
pub mod update_ui;

pub use state::App;
pub use types::*;

use eframe::egui;
use std::time::Duration;
use crate::update;
use crate::ui;
use crate::ui::util::*;
use crate::ui::widgets::*;
use crate::video::*;

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.save_persisted(storage);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_input(ctx);
        self.poll_updates(ctx);

        if self.workspace == Workspace::Video {
            // 视频工作区：自己轮询预览 / 试编码 / 导出三个后台通道，
            // 图片工作区的高分辨率刷新完全不参与，避免无谓重绘。
            self.video.poll(ctx);
        } else {
            // 自适应：拖动时用快代理 + 16ms 间隔；松手后 180ms 补一次全质量
            let pointer_down = ctx.input(|i| i.pointer.any_down());
            let want_fast = pointer_down || self.pending_fast;
            if self.dirty {
                let gap = if want_fast { 16 } else if self.fast_done { 90 } else { 45 };
                if self.last_run.elapsed() >= Duration::from_millis(gap) {
                    self.refresh(ctx, want_fast);
                }
                ctx.request_repaint();
            } else if self.fast_done {
                if self.last_run.elapsed() >= Duration::from_millis(180) {
                    self.dirty = true;
                    ctx.request_repaint();
                } else {
                    ctx.request_repaint_after(Duration::from_millis(40));
                }
            }
        }

        // 窗口标题 = 文件名 + 状态
        let file_part = if self.workspace == Workspace::Video {
            self.video.info.as_ref()
                .map(|i| i.path.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default())
                .unwrap_or_else(|| "视频抠图".into())
        } else {
            match self.recent.first() {
                Some(p) if self.orig.is_some() => file_name_of(p),
                _ => "智能抠图处理".into(),
            }
        };
        let state = if self.workspace == Workspace::Video {
            if self.video.exporting { "导出中" } else { "就绪" }
        } else if self.dirty {
            "处理中"
        } else {
            "就绪"
        };
        let title = format!("框构 // {file_part} · {state}");
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // Toast 过期
        self.toasts.retain(|t| t.born.elapsed() < Duration::from_millis(2500));
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        ui::root(self, ui, frame);
    }
}
