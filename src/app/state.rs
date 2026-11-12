//! 应用状态本体：App 字段定义 + 构造 / 持久化 / 撤销重做。
//!
//! App 是所有工作区共享的顶层状态；图片与视频工作区的字段各自独立，
//! 互不覆盖。字段一律 pub(crate)，供 ui:: 与 app:: 的其它模块读写。

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


#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct Persisted {
    pub(crate) mode: Option<String>,
    pub(crate) tol: Option<f32>,
    pub(crate) feather: Option<f32>,
    pub(crate) suppress: Option<f32>,
    pub(crate) c1: Option<u32>,
    pub(crate) c2: Option<u32>,
    pub(crate) key_green: Option<u32>,
    pub(crate) key_blue: Option<u32>,
    pub(crate) ck_erode: Option<f32>,
    pub(crate) ck_smooth: Option<f32>,
    pub(crate) db_tol: Option<f32>,
    pub(crate) db_edge: Option<f32>,
    pub(crate) db_erode: Option<f32>,
    pub(crate) fx_model: Option<String>,
    pub(crate) fx_bg_type: Option<String>,
    pub(crate) fx_bg: Option<u32>,
    pub(crate) fx_ctol: Option<f32>,
    pub(crate) fx_bpoint: Option<f32>,
    pub(crate) fx_white: Option<f32>,
    pub(crate) fx_density: Option<f32>,
    pub(crate) fx_despill: Option<f32>,
    pub(crate) view_mask: Option<bool>,
    pub(crate) preview_bg: Option<String>,
    pub(crate) preview_custom: Option<u32>,
    pub(crate) compare: Option<bool>,
    pub(crate) split_x: Option<f32>,
    pub(crate) props_w: Option<f32>,
    pub(crate) skip_version: Option<String>,
    pub(crate) recent: Option<Vec<String>>,
    pub(crate) last_dir: Option<String>,
    pub(crate) last_export: Option<String>,
    /// 当前工作区（图片 / 视频）
    pub(crate) workspace: Option<String>,
    /// Unmult 去黑底参数（持久化）
    pub(crate) un_black: Option<f32>,
    pub(crate) un_white: Option<f32>,
    pub(crate) un_density: Option<f32>,
    pub(crate) un_floor: Option<f32>,
    pub(crate) un_gain: Option<f32>,
    pub(crate) un_luma: Option<bool>,
    /// 视频工作区参数（图片参数互不干扰）
    pub(crate) video: Option<crate::video::VideoSettings>,
}

// ---------------------------------------------------------------------------
// 应用状态
// ---------------------------------------------------------------------------


pub struct App {
    pub(crate) orig: Option<Img>,
    pub(crate) black: Option<Img>,
    pub(crate) white: Option<Img>,
    pub(crate) proxy_main: Option<Img>,
    pub(crate) proxy_black: Option<Img>,
    pub(crate) proxy_white: Option<Img>,
    pub(crate) fast_main: Option<Img>,
    pub(crate) fast_black: Option<Img>,
    pub(crate) fast_white: Option<Img>,
    pub(crate) fast_done: bool,
    pub(crate) pending_fast: bool,
    pub(crate) scroll_top: bool,
    pub(crate) result: Option<Img>,
    pub(crate) tex_main: Option<TextureHandle>,
    pub(crate) tex_black: Option<TextureHandle>,
    pub(crate) tex_white: Option<TextureHandle>,
    pub(crate) tex_result: Option<TextureHandle>,
    pub(crate) mode: Mode,
    pub(crate) tol: f32,
    pub(crate) feather: f32,
    pub(crate) suppress: f32,
    pub(crate) c1: Color32,
    pub(crate) c2: Color32,
    pub(crate) key_green: Color32,
    pub(crate) key_blue: Color32,
    pub(crate) ck_erode: f32,
    pub(crate) ck_smooth: f32,
    pub(crate) db_tol: f32,
    pub(crate) db_edge: f32,
    pub(crate) db_erode: f32,
    pub(crate) fx_model: FxModel,
    pub(crate) fx_bg_type: FxBgType,
    pub(crate) fx_bg: Color32,
    pub(crate) fx_ctol: f32,
    pub(crate) fx_bpoint: f32,
    pub(crate) fx_white: f32,
    pub(crate) fx_density: f32,
    pub(crate) fx_despill: f32,
    pub(crate) auto_bg: Option<Color32>,
    pub(crate) auto_tried: bool,
    pub(crate) view_mask: bool,
    pub(crate) preview_bg: PreviewBg,
    pub(crate) preview_custom: Color32,
    pub(crate) view_scale: f32,
    pub(crate) view_offset: Vec2,
    pub(crate) peek: bool,
    pub(crate) compare: bool,
    pub(crate) split_x: f32,
    pub(crate) props_w: f32,
    pub(crate) upd_tx: Sender<update::Msg>,
    pub(crate) upd_rx: Receiver<update::Msg>,
    pub(crate) upd_info: Option<update::UpdateInfo>,
    pub(crate) upd_state: UpdState,
    pub(crate) upd_open: bool,
    pub(crate) upd_busy: bool,
    pub(crate) upd_manual: bool,
    pub(crate) skip_version: Option<String>,
    pub(crate) undo: Vec<Snapshot>,
    pub(crate) redo: Vec<Snapshot>,
    pub(crate) toasts: Vec<Toast>,
    pub(crate) recent: Vec<PathBuf>,
    pub(crate) last_dir: Option<PathBuf>,
    pub(crate) last_export: Option<PathBuf>,
    pub(crate) dirty: bool,
    pub(crate) last_run: Instant,
    pub(crate) last_ms: f32,
    pub(crate) drop_paths: Vec<PathBuf>,
    pub(crate) hovering: usize,
    pub(crate) help_open: bool,
    pub(crate) last_title: String,
    pub(crate) status: String,
    /// Unmult 去黑底参数（图片工作区独立记忆）
    pub(crate) un_black: f32,
    pub(crate) un_white: f32,
    pub(crate) un_density: f32,
    pub(crate) un_floor: f32,
    pub(crate) un_gain: f32,
    pub(crate) un_luma: bool,
    /// 顶层工作区（图片 / 视频）
    pub(crate) workspace: Workspace,
    /// 视频工作区的独立状态（素材、参数、导出任务）
    pub(crate) video: crate::video::VideoState,
}


impl App {
    pub(crate) fn new(
        cc: &eframe::CreationContext,
        arg_img: Option<String>,
        arg_mode: Option<String>,
        arg_bg: Option<String>,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);
        let (upd_tx, upd_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            upd_tx,
            upd_rx,
            upd_info: None,
            upd_state: UpdState::Idle,
            upd_open: false,
            upd_busy: false,
            upd_manual: false,
            skip_version: None,
            orig: None,
            black: None,
            white: None,
            proxy_main: None,
            proxy_black: None,
            proxy_white: None,
            fast_main: None,
            fast_black: None,
            fast_white: None,
            fast_done: false,
            pending_fast: false,
            scroll_top: false,
            result: None,
            tex_main: None,
            tex_black: None,
            tex_white: None,
            tex_result: None,
            mode: Mode::Auto,
            tol: 80.0,
            feather: 5.0,
            suppress: 60.0,
            c1: Color32::from_rgb(0, 255, 0),
            c2: Color32::from_rgb(0, 0, 255),
            key_green: Color32::from_rgb(0, 255, 0),
            key_blue: Color32::from_rgb(0, 0, 255),
            ck_erode: 0.0,
            ck_smooth: 0.0,
            db_tol: 20.0,
            db_edge: 30.0,
            db_erode: 0.0,
            fx_model: FxModel::Color,
            fx_bg_type: FxBgType::Green,
            fx_bg: Color32::from_rgb(0, 255, 0),
            fx_ctol: 3.0,
            fx_bpoint: 1.0,
            fx_white: 100.0,
            fx_density: 1.0,
            fx_despill: 20.0,
            auto_bg: None,
            auto_tried: false,
            view_mask: false,
            preview_bg: PreviewBg::Checker,
            preview_custom: Color32::from_rgb(0x37, 0x63, 0x4D),
            view_scale: 1.0,
            view_offset: Vec2::ZERO,
            peek: false,
            compare: false,
            split_x: 0.5,
            props_w: PROPS_DEF,
            undo: Vec::new(),
            redo: Vec::new(),
            toasts: Vec::new(),
            recent: Vec::new(),
            last_dir: None,
            last_export: None,
            dirty: true,
            last_run: Instant::now() - Duration::from_secs(10),
            last_ms: 0.0,
            drop_paths: Vec::new(),
            hovering: 0,
            help_open: false,
            last_title: String::new(),
            status: "未载入图片".into(),
            un_black: 0.0,
            un_white: 255.0,
            un_density: 1.0,
            un_floor: 2.0,
            un_gain: 12.0,
            un_luma: false,
            workspace: Workspace::Image,
            video: crate::video::VideoState::default(),
        };
        app.load_persisted(cc);
        // 启动时安静地查一次更新（失败不打扰）
        update::check(app.upd_tx.clone());
        if let Some(m) = arg_mode.as_deref().and_then(Mode::from_cli) {
            app.mode = m;
        }
        // CLI 指定半透明底色类型（white/black/green/blue/magenta）
        if let Some(b) = arg_bg.as_deref().and_then(|s| match s {
            "white" => Some(FxBgType::White),
            "black" => Some(FxBgType::Black),
            "green" => Some(FxBgType::Green),
            "blue" => Some(FxBgType::Blue),
            "magenta" => Some(FxBgType::Magenta),
            _ => None,
        }) {
            let preset = fx_preset_for(b);
            app.fx_bg_type = b;
            app.fx_bg = Color32::from_rgb(preset.bg[0], preset.bg[1], preset.bg[2]);
            app.fx_model = preset.model;
            app.fx_ctol = preset.color_tolerance;
            app.fx_bpoint = preset.black_point;
            app.fx_white = preset.white_point;
            app.fx_density = preset.density;
            app.fx_despill = preset.despill;
        }
        if let Some(p) = arg_img {
            match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| Img::from_bytes(&b)) {
                Ok(img) => {
                    if app.mode == Mode::DualBg {
                        match app.set_slot(Slot::Black, img) {
                            Ok(()) => app.status = format!("已载入 {} → 黑底槽", file_name_of(Path::new(&p))),
                            Err(e) => app.status = e,
                        }
                    } else {
                        app.set_main(img);
                        app.status = format!("已载入 {}", file_name_of(Path::new(&p)));
                    }
                    app.push_recent(PathBuf::from(&p));
                }
                Err(e) => app.status = format!("启动图片载入失败：{e}"),
            }
        }
        app
    }

    // ---------------- 会话持久化 ----------------


    pub(crate) fn load_persisted(&mut self, cc: &eframe::CreationContext) {
        let Some(storage) = cc.storage else { return };
        let Some(json) = storage.get_string("kuanggou_v2") else { return };
        let Ok(p) = serde_json::from_str::<Persisted>(&json) else { return };
        if let Some(m) = p.mode.as_deref().and_then(Mode::from_cli) {
            self.mode = m;
        }
        if let Some(v) = p.tol { self.tol = v; }
        if let Some(v) = p.feather { self.feather = v; }
        if let Some(v) = p.suppress { self.suppress = v; }
        if let Some(v) = p.c1 { self.c1 = color32_from(v); }
        if let Some(v) = p.c2 { self.c2 = color32_from(v); }
        if let Some(v) = p.key_green { self.key_green = color32_from(v); }
        if let Some(v) = p.key_blue { self.key_blue = color32_from(v); }
        if let Some(v) = p.ck_erode { self.ck_erode = v; }
        if let Some(v) = p.ck_smooth { self.ck_smooth = v; }
        if let Some(v) = p.db_tol { self.db_tol = v; }
        if let Some(v) = p.db_edge { self.db_edge = v; }
        if let Some(v) = p.db_erode { self.db_erode = v; }
        if let Some(v) = p.fx_model.as_deref().and_then(|s| match s {
            "color" => Some(FxModel::Color),
            "smoke" => Some(FxModel::Smoke),
            "black" => Some(FxModel::Black),
            _ => None,
        }) {
            self.fx_model = v;
        }
        if let Some(v) = p.fx_bg_type.as_deref().and_then(|s| match s {
            "blue" => Some(FxBgType::Blue),
            "green" => Some(FxBgType::Green),
            "black" => Some(FxBgType::Black),
            "white" => Some(FxBgType::White),
            "magenta" => Some(FxBgType::Magenta),
            _ => None,
        }) {
            self.fx_bg_type = v;
        }
        if let Some(v) = p.fx_bg { self.fx_bg = color32_from(v); }
        if let Some(v) = p.fx_ctol { self.fx_ctol = v; }
        if let Some(v) = p.fx_bpoint { self.fx_bpoint = v; }
        if let Some(v) = p.fx_white { self.fx_white = v; }
        if let Some(v) = p.fx_density { self.fx_density = v; }
        if let Some(v) = p.fx_despill { self.fx_despill = v; }
        if let Some(v) = p.view_mask { self.view_mask = v; }
        if let Some(v) = p.preview_bg.as_deref().and_then(|s| match s {
            "checker" => Some(PreviewBg::Checker),
            "black" => Some(PreviewBg::Black),
            "white" => Some(PreviewBg::White),
            "custom" => Some(PreviewBg::Custom),
            _ => None,
        }) {
            self.preview_bg = v;
        }
        if let Some(v) = p.preview_custom { self.preview_custom = color32_from(v); }
        if let Some(v) = p.compare { self.compare = v; }
        if let Some(v) = p.split_x { self.split_x = v.clamp(0.04, 0.96); }
        if let Some(v) = p.props_w { self.props_w = v.clamp(PROPS_MIN, PROPS_MAX); }
        if let Some(v) = p.skip_version { self.skip_version = Some(v); }
        if let Some(v) = p.recent {
            self.recent = v.into_iter().map(PathBuf::from).filter(|p| p.exists()).collect();
        }
        if let Some(v) = p.last_dir { self.last_dir = Some(PathBuf::from(v)); }
        if let Some(v) = p.last_export { self.last_export = Some(PathBuf::from(v)); }
        if let Some(v) = p.workspace.as_deref() { self.workspace = Workspace::from_key(v); }
        if let Some(v) = p.un_black { self.un_black = v; }
        if let Some(v) = p.un_white { self.un_white = v; }
        if let Some(v) = p.un_density { self.un_density = v; }
        if let Some(v) = p.un_floor { self.un_floor = v; }
        if let Some(v) = p.un_gain { self.un_gain = v; }
        if let Some(v) = p.un_luma { self.un_luma = v; }
        if let Some(v) = p.video { self.video.apply_settings(v); }
    }


    pub(crate) fn save_persisted(&self, storage: &mut dyn eframe::Storage) {
        let enc = |c: Color32| ((c.r() as u32) << 16) | ((c.g() as u32) << 8) | c.b() as u32;
        let p = Persisted {
            mode: Some(format!("{:?}", self.mode)),
            tol: Some(self.tol),
            feather: Some(self.feather),
            suppress: Some(self.suppress),
            c1: Some(enc(self.c1)),
            c2: Some(enc(self.c2)),
            key_green: Some(enc(self.key_green)),
            key_blue: Some(enc(self.key_blue)),
            ck_erode: Some(self.ck_erode),
            ck_smooth: Some(self.ck_smooth),
            db_tol: Some(self.db_tol),
            db_edge: Some(self.db_edge),
            db_erode: Some(self.db_erode),
            fx_model: Some(format!("{:?}", self.fx_model).to_lowercase()),
            fx_bg_type: Some(format!("{:?}", self.fx_bg_type).to_lowercase()),
            fx_bg: Some(enc(self.fx_bg)),
            fx_ctol: Some(self.fx_ctol),
            fx_bpoint: Some(self.fx_bpoint),
            fx_white: Some(self.fx_white),
            fx_density: Some(self.fx_density),
            fx_despill: Some(self.fx_despill),
            view_mask: Some(self.view_mask),
            preview_bg: Some(
                match self.preview_bg {
                    PreviewBg::Checker => "checker",
                    PreviewBg::Black => "black",
                    PreviewBg::White => "white",
                    PreviewBg::Custom => "custom",
                }
                .to_string(),
            ),
            preview_custom: Some(enc(self.preview_custom)),
            compare: Some(self.compare),
            split_x: Some(self.split_x),
            props_w: Some(self.props_w),
            skip_version: self.skip_version.clone(),
            recent: Some(self.recent.iter().map(|p| p.display().to_string()).collect()),
            last_dir: self.last_dir.as_ref().map(|p| p.display().to_string()),
            last_export: self.last_export.as_ref().map(|p| p.display().to_string()),
            workspace: Some(self.workspace.key().to_string()),
            un_black: Some(self.un_black),
            un_white: Some(self.un_white),
            un_density: Some(self.un_density),
            un_floor: Some(self.un_floor),
            un_gain: Some(self.un_gain),
            un_luma: Some(self.un_luma),
            video: Some(self.video.settings().clone()),
        };
        if let Ok(json) = serde_json::to_string(&p) {
            storage.set_string("kuanggou_v2", json);
        }
    }

    // ---------------- 载入 / 清空 ----------------

    /// 设置主图（拖入 / 粘贴 / 打开 / 最近文件 → 一律替换主图）

    pub(crate) fn push_recent(&mut self, p: PathBuf) {
        self.recent.retain(|x| x != &p);
        self.recent.insert(0, p);
        self.recent.truncate(5);
    }


    pub(crate) fn toast(&mut self, msg: impl Into<String>, warn: bool) {
        self.toasts.push(Toast { msg: msg.into(), warn, born: Instant::now() });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    // ---------------- 撤销 / 重做 ----------------


    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            mode: self.mode,
            tol: self.tol,
            feather: self.feather,
            suppress: self.suppress,
            c1: self.c1,
            c2: self.c2,
            key_green: self.key_green,
            key_blue: self.key_blue,
            ck_erode: self.ck_erode,
            ck_smooth: self.ck_smooth,
            db_tol: self.db_tol,
            db_edge: self.db_edge,
            db_erode: self.db_erode,
            fx_model: self.fx_model,
            fx_bg_type: self.fx_bg_type,
            fx_bg: self.fx_bg,
            fx_ctol: self.fx_ctol,
            fx_bpoint: self.fx_bpoint,
            fx_white: self.fx_white,
            fx_density: self.fx_density,
            fx_despill: self.fx_despill,
            un_black: self.un_black,
            un_white: self.un_white,
            un_density: self.un_density,
            un_floor: self.un_floor,
            un_gain: self.un_gain,
            un_luma: self.un_luma,
        }
    }


    pub(crate) fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }


    pub(crate) fn apply_snapshot(&mut self, s: Snapshot) {
        self.mode = s.mode;
        self.tol = s.tol;
        self.feather = s.feather;
        self.suppress = s.suppress;
        self.c1 = s.c1;
        self.c2 = s.c2;
        self.ck_erode = s.ck_erode;
        self.ck_smooth = s.ck_smooth;
        self.db_tol = s.db_tol;
        self.db_edge = s.db_edge;
        self.db_erode = s.db_erode;
        self.fx_model = s.fx_model;
        self.fx_bg_type = s.fx_bg_type;
        self.fx_bg = s.fx_bg;
        self.fx_ctol = s.fx_ctol;
        self.fx_bpoint = s.fx_bpoint;
        self.fx_white = s.fx_white;
        self.fx_density = s.fx_density;
        self.fx_despill = s.fx_despill;
        self.un_black = s.un_black;
        self.un_white = s.un_white;
        self.un_density = s.un_density;
        self.un_floor = s.un_floor;
        self.un_gain = s.un_gain;
        self.un_luma = s.un_luma;
        self.dirty = true;
    }


    pub(crate) fn undo_step(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.apply_snapshot(s);
            self.pending_fast = true;
            self.status = "已撤销".into();
        } else {
            self.status = "没有可撤销的参数修改".into();
        }
    }


    pub(crate) fn redo_step(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.apply_snapshot(s);
            self.pending_fast = true;
            self.status = "已恢复".into();
        } else {
            self.status = "没有可恢复的参数修改".into();
        }
    }


    pub(crate) fn switch_mode(&mut self, m: Mode) {
        if self.mode != m {
            self.push_undo();
            self.mode = m;
            // 连通域模式：沿用原站 bpset 蓝图经典参数（显示量纲 容差80 / 羽化5）
            if m == Mode::Corner {
                self.tol = 80.0;
                self.feather = 5.0;
            }
            self.dirty = true;
            self.pending_fast = true;
            self.scroll_top = true;
            self.last_run = Instant::now() - Duration::from_secs(5);
        }
    }

    // ---------------- 算法调用（预览代理 / 导出全分辨率共用） ----------------

}
