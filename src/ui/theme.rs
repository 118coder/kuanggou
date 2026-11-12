//! 设计令牌与全局样式：颜色、切角尺寸、布局高度、字体、图标、egui 主题。

#![allow(unused_imports)]
use eframe::egui;
use egui::{Color32, CornerRadius, FontId, Margin, Stroke, Vec2};

pub(crate) const C_PAPER: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF2);

pub(crate) const C_PANEL: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

pub(crate) const C_INK: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);

pub(crate) const C_INK2: Color32 = Color32::from_rgb(0x5C, 0x5C, 0x5C);

pub(crate) const C_RULE: Color32 = Color32::from_rgb(0xD9, 0xD9, 0xD6); // 1px 发丝线

pub(crate) const C_CHIP: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE0); // 安静填充

pub(crate) const C_SIGNAL: Color32 = Color32::from_rgb(0xFF, 0xFA, 0x00); // 唯一强调色

pub(crate) const C_TAG_BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);

pub(crate) const C_TAG_FG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

pub(crate) const C_TRI_A: Color32 = Color32::from_rgb(0xFF, 0x00, 0xF0);

pub(crate) const C_TRI_B: Color32 = Color32::from_rgb(0xFF, 0xFA, 0x00);

pub(crate) const C_TRI_C: Color32 = Color32::from_rgb(0x00, 0xFF, 0xA2);

pub(crate) const C_BAR: Color32 = Color32::from_rgb(0x16, 0x16, 0x16); // 深色命令栏

pub(crate) const C_BAR_TXT: Color32 = Color32::from_rgb(0xED, 0xED, 0xED);

pub(crate) const C_RULE_DARK: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);

pub(crate) const C_INK3_DARK: Color32 = Color32::from_rgb(0xA3, 0xA3, 0xA3);

pub(crate) const C_HOVER_DARK: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x3A);


pub(crate) const CUT_BTN: f32 = 8.0;

pub(crate) const CUT_SMALL: f32 = 7.0;

pub(crate) const HATCH_ALPHA: u8 = 56;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------


pub(crate) const TITLE_H: f32 = 34.0;

pub(crate) const TRI_H: f32 = 3.0;

pub(crate) const TAB_H: f32 = 38.0;

pub(crate) const STATUS_H: f32 = 30.0;

pub(crate) const STAGE_HEAD: f32 = 38.0;

pub(crate) const STAGE_FOOT: f32 = 30.0;

pub(crate) const PROPS_DEF: f32 = 300.0;

pub(crate) const PROPS_MIN: f32 = 264.0;

pub(crate) const PROPS_MAX: f32 = 460.0;

pub(crate) const BODY_PAD: f32 = 12.0;

pub(crate) const SPLITTER: f32 = 10.0;

/// 舞台一帧的测量结果，供脚注渲染与缩放交互复用

pub(crate) fn app_icon() -> Option<egui::IconData> {
    let img = ::image::load_from_memory(include_bytes!("../../assets/app_256.png")).ok()?.to_rgba8();
    Some(egui::IconData {
        rgba: img.as_raw().clone(),
        width: img.width(),
        height: img.height(),
    })
}

/// 安装字体：SairaWide（拉丁宽体）+ 微软雅黑（中文黑体）

pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("saira_wide".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../assets/SairaWide.ttf"))));
    let cjk_bytes = ["msyh.ttc", "msyhl.ttc", "simhei.ttf", "simsun.ttc"]
        .iter()
        .find_map(|f| std::fs::read(format!("C:\\Windows\\Fonts\\{f}")).ok());
    let cjk_ok = cjk_bytes.is_some();
    if let Some(bytes) = cjk_bytes {
        fonts
            .font_data
            .insert("cjk".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    }
    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if cjk_ok {
            fam.insert(0, "cjk".into());
        }
        fam.insert(0, "saira_wide".into());
    }
    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        if cjk_ok {
            fam.push("cjk".into());
        }
    }
    ctx.set_fonts(fonts);
}

/// 主题（终末地 · 浅色 + 全直角 + 发丝线）

pub(crate) fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.style_mut_of(egui::Theme::Light, |style| {
        let sp = &mut style.spacing;
        sp.item_spacing = Vec2::new(8.0, 6.0);
        sp.button_padding = Vec2::new(10.0, 6.0);
        sp.interact_size = Vec2::new(48.0, 26.0);
        sp.slider_rail_height = 2.0;
        sp.scroll.bar_width = 6.0;
        sp.scroll.floating = false;
        sp.scroll.bar_outer_margin = 3.0;
        sp.scroll.handle_min_length = 36.0;
        sp.scroll.content_margin = Margin::symmetric(0, 10);

        let v = &mut style.visuals;
        v.dark_mode = false;
        v.panel_fill = C_PAPER;
        v.window_fill = C_PANEL;
        v.faint_bg_color = C_CHIP;
        v.extreme_bg_color = C_CHIP; // 滚动槽/输入底：浅灰，墨色把手才看得见
        v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 1.0 };
        v.slider_trailing_fill = false;
        v.selection.bg_fill = C_INK;
        v.selection.stroke = Stroke::new(1.0, C_INK);
        v.window_corner_radius = CornerRadius::same(0);
        v.menu_corner_radius = CornerRadius::same(0);

        let non = &mut v.widgets.noninteractive;
        non.bg_fill = C_PAPER;
        non.weak_bg_fill = C_PAPER;
        non.fg_stroke = Stroke::new(1.0, C_INK2);
        non.bg_stroke = Stroke::new(1.0, C_RULE); // 发丝分隔线
        non.corner_radius = CornerRadius::same(0);
        let inact = &mut v.widgets.inactive;
        inact.bg_fill = C_PANEL;
        inact.weak_bg_fill = C_PANEL;
        inact.fg_stroke = Stroke::new(1.0, C_INK);
        inact.bg_stroke = Stroke::new(1.0, C_INK);
        inact.corner_radius = CornerRadius::same(0);
        let hov = &mut v.widgets.hovered;
        hov.bg_fill = C_SIGNAL;
        hov.weak_bg_fill = C_SIGNAL;
        hov.fg_stroke = Stroke::new(1.0, C_INK); // 悬停擦黄 + 墨字
        hov.bg_stroke = Stroke::new(1.0, C_INK);
        hov.corner_radius = CornerRadius::same(0);
        let act = &mut v.widgets.active;
        act.bg_fill = C_INK;
        act.weak_bg_fill = C_INK;
        act.fg_stroke = Stroke::new(1.0, C_PANEL);
        act.bg_stroke = Stroke::new(1.0, C_INK);
        act.corner_radius = CornerRadius::same(0);
        let open = &mut v.widgets.open;
        open.bg_fill = C_CHIP;
        open.weak_bg_fill = C_CHIP;
        open.fg_stroke = Stroke::new(1.0, C_INK);
        open.bg_stroke = Stroke::new(1.0, C_INK);
        open.corner_radius = CornerRadius::same(0);
    });
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------
