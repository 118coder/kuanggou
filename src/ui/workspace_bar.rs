//! 顶层工作区切换条：图片 / 视频两个互斥工作区。
//!
//! 切换只改变 `App::workspace`，两个工作区的素材与参数各自独立保存，
//! 切回来时原状态不变。快捷键 `W` 也可切换。

use crate::app::{App, Workspace};
use crate::ui::theme::*;
use crate::ui::widgets::*;
use eframe::egui;
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

pub const BAR_H: f32 = 38.0;

/// 画在标题栏 + 三色条下方、命令栏上方。
pub fn bar(app: &mut App, ui: &mut egui::Ui, w: f32) {
    let rect = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(Rect::from_min_size(rect.min, vec2(w, BAR_H)), 0.0, C_PAPER);
    p.line_segment(
        [pos2(rect.left(), rect.bottom() - 0.5), pos2(rect.right(), rect.bottom() - 0.5)],
        Stroke::new(1.0, C_RULE),
    );

    // 左侧眉标
    p.text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        "// WORKSPACE",
        FontId::proportional(9.5),
        C_INK2,
    );

    let mut x = rect.left() + 116.0;
    let labels = [(Workspace::Image, "01 图片抠图"), (Workspace::Video, "02 视频 / GIF")];
    for (i, (ws, label)) in labels.iter().enumerate() {
        let bw = if i == 0 { 108.0 } else { 132.0 };
        let r = Rect::from_min_size(pos2(x, rect.top() + 6.0), vec2(bw, 26.0));
        let resp = ui.interact(r, egui::Id::new(("wstab", i)), Sense::click());
        let on = app.workspace == *ws;
        let (fill, stroke, txt) = if on {
            (C_SIGNAL, Stroke::NONE, C_INK)
        } else if resp.hovered() {
            (C_CHIP, Stroke::new(1.0, C_RULE), C_INK)
        } else {
            (C_PANEL, Stroke::new(1.0, C_RULE), C_INK2)
        };
        fill_path(&p, cut_polygon(r, CUT_SMALL), fill, stroke);
        p.text(r.center(), Align2::CENTER_CENTER, *label, FontId::proportional(12.0), txt);
        if resp.clicked() && !on {
            app.workspace = *ws;
            app.scroll_top = true;
            app.peek = false;
            if *ws == Workspace::Image {
                app.dirty = true;
            }
        }
        x += bw + 8.0;
    }

    // 右侧说明
    let hint = match app.workspace {
        Workspace::Image => "单张图片 · 10 种抠底方式 · 导出 PNG",
        Workspace::Video => "视频逐帧抠底 · 透明视频 / GIF / APNG · 规格预设",
    };
    p.text(
        pos2(rect.right() - 14.0, rect.center().y),
        Align2::RIGHT_CENTER,
        hint,
        FontId::proportional(10.5),
        C_INK2,
    );
}
