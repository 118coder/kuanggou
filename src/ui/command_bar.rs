//! 图片模式命令栏：10 个抠图模式标签 + 打开 / 粘贴 / 复制 / 导出动作。
//!
//! 排版由 cmd_layout 手工计算（窄窗自动换行），绘制走 widgets 的切角多边形。

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
use crate::ui::stage::*;
use crate::update::*;
use crate::video::*;


#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    Open,
    Paste,
    Copy,
    Export,
}


impl Act {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Act::Open => "打开",
            Act::Paste => "粘贴",
            Act::Copy => "复制",
            Act::Export => "导出 PNG",
        }
    }
    pub(crate) fn id(self) -> u8 {
        match self {
            Act::Open => 1,
            Act::Paste => 2,
            Act::Copy => 3,
            Act::Export => 4,
        }
    }
}


pub(crate) struct CmdLayout {
    pub(crate) tabs: Vec<(Mode, Rect)>,
    pub(crate) acts: Vec<(Act, Rect)>,
    pub(crate) h: f32,
}


pub(crate) fn text_w(ctx: &egui::Context, s: &str, size: f32) -> f32 {
    ctx.fonts_mut(|f| {
        f.layout_no_wrap(s.to_owned(), FontId::proportional(size), C_PANEL)
            .size()
            .x
    })
}


pub(crate) fn mode_idx(m: Mode) -> usize {
    Mode::GRID.iter().position(|(x, _)| *x == m).unwrap_or(0)
}

/// 模式的中英名与一句说明

pub(crate) fn mode_meta(m: Mode) -> (&'static str, &'static str, &'static str) {
    match m {
        Mode::Auto => (
            "SMART MATTE",
            "自动去底",
            "自动找图片边缘最常见的颜色当背景并删掉；不准时点画布改。",
        ),
        Mode::Green => ("GREEN KEY", "绿幕去底", "删掉绿色背景，保留主体。"),
        Mode::Blue => ("BLUE KEY", "蓝幕去底", "删掉蓝色背景，保留主体。"),
        Mode::White => ("WHITE FLOOD", "去白底", "从四边往里清掉白底，适合白底商品图。"),
        Mode::Corner => (
            "CORNER KEY",
            "左上角去底",
            "只删和左上角连着的背景，主体内部的同色部分保留。",
        ),
        Mode::Custom => ("CUSTOM KEY", "自选颜色", "指定一种要删掉的颜色，点画布任意位置取样。"),
        Mode::Dual => ("DUAL KEY", "双色去底", "同时删掉两种颜色，适合渐变或双色背景。"),
        Mode::DualBg => ("DUAL BACKDROP", "双底合成", "同一主体拍黑底、白底各一张，合成出透明图。"),
        Mode::Fx => ("EFFECT MATTE", "半透明保留", "保留烟雾、薄纱、光效的半透明层次。"),
        Mode::Unmult => ("UNMULT", "去黑底 Unmult", "黑底光效 / 烟雾 / 火花：按亮度反解透明度并还原原色。"),
    }
}

/// 命令栏排版：模式标签自动换行，动作固定在右上

pub(crate) fn cmd_layout(ctx: &egui::Context, origin: Pos2, w: f32) -> CmdLayout {
    const AC: [Act; 4] = [Act::Open, Act::Paste, Act::Copy, Act::Export];
    let bw: Vec<f32> = AC.iter().map(|a| text_w(ctx, a.label(), 13.0) + 26.0).collect();
    let acts_w: f32 = bw.iter().sum::<f32>() + 8.0 * 3.0;
    let pad_l = 14.0;
    let pad_r = 12.0;
    let avail = (w - pad_l - pad_r - acts_w - 18.0).max(150.0);

    let widths: Vec<f32> = Mode::GRID.iter().map(|(_, n)| text_w(ctx, n, 13.0) + 44.0).collect();
    let sum: f32 = widths.iter().sum();
    // 分组间距：自动 | 色键组 | 双背景 | 半透明
    // 分组间距：自动 | 色键组 | 双背景 | 半透明 | Unmult
    let gx = 12.0 * 4.0;
    let single = sum + gx + 8.0 * (widths.len() - 1) as f32 <= avail;
    let rows = if single { 1usize } else { (((sum + gx) / avail).ceil() as usize).max(2) };
    let per = (widths.len() + rows - 1) / rows;

    let h = rows as f32 * (30.0 + 8.0) + 16.0;
    let mut tabs = Vec::with_capacity(9);
    for r in 0..rows {
        let start = r * per;
        let end = (start + per).min(widths.len());
        if start >= end {
            break;
        }
        let row_sum: f32 = widths[start..end].iter().sum();
        let breaks = (start..end.saturating_sub(1)).filter(|i| matches!(i, 0 | 6 | 7 | 8)).count() as f32;
        let extra = breaks * 12.0;
        let n = (end - start) as f32;
        let gap = if n > 1.0 {
            ((avail - row_sum - extra) / (n - 1.0)).clamp(6.0, 44.0)
        } else {
            6.0
        };
        let mut x = 0.0;
        for i in start..end {
            let tw = widths[i];
            tabs.push((
                Mode::GRID[i].0,
                Rect::from_min_size(
                    pos2(origin.x + pad_l + x, origin.y + 12.0 + r as f32 * 38.0),
                    vec2(tw, 30.0),
                ),
            ));
            x += tw + gap;
            if matches!(i, 0 | 6 | 7 | 8) {
                x += 12.0;
            }
        }
    }

    let mut acts = Vec::with_capacity(4);
    let mut ax = origin.x + w - pad_r;
    let ay = origin.y + h / 2.0 - 15.0;
    for (i, a) in AC.iter().enumerate().rev() {
        ax -= bw[i];
        acts.push((*a, Rect::from_min_size(pos2(ax, ay), vec2(bw[i], 30.0))));
        ax -= 8.0;
    }
    acts.reverse();
    CmdLayout { tabs, acts, h }
}

// ---------------------------------------------------------------------------
// 绘制部件
// ---------------------------------------------------------------------------

/// 模式标签：深色底，选中 = 黄块墨字

pub(crate) fn paint_mode_tab(p: &egui::Painter, rect: Rect, num: usize, label: &str, selected: bool, hovered: bool) {
    if selected {
        fill_path(p, cut_polygon(rect, CUT_SMALL), C_SIGNAL, Stroke::NONE);
    } else if hovered {
        fill_path(p, cut_polygon(rect, CUT_SMALL), C_HOVER_DARK, Stroke::NONE);
    } else {
        p.line_segment(
            [pos2(rect.right() + 1.0, rect.top() + 10.0), pos2(rect.right() + 1.0, rect.bottom() - 10.0)],
            Stroke::new(1.0, C_RULE_DARK),
        );
    }
    let num_col = if selected { C_INK } else { C_INK3_DARK };
    let txt_col = if selected { C_INK } else { C_BAR_TXT };
    p.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{num:02}"),
        FontId::proportional(10.0),
        num_col,
    );
    p.text(
        pos2(rect.left() + 31.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        txt_col,
    );
}

/// 深色栏按钮：悬停黄色从左擦入，主按钮为亮底墨字

pub(crate) fn paint_dark_btn(p: &egui::Painter, rect: Rect, label: &str, t: f32, primary: bool, enabled: bool) {
    let base = if primary { C_BAR_TXT } else { Color32::TRANSPARENT };
    let line = if primary { Stroke::NONE } else { Stroke::new(1.0, C_RULE_DARK) };
    if enabled {
        fill_path(p, cut_polygon(rect, CUT_BTN), base, line);
        if t > 0.001 {
            let sub = Rect::from_min_size(rect.left_top(), vec2(rect.width() * t, rect.height()));
            let pc = p.with_clip_rect(sub);
            fill_path(&pc, cut_polygon(rect, CUT_BTN), C_SIGNAL, Stroke::NONE);
        }
    }
    let col = if !enabled {
        C_RULE_DARK
    } else if t > 0.5 {
        C_INK
    } else if primary {
        C_INK
    } else {
        C_BAR_TXT
    };
    p.text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(13.0), col);
}

/// 标题栏窗口键

impl App {
    pub(crate) fn command_bar(&mut self, ui: &mut egui::Ui, w: f32) {
        let lay = cmd_layout(ui.ctx(), ui.max_rect().min, w);
        let p = ui.painter().clone();
        p.rect_filled(Rect::from_min_size(ui.max_rect().min, vec2(w, lay.h)), 0.0, C_BAR);
        for (i, (m, rect)) in lay.tabs.iter().enumerate() {
            let resp = ui.interact(*rect, egui::Id::new(("mtab", i)), Sense::click());
            paint_mode_tab(&p, *rect, i + 1, Mode::GRID[i].1, self.mode == *m, resp.hovered());
            if resp.clicked() {
                self.switch_mode(*m);
            }
        }
        for (a, rect) in &lay.acts {
            let enabled = match a {
                Act::Export | Act::Copy => self.result.is_some(),
                _ => true,
            };
            let hovered = ui.rect_contains_pointer(*rect);
            let id = egui::Id::new(("actw", a.id()));
            let t = ui.ctx().animate_value_with_time(id, if enabled && hovered { 1.0 } else { 0.0 }, 0.12);
            let resp = ui.interact(*rect, egui::Id::new(("act", a.id())), Sense::click());
            paint_dark_btn(&p, *rect, a.label(), t, *a == Act::Export, enabled);
            if resp.clicked() && enabled {
                match a {
                    Act::Open => self.open_file_dialog(),
                    Act::Paste => self.paste_clipboard(),
                    Act::Copy => self.copy_result(),
                    Act::Export => self.export(),
                }
            }
        }
    }

    // ---------------- 状态栏 ----------------

}
