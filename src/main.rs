//! 框构 // FRAMEFORGE —— 程序入口。
//!
//! 这里只做三件事：声明模块、解析命令行、启动 eframe 窗口。
//! 其余职责按目录划分：
//! - `algo/`  纯算法（无 UI 依赖，可独立单测）
//! - `app/`   应用状态与业务动作（图片工作区、输入、更新）
//! - `ui/`    纯绘制（主题、部件、各面板、工作区布局）
//! - `video/` 视频工作区后端（FFmpeg 解码、帧处理、GIF / 视频编码、体积估算）
//! - `update.rs` 自动更新（GitHub Releases）
//!
//! 设计语言：终末地风味工业风 —— 纸 #F4F4F2 / 墨 #161616 /
//! 唯一强调黄 #FFFA00 / 切角=可点击 / 无圆角无阴影 / 深色命令栏。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod algo;
mod app;
mod ui;
mod update;
mod video;

use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    let mut args = std::env::args().skip(1);
    let arg_img = args.next();
    let arg_mode = args.next();
    let arg_bg = args.next();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("框构 // 智能抠图处理")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([940.0, 620.0])
        .with_decorations(false);
    if let Some(icon) = ui::theme::app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "框构 // 智能抠图处理",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new(cc, arg_img, arg_mode, arg_bg)) as Box<dyn eframe::App>)
        }),
    )
}
