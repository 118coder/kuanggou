//! 视频工作区 UI：深色命令栏 + 预览舞台（帧画布、时间轴、取色）。
//!
//! 参数栏在 `ui/video_props.rs`，两者通过 `App::video` 这同一个状态对象协作，
//! 但各自只画自己负责的区域，互不干涉。

use crate::app::App;
use crate::ui::command_bar::{paint_dark_btn, text_w};
use crate::ui::theme::*;
use crate::ui::util::fit_size;
use crate::ui::widgets::*;
use crate::video::state::VBackdrop;
use crate::video::types::KeyMode;
use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use std::path::PathBuf;

const HEAD_H: f32 = 38.0;
const FOOT_H: f32 = 34.0;

struct VAct {
    label: String,
    id: u8,
    enabled: bool,
    primary: bool,
}

fn act(label: &str, id: u8, enabled: bool, primary: bool) -> VAct {
    VAct { label: label.to_string(), id, enabled, primary }
}

/// 视频模式的深色命令栏：左侧素材/预览动作，右侧预估与导出。
pub fn command_bar(app: &mut App, ui: &mut egui::Ui, w: f32) {
    let rect = ui.max_rect();
    let h = 56.0;
    let p = ui.painter().clone();
    p.rect_filled(Rect::from_min_size(rect.min, vec2(w, h)), 0.0, C_BAR);

    let has = app.video.has_video();
    let fmt_short = crate::video::types::OutFormat::from_key(&app.video.s.out_format).short();
    let export_label = if app.video.exporting { "取消导出".to_string() } else { format!("导出 {fmt_short}") };

    let left = [
        act("打开视频", 1, true, false),
        act(if app.video.playing { "暂停" } else { "播放" }, 2, has, false),
        act("重新取样", 3, has, false),
        act(if app.video.show_orig { "看结果" } else { "看原图" }, 4, has, false),
    ];
    let right = [
        act("精确预估", 5, has && !app.video.sampling, false),
        act(&export_label, 6, has, true),
    ];

    let widths: Vec<f32> = left.iter().chain(right.iter())
        .map(|a| text_w(ui.ctx(), &a.label, 13.0) + 26.0).collect();
    let total_left: f32 = widths[..4].iter().sum::<f32>() + 8.0 * 3.0;
    let total_right: f32 = widths[4..].iter().sum::<f32>() + 8.0;

    let mut x = rect.left() + 14.0;
    for (i, a) in left.iter().enumerate() {
        draw_act(app, ui, &p, pos2(x, rect.top() + (h - 30.0) / 2.0), widths[i], a);
        x += widths[i] + 8.0;
    }
    if w > total_left + total_right + 420.0 {
        p.text(
            pos2(x + 16.0, rect.center().y),
            Align2::LEFT_CENTER,
            "逐帧抠底 · 支持不同颜色色底 / Unmult 去黑底",
            FontId::proportional(10.5),
            C_INK3_DARK,
        );
    }
    let mut rx = rect.right() - 12.0;
    for (i, a) in right.iter().enumerate().rev() {
        rx -= widths[4 + i];
        draw_act(app, ui, &p, pos2(rx, rect.top() + (h - 30.0) / 2.0), widths[4 + i], a);
        rx -= 8.0;
    }
}

fn draw_act(app: &mut App, ui: &mut egui::Ui, p: &egui::Painter, at: Pos2, bw: f32, a: &VAct) {
    let r = Rect::from_min_size(at, vec2(bw, 30.0));
    let hovered = a.enabled && ui.rect_contains_pointer(r);
    let id = egui::Id::new(("vact_hover", a.id));
    let t = ui.ctx().animate_value_with_time(id, if hovered { 1.0 } else { 0.0 }, 0.12);
    let resp = ui.interact(r, egui::Id::new(("vact", a.id)), Sense::click());
    paint_dark_btn(p, r, &a.label, t, a.primary, a.enabled);
    if !resp.clicked() || !a.enabled { return; }
    let ctx = ui.ctx().clone();
    match a.id {
        1 => open_video_dialog(app, &ctx),
        2 => app.video.toggle_play(&ctx),
        3 => app.video.resample_auto(&ctx),
        4 => app.video.show_orig = !app.video.show_orig,
        5 => app.video.request_measure(&ctx),
        6 => {
            if app.video.exporting { app.video.cancel_export(); }
            else { app.video.start_export(&ctx); }
        }
        _ => {}
    }
}

fn open_video_dialog(app: &mut App, ctx: &egui::Context) {
    let mut dlg = rfd::FileDialog::new().add_filter(
        "视频文件",
        &["mp4", "mov", "mkv", "webm", "avi", "flv", "wmv", "m4v", "mpg", "mpeg", "gif", "apng", "png", "ts"],
    );
    if let Some(dir) = &app.video.last_dir { dlg = dlg.set_directory(dir); }
    if let Some(path) = dlg.pick_file() {
        app.video.load_video(PathBuf::from(path), ctx);
    }
}

/// 视频预览舞台：白面板 + 头 / 画布 / 时间轴。
pub fn stage(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, C_PANEL);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
    let inner = rect.shrink(1.0);
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), HEAD_H));
    let foot = Rect::from_min_size(pos2(inner.left(), inner.bottom() - FOOT_H), vec2(inner.width(), FOOT_H));
    let canvas = Rect::from_min_max(pos2(inner.left(), head.bottom()), pos2(inner.right(), foot.top()));
    p.line_segment([pos2(inner.left(), head.bottom()), pos2(inner.right(), head.bottom())], Stroke::new(1.0, C_RULE));
    p.line_segment([pos2(inner.left(), foot.top()), pos2(inner.right(), foot.top())], Stroke::new(1.0, C_RULE));

    stage_head(app, ui, head);
    stage_canvas(app, ui, canvas);
    stage_foot(app, ui, foot);
}

fn stage_head(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            draw_tag_chip_widget(ui, if app.video.show_orig { "01 / 原帧" } else { "01 / 抠底结果" });
            ui.add_space(10.0);
            let info = match &app.video.info {
                Some(i) => format!(
                    "{}×{} → 预览 {}×{}   │   {}   │   {:.1} / {:.1}s",
                    i.w, i.h, app.video.preview_w, app.video.preview_h, i.codec, app.video.time, i.duration
                ),
                None => "— × —".into(),
            };
            ui.label(egui::RichText::new(info).size(11.5).color(C_INK2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for bg in [VBackdrop::Green, VBackdrop::White, VBackdrop::Black, VBackdrop::Checker] {
                    if seg_block(ui, bg.label(), app.video.backdrop == bg, vec2(36.0, 24.0)).clicked() {
                        app.video.backdrop = bg;
                    }
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new("底色").size(10.0).color(C_INK2));
            });
        },
    );
}

fn stage_canvas(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, C_PAPER);

    let dims = app.video.frame_result.as_ref().map(|i| (i.w, i.h));
    let Some((iw, ih)) = dims else {
        empty_state(app, ui, rect);
        return;
    };
    let fit = fit_size(rect.size() - vec2(24.0, 24.0), iw as usize, ih as usize);
    let ir = Rect::from_center_size(rect.center(), fit);
    let base = app.video.backdrop_color();
    p.rect_filled(ir, 0.0, base);
    if app.video.backdrop == VBackdrop::Checker {
        let cell = 12.0f32;
        let nx = (ir.width() / cell).ceil() as i32;
        let ny = (ir.height() / cell).ceil() as i32;
        for y in 0..ny {
            for x in 0..nx {
                if (x + y) % 2 == 0 { continue; }
                let left = ir.left() + x as f32 * cell;
                let top = ir.top() + y as f32 * cell;
                let w = cell.min(ir.right() - left);
                let h = cell.min(ir.bottom() - top);
                if w > 0.0 && h > 0.0 {
                    p.rect_filled(Rect::from_min_size(pos2(left, top), vec2(w, h)), 0.0, Color32::from_rgb(0xD8, 0xD8, 0xD4));
                }
            }
        }
    }
    if let Some(tex) = app.video.active_tex() {
        p.image(tex.id(), ir, uv01(), Color32::WHITE);
    }
    p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_RULE), StrokeKind::Inside);

    // 支持取色的键色模式：点画布任意位置写入视频工作区自己的键色
    if KeyMode::from_key(&app.video.s.key_mode).needs_pick() {
        let resp = ui.interact(ir, egui::Id::new("video_pick"), Sense::click());
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                app.video.pick_color(pos, ir, ui.ctx());
            }
        }
        if resp.hovered() {
            p.text(
                pos2(ir.left() + 8.0, ir.bottom() - 10.0),
                Align2::LEFT_BOTTOM,
                "点画面取键色",
                FontId::proportional(10.0),
                C_INK2,
            );
        }
    }
}

fn empty_state(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    let p = ui.painter().clone();
    hatch(&p, rect);
    let cx = rect.center().x;
    let cy = rect.center().y;
    let ff_ok = app.video.tools.is_some();
    p.text(pos2(cx, cy - 46.0), Align2::CENTER_CENTER, "// VIDEO MATTE", FontId::proportional(11.0), C_INK2);
    p.text(pos2(cx, cy - 14.0), Align2::CENTER_CENTER, "拖入视频，或点上方「打开视频」", FontId::proportional(18.0), C_INK);
    p.text(
        pos2(cx, cy + 16.0), Align2::CENTER_CENTER,
        "绿 / 蓝 / 白 / 黑 / 洋红 / 自选色底 · Unmult 去黑底光效",
        FontId::proportional(11.5), C_INK2,
    );
    let status = if ff_ok {
        "FFmpeg 已就绪 · 可导出透明视频、GIF、APNG、PNG 序列".to_string()
    } else {
        format!("未找到 FFmpeg — {}", app.video.status)
    };
    p.text(
        pos2(cx, cy + 44.0), Align2::CENTER_CENTER, status,
        FontId::proportional(11.0),
        if ff_ok { C_INK2 } else { Color32::from_rgb(0xB0, 0x50, 0x10) },
    );
    let r = Rect::from_center_size(pos2(cx, cy + 84.0), vec2(150.0, 30.0));
    let resp = ui.interact(r, egui::Id::new("video_empty_open"), Sense::click());
    fill_path(&p, cut_polygon(r, CUT_BTN), if resp.hovered() { C_SIGNAL } else { C_PANEL }, Stroke::new(1.0, C_INK));
    p.text(r.center(), Align2::CENTER_CENTER, "打开视频文件", FontId::proportional(12.5), C_INK);
    if resp.clicked() { open_video_dialog(app, ui.ctx()); }
}

fn stage_foot(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    let p = ui.painter().clone();
    let Some(info) = app.video.info.clone() else {
        p.text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, "就绪", FontId::proportional(11.0), C_INK2);
        return;
    };
    let dur = info.duration.max(0.01);
    let t = app.video.time.clamp(0.0, dur);
    let spec = crate::video::presets::resolve_spec(&app.video.s, &info);
    let pad = 14.0;
    let bar = Rect::from_min_max(
        pos2(rect.left() + pad + 96.0, rect.center().y - 4.0),
        pos2(rect.right() - pad - 92.0, rect.center().y + 4.0),
    );
    p.rect_filled(bar, 0.0, C_CHIP);
    let frac = (t / dur).clamp(0.0, 1.0) as f32;
    p.rect_filled(Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * frac, bar.bottom())), 0.0, C_INK);
    let sx = bar.left() + bar.width() * (spec.start / dur).clamp(0.0, 1.0) as f32;
    let ex = bar.left() + bar.width() * (spec.end / dur).clamp(0.0, 1.0) as f32;
    p.rect_filled(Rect::from_min_max(pos2(sx, bar.top() - 3.0), pos2(ex, bar.top() - 1.0)), 0.0, C_SIGNAL);
    p.circle_filled(pos2(bar.left() + bar.width() * frac, bar.center().y), 5.0, C_SIGNAL);

    let resp = ui.interact(bar.expand(6.0), egui::Id::new("video_timeline"), Sense::click_and_drag());
    if resp.clicked() || resp.drag_stopped() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let f = ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
            app.video.seek(f as f64 * dur, ui.ctx(), false);
        }
    }
    p.text(
        pos2(rect.left() + pad, rect.center().y), Align2::LEFT_CENTER,
        format!("{t:.1}s / {dur:.1}s"), FontId::proportional(11.0), C_INK2,
    );
    p.text(
        pos2(rect.right() - pad, rect.center().y), Align2::RIGHT_CENTER,
        app.video.status.clone(), FontId::proportional(10.5),
        if app.video.status_warn { Color32::from_rgb(0xB0, 0x50, 0x10) } else { C_INK2 },
    );
}
