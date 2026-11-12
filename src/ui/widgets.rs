//! 通用绘制部件库：切角按钮、分段控件、滑杆、标签芯片、Toast、取色放大镜等。
//!
//! 这里全部是「无状态」的纯绘制函数，不读写 App，便于复用与单独修改。

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
use crate::ui::command_bar::*;
use crate::ui::stage::*;
use crate::update::*;
use crate::video::*;


pub(crate) fn uv01() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
}

/// 切角多边形（右下切角）

pub(crate) fn cut_polygon(rect: Rect, cut: f32) -> Vec<Pos2> {
    vec![
        pos2(rect.left(), rect.top()),
        pos2(rect.right(), rect.top()),
        pos2(rect.right(), rect.bottom() - cut),
        pos2(rect.right() - cut, rect.bottom()),
        pos2(rect.left(), rect.bottom()),
    ]
}


pub(crate) fn fill_path(p: &egui::Painter, pts: Vec<Pos2>, fill: Color32, stroke: Stroke) {
    p.add(egui::epaint::PathShape {
        points: pts,
        closed: true,
        fill,
        stroke: stroke.into(),
    });
}

// ---------------------------------------------------------------------------
// 布局度量与命令栏排版
// ---------------------------------------------------------------------------


pub(crate) fn window_btn(ui: &mut egui::Ui, glyph: &str, hover_fill: Color32, hover_text: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(42.0, 30.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 0.0, hover_fill);
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(12.0),
        if resp.hovered() { hover_text } else { C_BAR_TXT },
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 浅色细竖线

pub(crate) fn rule_v(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 12.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, C_RULE);
}

/// 深色细竖线

pub(crate) fn rule_v_dark(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 10.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, C_RULE_DARK);
}

/// 状态栏字段（在 right_to_left 里调用：先值后标签，视觉顺序为「标签 值」）

pub(crate) fn field_dark(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(value).size(11.5).strong().color(C_BAR_TXT));
    ui.add_space(5.0);
    ui.label(egui::RichText::new(label).size(10.0).color(C_INK3_DARK));
}

/// 状态栏文字链（悬停黄块墨字）

pub(crate) fn link_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(label.to_string(), FontId::proportional(10.5), C_INK3_DARK);
    let size = g.size() + vec2(12.0, 8.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, C_SIGNAL);
    }
    ui.painter().galley(
        rect.center() - g.size() / 2.0,
        g,
        if resp.hovered() { C_INK } else { C_INK3_DARK },
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 状态栏「新版本」芯片（黄底墨字，唯一强调色只给"要你看一眼"的东西）

pub(crate) fn upd_chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(label.to_string(), FontId::proportional(11.0), C_INK);
    let size = g.size() + vec2(16.0, 9.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let bg = if resp.hovered() { blend(C_SIGNAL, C_PANEL, 0.25) } else { C_SIGNAL };
    fill_path(p, cut_polygon(rect, 5.0), bg, Stroke::NONE);
    p.galley(rect.center() - g.size() / 2.0, g, C_INK);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 小色块（不可点）

pub(crate) fn swatch(ui: &mut egui::Ui, c: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, c);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
}

/// 参数分组标题：`// EN` + 中文粗体 + 右侧字段 + 发丝线（单行版，节省面板高度）

pub(crate) fn section(ui: &mut egui::Ui, en: &str, zh: &str, right: Option<String>) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            vec2(74.0, 18.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(format!("// {en}")).size(9.5).color(C_INK2));
            },
        );
        ui.label(egui::RichText::new(zh).size(12.5).strong().color(C_INK));
        if let Some(r) = right {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(r).size(9.5).color(C_INK2));
            });
        }
    });
    ui.add_space(3.0);
    ui.separator();
    ui.add_space(5.0);
}

/// 浅色切角块（筛选块：选中 = 黄）

pub(crate) fn cut_block(ui: &mut egui::Ui, label: &str, selected: bool, size: Vec2, cut: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let hover_t = ui.ctx().animate_value_with_time(
        resp.id.with("wipe"),
        if resp.hovered() { 1.0 } else { 0.0 },
        0.10,
    );
    let bg = if selected {
        C_SIGNAL
    } else if hover_t > 0.0 {
        blend(C_CHIP, C_PANEL, hover_t)
    } else {
        C_CHIP
    };
    let stroke = if selected || hover_t > 0.5 {
        Stroke::new(1.0, C_INK)
    } else {
        Stroke::new(1.0, C_RULE)
    };
    fill_path(p, cut_polygon(rect, cut), bg, stroke);
    let galley = p.layout_no_wrap(label.to_string(), FontId::proportional(11.5), C_INK);
    p.galley(rect.center() - galley.size() / 2.0, galley, C_INK);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 分段控件：选中 = 墨底反白（不用黄，避免整屏抢眼）

pub(crate) fn seg_block(ui: &mut egui::Ui, label: &str, selected: bool, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let hover_t = ui.ctx().animate_value_with_time(
        resp.id.with("seg"),
        if resp.hovered() { 1.0 } else { 0.0 },
        0.10,
    );
    let bg = if selected {
        C_INK
    } else if hover_t > 0.0 {
        blend(C_CHIP, C_PANEL, hover_t)
    } else {
        C_CHIP
    };
    let stroke = if selected {
        Stroke::NONE
    } else if hover_t > 0.5 {
        Stroke::new(1.0, C_INK)
    } else {
        Stroke::new(1.0, C_RULE)
    };
    fill_path(p, cut_polygon(rect, 5.0), bg, stroke);
    let col = if selected { C_PANEL } else { C_INK };
    let g = p.layout_no_wrap(label.to_string(), FontId::proportional(11.5), col);
    p.galley(rect.center() - g.size() / 2.0, g, col);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 勾选块：14px 方框 + 标签，开启 = 黄底墨勾

pub(crate) fn check_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let tw = text_w(ui.ctx(), label, 11.5);
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0 + 7.0 + tw + 12.0, 24.0), Sense::click());
    let p = ui.painter();
    let boxr = Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(16.0, 16.0));
    p.rect_filled(boxr, 0.0, if on { C_SIGNAL } else { C_PANEL });
    p.rect_stroke(
        boxr,
        0.0,
        Stroke::new(1.0, if resp.hovered() { C_INK } else { C_RULE }),
        StrokeKind::Inside,
    );
    if on {
        let s = Stroke::new(1.6, C_INK);
        let c = boxr.center();
        p.line_segment([pos2(c.x - 4.0, c.y), pos2(c.x - 1.0, c.y + 3.0)], s);
        p.line_segment([pos2(c.x - 1.0, c.y + 3.0), pos2(c.x + 4.0, c.y - 3.5)], s);
    }
    p.text(
        pos2(boxr.right() + 7.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.5),
        C_INK,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 主要按钮（纸面上）：墨底反白，悬停黄从左擦入

pub(crate) fn primary_block(ui: &mut egui::Ui, label: &str, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = ui.ctx().animate_value_with_time(
        resp.id.with("wipe"),
        if resp.hovered() { 1.0 } else { 0.0 },
        0.12,
    );
    let p = ui.painter();
    fill_path(p, cut_polygon(rect, CUT_BTN), C_INK, Stroke::NONE);
    if t > 0.001 {
        let sub = Rect::from_min_size(rect.left_top(), vec2(rect.width() * t, rect.height()));
        let pc = p.with_clip_rect(sub);
        fill_path(&pc, cut_polygon(rect, CUT_BTN), C_SIGNAL, Stroke::NONE);
    }
    let col = if t > 0.5 { C_INK } else { C_PANEL };
    let g = p.layout_no_wrap(label.to_string(), FontId::proportional(12.5), col);
    p.galley(rect.center() - g.size() / 2.0, g, col);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}


pub(crate) struct SliderOut {
    #[allow(dead_code)]
    pub(crate) resp: egui::Response,
    pub(crate) drag_started: bool,
    pub(crate) changed: bool,
}

/// 参数行：标签 + 右对齐数值 + 通栏细轨（点击跳值、拖动、双击复位、滚轮微调）

pub(crate) fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    step: f32,
    decimals: usize,
) -> SliderOut {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 38.0), Sense::hover());
    let lo = *range.start();
    let hi = *range.end();
    let span = (hi - lo).max(f32::EPSILON);

    let p = ui.painter();
    p.text(
        pos2(rect.left(), rect.top() + 8.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.5),
        C_INK2,
    );
    let vtxt = format!("{:.*}", decimals, *val);
    p.text(
        pos2(rect.right(), rect.top() + 8.0),
        Align2::RIGHT_CENTER,
        &vtxt,
        FontId::proportional(12.5),
        C_INK,
    );

    let rail_y = rect.top() + 25.0;
    let rail = Rect::from_min_size(pos2(rect.left(), rail_y), vec2(w, 3.0));
    let hit = Rect::from_min_size(pos2(rect.left(), rail_y - 9.0), vec2(w, 20.0));
    let resp = ui.interact(hit, ui.id().with(("slider", label)), Sense::click_and_drag());

    let mut changed = false;
    if resp.clicked() || resp.dragged() {
        if let Some(pp) = resp.interact_pointer_pos() {
            let t = ((pp.x - rail.left()) / rail.width().max(1.0)).clamp(0.0, 1.0);
            let nv = lo + t * span;
            if (nv - *val).abs() > f32::EPSILON {
                *val = nv;
                changed = true;
            }
        }
    }
    if resp.double_clicked() {
        *val = default;
        changed = true;
    }
    // 滚轮默认交给属性栏滚动；只有按住 Shift 才微调，避免"滚页面却改了数值"
    if resp.hovered() {
        let (shift, s) = ui.ctx().input(|i| (i.modifiers.shift, i.smooth_scroll_delta.y));
        if shift && s != 0.0 {
            let dir = if s > 0.0 { step * 10.0 } else { -step * 10.0 };
            *val = (*val + dir).clamp(lo, hi);
            changed = true;
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    // 数值区：按住左右拖动微调，双击复位
    let vrect = Rect::from_min_size(pos2(rect.right() - 48.0, rect.top()), vec2(48.0, 18.0));
    let vresp = ui.interact(vrect, ui.id().with(("val", label)), Sense::click_and_drag());
    if vresp.dragged() {
        let dx = vresp.drag_delta().x;
        if dx != 0.0 {
            *val = (*val + dx * span / 260.0).clamp(lo, hi);
            changed = true;
        }
    }
    if vresp.double_clicked() {
        *val = default;
        changed = true;
    }
    if vresp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    *val = val.clamp(lo, hi);

    let t = ((*val - lo) / span).clamp(0.0, 1.0);
    let p = ui.painter();
    p.rect_filled(rail, 0.0, C_CHIP);
    p.rect_filled(
        Rect::from_min_size(rail.min, vec2(rail.width() * t, rail.height())),
        0.0,
        C_INK,
    );
    let hx = rail.left() + rail.width() * t;
    let handle = Rect::from_center_size(pos2(hx, rail.center().y), vec2(9.0, 16.0));
    let hcol = if resp.hovered() || resp.dragged() { C_SIGNAL } else { C_INK };
    p.rect_filled(handle, 0.0, hcol);
    p.rect_stroke(handle, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let drag_started = resp.drag_started();
    SliderOut { resp, drag_started, changed }
}

/// 编号标签芯片（墨底白字 + 右下小切角），作为 widget 绘制

pub(crate) fn draw_tag_chip_widget(ui: &mut egui::Ui, text: &str) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(11.5), C_TAG_FG);
    let size = galley.size() + vec2(14.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().add(egui::epaint::PathShape {
        points: cut_polygon(rect, 5.0),
        closed: true,
        fill: C_TAG_BG,
        stroke: Stroke::NONE.into(),
    });
    ui.painter().galley(
        pos2(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        C_TAG_FG,
    );
}


pub(crate) fn draw_tag_chip(p: &egui::Painter, pos: Pos2, text: &str) {
    let galley = p.layout_no_wrap(text.to_string(), FontId::proportional(10.5), C_TAG_FG);
    let size = galley.size() + vec2(12.0, 7.0);
    let rect = Rect::from_min_size(pos, size);
    p.add(egui::epaint::PathShape {
        points: cut_polygon(rect, 5.0),
        closed: true,
        fill: C_TAG_BG,
        stroke: Stroke::NONE.into(),
    });
    p.galley(
        pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        C_TAG_FG,
    );
}

/// 右对齐编号芯片

pub(crate) fn draw_tag_chip_right(p: &egui::Painter, right_top: Pos2, text: &str) {
    let galley = p.layout_no_wrap(text.to_string(), FontId::proportional(10.5), C_TAG_FG);
    let size = galley.size() + vec2(12.0, 7.0);
    let rect = Rect::from_min_size(pos2(right_top.x - size.x, right_top.y), size);
    p.add(egui::epaint::PathShape {
        points: cut_polygon(rect, 5.0),
        closed: true,
        fill: C_TAG_BG,
        stroke: Stroke::NONE.into(),
    });
    p.galley(
        pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        C_TAG_FG,
    );
}

/// 对比分割线把手（墨块 + 左右白箭头）

pub(crate) fn draw_divider_handle(p: &egui::Painter, c: Pos2) {
    let r = Rect::from_center_size(c, vec2(30.0, 36.0));
    p.rect_filled(r, 0.0, C_INK);
    let s = Stroke::new(1.5, C_PANEL);
    p.line_segment([pos2(c.x - 4.0, c.y - 5.0), pos2(c.x - 8.0, c.y)], s);
    p.line_segment([pos2(c.x - 8.0, c.y), pos2(c.x - 4.0, c.y + 5.0)], s);
    p.line_segment([pos2(c.x + 4.0, c.y - 5.0), pos2(c.x + 8.0, c.y)], s);
    p.line_segment([pos2(c.x + 8.0, c.y), pos2(c.x + 4.0, c.y + 5.0)], s);
}

/// 45° 斜线纹（空状态）

pub(crate) fn hatch(p: &egui::Painter, rect: Rect) {
    let pc = p.with_clip_rect(rect);
    let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x16, 0x16, 0x16, HATCH_ALPHA));
    let mut x = rect.left();
    while x < rect.right() + rect.height() {
        pc.line_segment([pos2(x, rect.bottom()), pos2(x + rect.height(), rect.top())], stroke);
        x += 7.0;
    }
}

/// 按可用宽度截断文字并补省略号

pub(crate) fn ellipsize(p: &egui::Painter, s: &str, font: FontId, max_w: f32) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if p.layout_no_wrap(s.to_string(), font.clone(), C_INK).size().x <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let cand = format!("{}…", chars[..mid].iter().collect::<String>());
        if p.layout_no_wrap(cand, font.clone(), C_INK).size().x <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    format!("{}…", chars[..lo].iter().collect::<String>())
}

/// 屏幕坐标 → 图像像素

pub(crate) fn pixel_at(img_rect: Rect, pos: Pos2, img: &Img) -> Option<[usize; 2]> {
    if !img_rect.contains(pos) || img.w == 0 || img.h == 0 {
        return None;
    }
    let fx = ((pos.x - img_rect.left()) / img_rect.width()).clamp(0.0, 0.9999);
    let fy = ((pos.y - img_rect.top()) / img_rect.height()).clamp(0.0, 0.9999);
    Some([(fx * img.w as f32) as usize, (fy * img.h as f32) as usize])
}

/// 取色放大镜：9×9 邻域放大 + 十字线 + HEX 芯片

pub(crate) fn draw_loupe(p: &egui::Painter, viewport: Rect, hp: Pos2, img: &Img, ipx: [usize; 2]) {
    const N: i32 = 4;
    const CELL: f32 = 10.0;
    let size = (2 * N + 1) as f32 * CELL;
    let mut origin = pos2(hp.x + 20.0, hp.y - size - 14.0);
    if origin.x + size > viewport.right() - 4.0 {
        origin.x = hp.x - size - 20.0;
    }
    if origin.y < viewport.top() + 4.0 {
        origin.y = hp.y + 20.0;
    }
    origin.x = origin.x.clamp(viewport.left() + 4.0, viewport.right() - size - 4.0);
    origin.y = origin.y.clamp(viewport.top() + 4.0, viewport.bottom() - size - 30.0);
    let mut color = C_INK;
    for dy in -N..=N {
        for dx in -N..=N {
            let x = (ipx[0] as i32 + dx).clamp(0, img.w as i32 - 1) as usize;
            let y = (ipx[1] as i32 + dy).clamp(0, img.h as i32 - 1) as usize;
            let o = (y * img.w + x) * 4;
            let c = Color32::from_rgb(img.buf[o], img.buf[o + 1], img.buf[o + 2]);
            if dx == 0 && dy == 0 {
                color = c;
            }
            p.rect_filled(
                Rect::from_min_size(
                    pos2(origin.x + (dx + N) as f32 * CELL, origin.y + (dy + N) as f32 * CELL),
                    Vec2::new(CELL, CELL),
                ),
                0.0,
                c,
            );
        }
    }
    let outer = Rect::from_min_size(origin, Vec2::new(size, size));
    p.rect_stroke(outer, 0.0, Stroke::new(1.5, C_INK), StrokeKind::Middle);
    // 十字线（当前位置标记）
    let cx = origin.x + size / 2.0;
    let cy = origin.y + size / 2.0;
    p.line_segment([pos2(cx - CELL, cy), pos2(cx + CELL, cy)], Stroke::new(1.2, C_INK));
    p.line_segment([pos2(cx, cy - CELL), pos2(cx, cy + CELL)], Stroke::new(1.2, C_INK));
    // HEX 芯片
    let o = (ipx[1] * img.w + ipx[0]) * 4;
    let hex = format!(
        "#{:02X}{:02X}{:02X}",
        img.buf[o],
        img.buf[o + 1],
        img.buf[o + 2]
    );
    let _ = color;
    draw_tag_chip(p, pos2(origin.x, origin.y + size + 6.0), &hex);
}

/// Toast 通知堆栈（右上角）

pub(crate) fn draw_toasts(ctx: &egui::Context, toasts: &[Toast]) {
    if toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_TOP, [-20.0, 64.0])
        .show(ctx, |ui| {
            for t in toasts {
                egui::Frame::new()
                    .fill(C_PANEL)
                    .stroke(Stroke::new(1.0, C_INK))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // 黄旗
                            ui.label(
                                egui::RichText::new("  ")
                                    .size(13.0)
                                    .background_color(if t.warn { C_SIGNAL } else { C_CHIP }),
                            );
                            ui.label(egui::RichText::new(&t.msg).size(13.0).color(C_INK));
                        });
                    });
                ui.add_space(8.0);
            }
        });
}

/// 拖放提示（居中切角提示条）

pub(crate) fn draw_drop_hint(
    ctx: &egui::Context,
    count: usize,
    mode: Mode,
    black: &Option<Img>,
    white: &Option<Img>,
) {
    let second = if mode == Mode::DualBg {
        if black.is_none() {
            "将填入黑底槽位"
        } else if white.is_none() {
            "将填入白底槽位"
        } else {
            "槽位已满，主图将被替换"
        }
    } else {
        "将替换当前主图"
    };
    egui::Area::new(egui::Id::new("drop_hint"))
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(C_PANEL)
                .stroke(Stroke::new(2.0, C_INK))
                .inner_margin(Margin::symmetric(24, 16))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!("// 松开载入 · {} 张", count.max(1)))
                                .size(13.0)
                                .color(C_INK2),
                        );
                        ui.label(
                            egui::RichText::new(second)
                                .size(17.0)
                                .strong()
                                .color(C_INK),
                        );
                    });
                });
        });
}

// ---------------------------------------------------------------------------
// 通用辅助
// ---------------------------------------------------------------------------


pub(crate) fn update_tex(ctx: &egui::Context, slot: &mut Option<TextureHandle>, name: &str, img: &Img) {
    let ci = image_to_color_image(img, false);
    match slot {
        Some(h) => h.set(ci, TextureOptions::LINEAR),
        None => *slot = Some(ctx.load_texture(name, ci, TextureOptions::LINEAR)),
    }
}

/// 纹理数据：mask → 灰度蒙版；否则直出带透明度的 RGBA（底色改由画布在屏幕空间绘制）

pub(crate) fn image_to_color_image(img: &Img, mask: bool) -> ColorImage {
    if mask {
        let mut px = Vec::with_capacity(img.w * img.h * 4);
        for o in (0..img.buf.len()).step_by(4) {
            let v = img.buf[o + 3];
            px.extend_from_slice(&[v, v, v, 255]);
        }
        return ColorImage::from_rgba_unmultiplied([img.w, img.h], &px);
    }
    ColorImage::from_rgba_unmultiplied([img.w, img.h], &img.buf)
}

/// 屏幕空间预览底色（棋盘格只画可见区域）

pub(crate) fn paint_backdrop(p: &egui::Painter, ir: Rect, bg: PreviewBg, custom: Color32) {
    let area = ir.intersect(p.clip_rect());
    if area.width() <= 1.0 || area.height() <= 1.0 {
        return;
    }
    let pc = p.with_clip_rect(area);
    match bg {
        PreviewBg::Checker => {
            const CELL: f32 = 12.0;
            pc.rect_filled(area, 0.0, Color32::from_rgb(0xE6, 0xE6, 0xE4));
            let mut y = ir.top();
            let mut row = 0;
            while y < ir.bottom() {
                let mut x = ir.left() + if row % 2 == 0 { 0.0 } else { CELL };
                while x < ir.right() {
                    pc.rect_filled(
                        Rect::from_min_size(pos2(x, y), vec2(CELL, CELL)).intersect(area),
                        0.0,
                        C_PANEL,
                    );
                    x += CELL * 2.0;
                }
                y += CELL;
                row += 1;
            }
        }
        PreviewBg::Black => {
            pc.rect_filled(area, 0.0, Color32::BLACK);
        }
        PreviewBg::White => {
            pc.rect_filled(area, 0.0, C_PANEL);
        }
        PreviewBg::Custom => {
            pc.rect_filled(area, 0.0, custom);
        }
    }
}
