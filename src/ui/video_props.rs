//! 视频工作区右侧参数栏：键色 / Unmult / 输出规格 / GIF 高级 / 体积预估。
//!
//! 所有控件只写 `App::video.s`；`VideoState::poll` 会检测参数签名变化并自动
//! 重算预估、重新处理当前预览帧，因此这里不需要手动触发任何后台逻辑。

use crate::app::App;
use crate::ui::theme::*;
use crate::ui::widgets::*;
use crate::video::estimate::human;
use crate::video::presets::{FPS, GIF_COLORS, SIZE_LIMITS, SPECS};
use crate::video::types::{FitMode, KeyMode, LimitStrategy, OutFormat};
use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

/// 绘制视频参数栏（自带滚动区域）。
pub fn props(app: &mut App, ui: &mut egui::Ui, rect: Rect) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(18.0, 14.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    head(app, ui);
                    keying(app, ui);
                    unmult(app, ui);
                    output_spec(app, ui);
                    format_advanced(app, ui);
                    estimate_card(app, ui);
                    ffmpeg_box(app, ui);
                    ui.add_space(16.0);
                });
        },
    );
}

fn head(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("// VIDEO MATTE").size(10.0).color(C_INK2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new("No.02").size(10.0).color(C_INK2));
        });
    });
    ui.label(egui::RichText::new("视频抠底 / 导出").size(19.0).strong().color(C_INK));
    ui.add_space(2.0);
    let sub = match &app.video.info {
        Some(i) => format!("{} · {}×{} · {:.2} fps · {:.1}s",
            i.path.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(), i.w, i.h, i.fps, i.duration),
        None => "尚未载入视频".into(),
    };
    ui.label(egui::RichText::new(sub).size(11.0).color(C_INK2));
    ui.add_space(4.0);
}

fn keying(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "KEYING", "去底色", None);
    let cur = KeyMode::from_key(&app.video.s.key_mode);
    let bw = (ui.available_width() - 6.0) / 2.0;
    for pair in KeyMode::ALL.chunks(2) {
        ui.horizontal(|ui| {
            for m in pair {
                if cut_block(ui, m.label(), *m == cur, vec2(bw, 26.0), CUT_SMALL).clicked() {
                    app.video.s.key_mode = m.key().to_string();
                    if m.needs_pick() && app.video.s.key_color == 0x00FF00 {
                        // 保持上次取样值；首次切换时用预设色
                        app.video.s.key_color = rgb_to_u32(m.default_rgb());
                    }
                    app.video.reset_auto();
                }
            }
        });
    }
    if cur == KeyMode::Unmult {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Unmult：黑底光效 / 烟雾 / 火花专用，按亮度反解透明度并还原原色。实拍黑底人像请用「黑底」。").size(10.5).color(C_INK2));
    } else if cur == KeyMode::Auto {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("自动识别：取首帧边缘主色作为整段视频的统一底色，避免逐帧闪烁；不准时点「重新取样」或直接在画布上取色。").size(10.5).color(C_INK2));
    }

    if cur != KeyMode::Auto && cur != KeyMode::Unmult {
        ui.add_space(6.0);
        let v = app.video.s.key_color;
        let mut c = Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("键色").size(11.0).color(C_INK2));
            if ui.color_edit_button_srgba(&mut c).changed() {
                app.video.s.key_color = ((c.r() as u32) << 16) | ((c.g() as u32) << 8) | c.b() as u32;
            }
            ui.label(egui::RichText::new(format!("#{:06X}", app.video.s.key_color)).size(11.0).color(C_INK2));
            if link_btn(ui, "恢复预设").clicked() {
                app.video.s.key_color = rgb_to_u32(cur.default_rgb());
            }
        });
        ui.label(egui::RichText::new("提示：在左侧画面上点一下即可取实际底色").size(10.0).color(C_INK2));
    }

    ui.add_space(4.0);
    slider_row(ui, "颜色相似度", &mut app.video.s.tol, 0.0..=150.0, 80.0, 1.0, 0);
    slider_row(ui, "柔边", &mut app.video.s.feather, 0.0..=50.0, 5.0, 1.0, 0);
    slider_row(ui, "去色边", &mut app.video.s.suppress, 0.0..=100.0, 60.0, 1.0, 0);
    section(ui, "EDGE", "边缘处理", None);
    slider_row(ui, "边缘内缩", &mut app.video.s.erode, 0.0..=10.0, 0.0, 0.5, 1);
    slider_row(ui, "边缘平滑", &mut app.video.s.smooth, 0.0..=10.0, 0.0, 0.5, 1);
}

fn unmult(app: &mut App, ui: &mut egui::Ui) {
    if KeyMode::from_key(&app.video.s.key_mode) != KeyMode::Unmult { return; }
    section(ui, "UNMULT", "去黑底参数", Some("参考 AE Unmult".into()));
    slider_row(ui, "黑场", &mut app.video.s.un_black, 0.0..=64.0, 0.0, 1.0, 0);
    slider_row(ui, "白场", &mut app.video.s.un_white, 128.0..=255.0, 255.0, 1.0, 0);
    slider_row(ui, "浓淡", &mut app.video.s.un_density, 0.2..=3.0, 1.0, 0.05, 2);
    slider_row(ui, "清底", &mut app.video.s.un_floor, 0.0..=32.0, 2.0, 1.0, 0);
    slider_row(ui, "颜色补偿", &mut app.video.s.un_gain, 1.0..=16.0, 12.0, 1.0, 0);
    ui.checkbox(&mut app.video.s.un_luma, "用亮度代替最大通道求透明度");
}

fn output_spec(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "OUTPUT", "输出规格", None);
    let fmt = OutFormat::from_key(&app.video.s.out_format);
    egui::ComboBox::from_id_salt("v_out_format")
        .selected_text(fmt.label())
        .width(ui.available_width() - 4.0)
        .show_ui(ui, |ui| {
            for f in OutFormat::ALL {
                if ui.selectable_label(f == fmt, f.label()).clicked() {
                    app.video.s.out_format = f.key().to_string();
                }
            }
        });
    ui.add_space(4.0);
    egui::ComboBox::from_id_salt("v_spec_preset")
        .selected_text(SPECS.iter().find(|p| p.key == app.video.s.preset).map(|p| p.label).unwrap_or("720P 1280×720"))
        .width(ui.available_width() - 4.0)
        .show_ui(ui, |ui| {
            for p in SPECS {
                if ui.selectable_label(p.key == app.video.s.preset, format!("{} · {}", p.label, p.note)).clicked() {
                    app.video.s.preset = p.key.to_string();
                }
            }
        });
    if app.video.s.preset == "custom" {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("宽").size(11.0).color(C_INK2));
            ui.add(egui::DragValue::new(&mut app.video.s.custom_w).range(2..=8192).speed(2.0));
            ui.label(egui::RichText::new("高").size(11.0).color(C_INK2));
            ui.add(egui::DragValue::new(&mut app.video.s.custom_h).range(2..=8192).speed(2.0));
            ui.label(egui::RichText::new("px（自动取偶）").size(10.0).color(C_INK2));
        });
    }
    ui.add_space(4.0);
    let fps_label = FPS.iter().find(|(k, _)| *k == app.video.s.fps_preset).map(|(_, l)| *l).unwrap_or("自定义");
    egui::ComboBox::from_id_salt("v_fps")
        .selected_text(fps_label)
        .width(ui.available_width() - 4.0)
        .show_ui(ui, |ui| {
            for (k, l) in FPS {
                if ui.selectable_label(*k == app.video.s.fps_preset, *l).clicked() {
                    app.video.s.fps_preset = k.to_string();
                }
            }
        });
    if app.video.s.fps_preset == "custom" {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("帧率").size(11.0).color(C_INK2));
            ui.add(egui::DragValue::new(&mut app.video.s.fps_custom).range(1.0..=120.0).speed(0.5));
            ui.label(egui::RichText::new("fps").size(10.0).color(C_INK2));
        });
    }
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        let bw = (ui.available_width() - 12.0) / 3.0;
        for f in [FitMode::Contain, FitMode::Cover, FitMode::Stretch] {
            let label = match f { FitMode::Contain => "适应", FitMode::Cover => "填充", FitMode::Stretch => "拉伸" };
            if cut_block(ui, label, FitMode::from_key(&app.video.s.fit) == f, vec2(bw, 24.0), CUT_SMALL).clicked() {
                app.video.s.fit = f.key().to_string();
            }
        }
    });

    if let Some(info) = &app.video.info {
        let dur_full = info.duration;
        let dur = dur_full as f32;
        ui.add_space(4.0);
        section(ui, "TRIM", "裁剪时间段", Some(format!("共 {dur_full:.1}s")));
        slider_row(ui, "起点", &mut app.video.s.trim_start, 0.0..=dur, 0.0, 0.1, 1);
        slider_row(ui, "终点", &mut app.video.s.trim_end, 0.0..=dur, dur, 0.1, 1);
        if app.video.s.trim_end <= app.video.s.trim_start + 0.05 {
            ui.label(egui::RichText::new("终点需大于起点（0 表示到片尾）").size(10.0).color(Color32::from_rgb(0xB0, 0x50, 0x10)));
        }
        if let Some(plan) = &app.video.plan {
            ui.label(egui::RichText::new(format!(
                "实际输出 {}×{} · {:.0} fps · {} 帧 · {:.1}s",
                plan.spec.w, plan.spec.h, plan.spec.fps, plan.spec.frames, plan.spec.duration
            )).size(10.5).color(C_INK2));
        }
    }
}

fn format_advanced(app: &mut App, ui: &mut egui::Ui) {
    let fmt = OutFormat::from_key(&app.video.s.out_format);
    if fmt.is_gif() {
        section(ui, "GIF ADVANCED", "GIF 高级选项", None);
        let cur = app.video.s.gif_limit_mb;
        let sel = SIZE_LIMITS.iter().find(|(k, _)| k.parse::<f32>().unwrap_or(-1.0) == cur)
            .map(|(_, l)| *l).unwrap_or("自定义");
        egui::ComboBox::from_id_salt("v_gif_limit")
            .selected_text(format!("输出大小上限：{sel}"))
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                for (k, l) in SIZE_LIMITS {
                    if *k == "custom" {
                        if ui.selectable_label(false, "自定义（下方输入）").clicked() {
                            app.video.s.gif_limit_mb = 6.0;
                        }
                    } else if ui.selectable_label(false, *l).clicked() {
                        app.video.s.gif_limit_mb = k.parse::<f32>().unwrap_or(0.0);
                    }
                }
            });
        if cur > 0.01 {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("上限").size(11.0).color(C_INK2));
                ui.add(egui::DragValue::new(&mut app.video.s.gif_limit_mb).range(0.2..=200.0).speed(0.2).suffix(" MB"));
                ui.checkbox(&mut app.video.s.strict_limit, "严格达标（自动重编码）");
            });
            let bw = ui.available_width();
            let labels = [
                (LimitStrategy::FramesFirst, "先抽帧"),
                (LimitStrategy::ResolutionFirst, "先降尺寸"),
                (LimitStrategy::Balanced, "均衡"),
            ];
            let cw = (bw - 12.0) / 3.0;
            let cur_s = LimitStrategy::from_key(&app.video.s.limit_strategy);
            ui.horizontal(|ui| {
                for (s, l) in labels {
                    if cut_block(ui, l, s == cur_s, vec2(cw, 24.0), CUT_SMALL).clicked() {
                        app.video.s.limit_strategy = s.key().to_string();
                    }
                }
            });
            ui.label(egui::RichText::new("超限时自动降帧率 / 尺寸 / 色数，直到落入上限；界面设置不会被改写。").size(10.0).color(C_INK2));
        }
        ui.add_space(4.0);
        egui::ComboBox::from_id_salt("v_gif_colors")
            .selected_text(format!("{} 色", app.video.s.gif_colors))
            .width(ui.available_width() - 4.0)
            .show_ui(ui, |ui| {
                for (n, l) in GIF_COLORS {
                    if ui.selectable_label(*n == app.video.s.gif_colors, *l).clicked() {
                        app.video.s.gif_colors = *n;
                    }
                }
            });
        ui.add_space(4.0);
        ui.checkbox(&mut app.video.s.gif_dither, "抖动模拟半透明（边缘更自然，体积略大）");
        let mut th = app.video.s.alpha_threshold as f32;
        slider_row(ui, "透明阈值", &mut th, 0.0..=255.0, 96.0, 1.0, 0);
        app.video.s.alpha_threshold = th.clamp(0.0, 255.0) as u8;
        // slider_row 写回的是临时 f32，需要同步回 u8
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("循环次数").size(11.0).color(C_INK2));
            ui.add(egui::DragValue::new(&mut app.video.s.loop_count).range(0..=999).speed(1.0));
            ui.label(egui::RichText::new("0 = 无限循环").size(10.0).color(C_INK2));
        });
    } else {
        section(ui, "VIDEO ADVANCED", "视频高级选项", None);
        if fmt == OutFormat::Mp4Flat {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("目标码率").size(11.0).color(C_INK2));
                ui.add(egui::DragValue::new(&mut app.video.s.bitrate_kbps).range(200..=80_000).speed(100.0).suffix(" kbps"));
            });
            let v = app.video.s.flat_color;
            let mut c = Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("合成底色").size(11.0).color(C_INK2));
                if ui.color_edit_button_srgba(&mut c).changed() {
                    app.video.s.flat_color = ((c.r() as u32) << 16) | ((c.g() as u32) << 8) | c.b() as u32;
                }
                ui.label(egui::RichText::new("MP4 不支持透明").size(10.0).color(C_INK2));
            });
        } else if fmt == OutFormat::WebmAlpha {
            let mut crf = app.video.s.crf as f32;
            slider_row(ui, "CRF（越小越清晰）", &mut crf, 0.0..=63.0, 30.0, 1.0, 0);
            app.video.s.crf = crf.clamp(0.0, 63.0) as u32;
        } else {
            ui.label(egui::RichText::new("该格式为无损 / 近无损编码，体积主要由分辨率、帧数与画面复杂度决定。").size(10.5).color(C_INK2));
        }
    }
}

fn estimate_card(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "ESTIMATE", "导出体积预估", None);
    let h = 108.0;
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(r, 0.0, C_CHIP);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, C_RULE), egui::StrokeKind::Inside);
    let (big, sub, warn) = match (&app.video.estimate, &app.video.plan) {
        (Some(e), Some(plan)) => {
            let budget_note = match plan.budget {
                Some(b) => format!("上限 {} · 预计占比 {:.0}%", human(b), e.bytes as f64 / b as f64 * 100.0),
                None => "未设体积上限".into(),
            };
            (human(e.bytes), format!("{} ~ {} · {} · {}", human(e.low), human(e.high), e.method, budget_note),
                plan.budget.map(|b| e.bytes > b).unwrap_or(false))
        }
        _ => ("—".into(), "载入视频后自动估算".into(), false),
    };
    p.text(pos2(r.left() + 12.0, r.top() + 26.0), Align2::LEFT_CENTER,
        "预计输出体积", FontId::proportional(10.0), C_INK2);
    p.text(pos2(r.left() + 12.0, r.top() + 54.0), Align2::LEFT_CENTER,
        big, FontId::proportional(24.0), if warn { Color32::from_rgb(0xB0, 0x50, 0x10) } else { C_INK });
    p.text(pos2(r.left() + 12.0, r.top() + 78.0), Align2::LEFT_CENTER,
        sub, FontId::proportional(10.0), C_INK2);
    if let (Some(e), Some(plan)) = (&app.video.estimate, &app.video.plan) {
        if let Some(b) = plan.budget {
            let bar = Rect::from_min_max(pos2(r.left() + 12.0, r.bottom() - 12.0), pos2(r.right() - 12.0, r.bottom() - 8.0));
            p.rect_filled(bar, 0.0, C_PANEL);
            let frac = (e.bytes as f32 / b as f32).min(1.0);
            p.rect_filled(Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * frac, bar.bottom())), 0.0,
                if frac > 0.98 { Color32::from_rgb(0xC0, 0x40, 0x20) } else { C_INK });
        }
    }
    ui.add_space(6.0);
    if let Some(e) = &app.video.estimate {
        ui.label(egui::RichText::new(&e.note).size(10.0).color(C_INK2));
    }
    if let Some(plan) = &app.video.plan {
        if !plan.note.is_empty() {
            ui.label(egui::RichText::new(&plan.note).size(10.0).color(Color32::from_rgb(0x8A, 0x60, 0x00)));
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if primary_block(ui, if app.video.sampling { "试编码中…" } else { "精确预估（抽 8 帧试编码）" }, vec2(ui.available_width() - 6.0, 28.0)).clicked() {
            app.video.request_measure(ui.ctx());
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label = if app.video.exporting { "取消导出" } else { "选择位置并导出" };
        if primary_block(ui, label, vec2(ui.available_width() - 6.0, 32.0)).clicked() {
            if app.video.exporting { app.video.cancel_export(); }
            else { app.video.start_export(ui.ctx()); }
        }
    });
    if app.video.exporting {
        ui.add_space(6.0);
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(bar, 0.0, C_PANEL);
        p.rect_filled(
            Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * app.video.progress.clamp(0.0, 1.0), bar.bottom())),
            0.0, C_SIGNAL,
        );
        ui.label(egui::RichText::new(&app.video.progress_text).size(10.0).color(C_INK2));
    }
}

fn ffmpeg_box(app: &mut App, ui: &mut egui::Ui) {
    section(ui, "FFMPEG", "FFmpeg 设置", None);
    let ok = app.video.tools.is_some();
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
        ui.painter().rect_filled(dot, 0.0, if ok { Color32::from_rgb(0x20, 0xA0, 0x50) } else { Color32::from_rgb(0xC0, 0x40, 0x20) });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(if ok { "已就绪（自动探测）" } else { "未找到，需手动指定" }).size(10.5).color(C_INK2));
    });
    if let Some(t) = &app.video.tools {
        ui.label(egui::RichText::new(t.ffmpeg.display().to_string()).size(9.5).color(C_INK2));
    }
    ui.horizontal(|ui| {
        let edit_w = ui.available_width() - 76.0;
        ui.add(egui::TextEdit::singleline(&mut app.video.s.ffmpeg_path).hint_text("ffmpeg.exe 完整路径（可留空自动查找）").desired_width(edit_w));
        if link_btn(ui, "选择").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("ffmpeg.exe", &["exe"]).pick_file() {
                app.video.s.ffmpeg_path = p.display().to_string();
                app.video.relink_ffmpeg();
            }
        }
    });
    ui.label(egui::RichText::new("也可把 ffmpeg.exe / ffprobe.exe 放到本程序同目录，启动时会自动识别。").size(9.5).color(C_INK2));
}

fn rgb_to_u32(c: [u8; 3]) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}
