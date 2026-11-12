//! F1 帮助弹窗：快捷键表与操作说明。

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
    pub(crate) fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.help_open;
        egui::Window::new("快捷键 // SHORTCUTS")
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
                const ROWS: [(&str, &str); 21] = [
                    ("1 – 9 / 0", "切换处理模式（0 = 去黑底 Unmult）"),
                    ("W", "切换 图片 / 视频 工作区"),
                    ("O", "打开图片"),
                    ("V / Ctrl+V", "粘贴剪贴板图片"),
                    ("E", "导出 PNG"),
                    ("Ctrl+C", "复制结果到剪贴板"),
                    ("X", "清空画布"),
                    ("C", "对比原图 / 结果"),
                    ("F", "适应窗口"),
                    ("Ctrl+Z / Ctrl+Y", "撤销 / 恢复参数"),
                    ("双击滑杆", "恢复该参数默认值"),
                    ("Shift + 滚轮", "微调参数（普通滚轮只滚动面板）"),
                    ("拖动数值", "左右拖动微调参数"),
                    ("预览滚轮", "以鼠标为中心缩放"),
                    ("预览拖动", "平移（放大后）"),
                    ("双击预览", "100% 像素 ↔ 适应窗口"),
                    ("按住空格", "速览原图"),
                    ("中键拖动", "平移画布（任何缩放级别）"),
                    ("Esc", "关闭弹窗 → 退出对比 → 回到适应窗口"),
                    ("视频：空格", "播放 / 暂停"),
                    ("视频：M / E", "精确预估 / 导出"),
                ];
                for (k, v) in ROWS {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(k)
                                .size(11.0)
                                .strong()
                                .color(C_TAG_FG)
                                .background_color(C_TAG_BG),
                        );
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(v).size(12.0).color(C_INK));
                    });
                    ui.add_space(2.0);
                }
                ui.add_space(6.0);
                ui.separator();
                ui.label(
                    egui::RichText::new("拖入图片直接替换主图（双背景模式先拖两张填黑/白槽）；拖入视频自动切到视频工作区。")
                        .size(10.5)
                        .color(C_INK2),
                );
            });
        self.help_open = open;
    }
}
