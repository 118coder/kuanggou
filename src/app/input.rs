//! 输入路由：拖放、剪贴板、全局快捷键、文件对话框、最近文件。
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
    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        self.hovering = ctx.input(|i| i.raw.hovered_files.len());
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());

        if dropped.is_empty() {
            self.drop_paths.clear();
        } else {
            for f in &dropped {
                let p = f.path().to_path_buf();
                if self.drop_paths.contains(&p) {
                    continue;
                }
                self.drop_paths.push(p.clone());
                // 视频 / 动图一律进视频工作区：拖进来即自动切过去，避免误当图片解码。
                if is_video_file(&p) {
                    if self.workspace != Workspace::Video {
                        self.workspace = Workspace::Video;
                    }
                    self.video.load_video(p.clone(), ctx);
                    continue;
                }
                // 视频工作区不接管图片，避免把整段视频的时间轴搞乱。
                if self.workspace == Workspace::Video {
                    self.toast("视频工作区只接收视频 / 动图，图片请切回「图片抠图」", true);
                    continue;
                }
                match f.bytes() {
                    Ok(bytes) => self.load_dropped(bytes, p),
                    Err(e) => self.toast(format!("读取拖放文件失败：{e}"), true),
                }
            }
        }

        // V / Ctrl+V 粘贴图片（直接替换主图；视频工作区不接管剪贴板）
        if ctx.input(|i| i.key_pressed(egui::Key::V)) && self.workspace != Workspace::Video {
            self.paste_clipboard();
        }

        self.handle_hotkeys(ctx);
    }

    /// 拖入路由：双背景模式多文件 → 依次填黑/白槽（多余忽略并提示）；
    /// 其余一律直接替换主图

    pub(crate) fn load_dropped(&mut self, bytes: Vec<u8>, path: PathBuf) {
        let img = match Img::from_bytes(&bytes) {
            Ok(i) => i,
            Err(e) => {
                self.toast(format!("载入失败：{e}"), true);
                return;
            }
        };
        let name = file_name_of(&path);
        let ordinal = self.drop_paths.iter().position(|x| *x == path).unwrap_or(0) + 1;
        if self.mode == Mode::DualBg {
            if self.black.is_none() {
                match self.set_slot(Slot::Black, img) {
                    Ok(()) => self.toast(format!("{name} → 黑底槽位"), false),
                    Err(e) => self.toast(e, true),
                }
            } else if self.white.is_none() {
                match self.set_slot(Slot::White, img) {
                    Ok(()) => self.toast(format!("{name} → 白底槽位"), false),
                    Err(e) => self.toast(e, true),
                }
            } else {
                self.toast("黑底、白底槽位已填满；如需替换请先点「清除」", true);
                return;
            }
        } else {
            // 直接替换主图
            self.set_main(img);
            self.toast(format!("已替换主图：{name}"), false);
        }
        let _ = ordinal;
        self.push_recent(path);
    }


    pub(crate) fn paste_clipboard(&mut self) {
        match arboard::Clipboard::new() {
            Ok(mut cb) => match cb.get_image() {
                Ok(aimg) => {
                    let (w, h) = (aimg.width as usize, aimg.height as usize);
                    let bytes = aimg.bytes.into_owned();
                    self.set_main(Img::from_raw(w, h, bytes));
                    self.toast("已粘贴剪贴板图片（替换主图）", false);
                }
                Err(_) => self.status = "剪贴板里没有图片".into(),
            },
            Err(e) => self.status = format!("无法访问剪贴板：{e}"),
        }
    }


    pub(crate) fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        // 窗口失焦时释放速览，防止卡在原图
        if !ctx.input(|i| i.focused) {
            self.peek = false;
        }
        // 取色器/输入框正在收键盘时不响应全局快捷键
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.help_open {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.help_open = false;
            }
            return;
        }
        // W：切换图片 / 视频工作区（两个工作区的素材与参数各自保留）
        if ctx.input(|i| i.key_pressed(Key::W)) && !ctx.input(|i| i.modifiers.ctrl) {
            self.workspace = match self.workspace {
                Workspace::Image => Workspace::Video,
                Workspace::Video => Workspace::Image,
            };
            if self.workspace == Workspace::Video {
                self.dirty = false;
            } else {
                self.dirty = true;
            }
            self.video.pause_all();
            self.scroll_top = true;
            self.status = if self.workspace == Workspace::Video { "视频工作区" } else { "图片工作区" }.into();
            return;
        }
        // 视频工作区：空格播放 / 暂停，M 精确预估，E 导出；其余图片快捷键一律不接管
        if self.workspace == Workspace::Video {
            if ctx.input(|i| i.key_pressed(Key::Space)) {
                self.video.toggle_play(ctx);
            }
            if ctx.input(|i| i.key_pressed(Key::M)) {
                self.video.request_measure(ctx);
            }
            if ctx.input(|i| i.key_pressed(Key::E)) {
                if self.video.exporting {
                    self.video.cancel_export();
                } else {
                    self.video.start_export(ctx);
                }
            }
            return;
        }
        // Esc 分级：关对比 → 回到适应窗口
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.compare {
                self.compare = false;
            } else if (self.view_scale - 1.0).abs() > 0.01 {
                self.view_scale = 1.0;
                self.view_offset = Vec2::ZERO;
            }
        }
        // 数字 1-8 → 模式
        for (i, k) in [
            Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5,
            Key::Num6, Key::Num7, Key::Num8, Key::Num9,
        ]
        .into_iter()
        .enumerate()
        {
            if ctx.input(|i| i.key_pressed(k)) {
                self.switch_mode(Mode::GRID[i].0);
                self.toast(format!("模式 {} · {}", i + 1, Mode::GRID[i].1), false);
            }
        }
        // 0 → 第 10 个模式「去黑底 Unmult」
        if ctx.input(|i| i.key_pressed(Key::Num0)) {
            self.switch_mode(Mode::GRID[9].0);
            self.toast("模式 10 · 去黑底 Unmult", false);
        }
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        let shift = ctx.input(|i| i.modifiers.shift);
        if ctx.input(|i| i.key_pressed(Key::O)) && !ctrl {
            self.open_file_dialog();
        }
        if ctx.input(|i| i.key_pressed(Key::E)) && !ctrl {
            if self.orig.is_some() || self.mode == Mode::DualBg {
                self.export();
            } else {
                self.toast("先载入图片再导出", true);
            }
        }
        if ctx.input(|i| i.key_pressed(Key::X)) && !ctrl {
            self.clear_all();
            self.status = "已清空".into();
            self.toast("已清空", false);
        }
        if ctx.input(|i| i.key_pressed(Key::C)) && ctrl {
            self.copy_result();
        }
        if ctx.input(|i| i.key_pressed(Key::C)) && !ctrl {
            self.compare = !self.compare;
        }
        if ctx.input(|i| i.key_pressed(Key::F)) && !ctrl {
            self.view_scale = 1.0;
            self.view_offset = Vec2::ZERO;
        }
        if ctx.input(|i| i.key_pressed(Key::Z)) && ctrl {
            if shift {
                self.redo_step();
            } else {
                self.undo_step();
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Y)) && ctrl {
            self.redo_step();
        }
        if ctx.input(|i| i.key_pressed(Key::F1))
            || (ctx.input(|i| i.key_pressed(Key::Slash)) && shift)
        {
            self.help_open = !self.help_open;
        }
        if self.help_open && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.help_open = false;
        }
        // 空格速览
        self.peek = ctx.input(|i| i.key_down(Key::Space));
    }


    pub(crate) fn open_file_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("图片文件", &["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "webp"]);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(p) = dialog.pick_file() {
            match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| Img::from_bytes(&b)) {
                Ok(img) => {
                    self.set_main(img);
                    self.push_recent(p.clone());
                    self.last_dir = p.parent().map(|x| x.to_path_buf());
                    self.toast(format!("已打开 {}（替换主图）", file_name_of(&p)), false);
                }
                Err(e) => self.toast(format!("载入失败：{e}"), true),
            }
        }
    }


    pub(crate) fn load_recent(&mut self, idx: usize) {
        let Some(p) = self.recent.get(idx).cloned() else { return };
        match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| Img::from_bytes(&b)) {
            Ok(img) => {
                self.set_main(img);
                self.toast(format!("已载入 {}（替换主图）", file_name_of(&p)), false);
            }
            Err(e) => {
                self.toast(format!("文件读取失败：{e}"), true);
                self.recent.remove(idx);
            }
        }
    }
}

/// 常见视频 / 动图扩展名（用于拖放时把素材路由到视频工作区）。
pub(crate) fn is_video_file(p: &Path) -> bool {
    let Some(ext) = p.extension().and_then(|e| e.to_str()) else { return false };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "flv" | "wmv" | "m4v" | "mpg" | "mpeg"
            | "ts" | "gif" | "apng" | "ogv" | "3gp"
    )
}
