//! 框构 // 智能抠图处理 —— FrameForge 抠图算法的 Rust 原生独立版
//!
//! 功能：绿幕 / 蓝幕 / 白底泛洪 / 左上角连通域 / 自定义色 / 双重色键 /
//! 双背景差分 / 半透明特效（光效、烟雾、薄纱）。
//!
//! UI 设计语言：终末地风味工业风（endfield-ui 技能，控制台型页面）
//!   纸 #F4F4F2 / 墨 #161616 / 唯一强调黄 #FFFA00（只做填充）/ 切角=可点击 /
//!   `//` 眉标 + 中英成对 / 真实数字装饰 / 1px 细线 / 无圆角无阴影 / 深色命令栏
//!
//! 交互：拖入即替换（双背景多文件填槽）、滚轮缩放+平移、双击 1:1、空格速览、
//! 取色放大镜、像素信息、参数撤销重做、双击标签复位、滚轮微调、
//! 快捷键体系、Toast、导出增强、最近文件、会话持久化、大图性能保护、
//! 自适应防抖、标题栏状态、取色视觉提示、斜线纹空状态、帮助弹窗。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use egui::{
    Align2, Color32, ColorImage, CornerRadius, FontId, Margin, Pos2, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod algo;
mod update;

use algo::{auto_bg_color, fx_preset_for, FxBgType, FxModel, FxParams, Img};

// ---------------------------------------------------------------------------
// 设计令牌（endfield-ui tokens · light + 深色命令栏）
// ---------------------------------------------------------------------------

const C_PAPER: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF2);
const C_PANEL: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const C_INK: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
const C_INK2: Color32 = Color32::from_rgb(0x5C, 0x5C, 0x5C);
const C_RULE: Color32 = Color32::from_rgb(0xD9, 0xD9, 0xD6); // 1px 发丝线
const C_CHIP: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE0); // 安静填充
const C_SIGNAL: Color32 = Color32::from_rgb(0xFF, 0xFA, 0x00); // 唯一强调色
const C_TAG_BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
const C_TAG_FG: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const C_TRI_A: Color32 = Color32::from_rgb(0xFF, 0x00, 0xF0);
const C_TRI_B: Color32 = Color32::from_rgb(0xFF, 0xFA, 0x00);
const C_TRI_C: Color32 = Color32::from_rgb(0x00, 0xFF, 0xA2);
const C_BAR: Color32 = Color32::from_rgb(0x16, 0x16, 0x16); // 深色命令栏
const C_BAR_TXT: Color32 = Color32::from_rgb(0xED, 0xED, 0xED);
const C_RULE_DARK: Color32 = Color32::from_rgb(0x35, 0x35, 0x35);
const C_INK3_DARK: Color32 = Color32::from_rgb(0xA3, 0xA3, 0xA3);
const C_HOVER_DARK: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x3A);

const CUT_BTN: f32 = 8.0;
const CUT_SMALL: f32 = 7.0;
const HATCH_ALPHA: u8 = 56;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Auto,
    Green,
    Blue,
    White,
    Corner,
    Custom,
    Dual,
    DualBg,
    Fx,
}

impl Mode {
    const GRID: [(Mode, &'static str); 9] = [
        (Mode::Auto, "自动去底"),
        (Mode::Green, "绿幕去底"),
        (Mode::Blue, "蓝幕去底"),
        (Mode::White, "去白底"),
        (Mode::Corner, "左上角去底"),
        (Mode::Custom, "自选颜色"),
        (Mode::Dual, "双色去底"),
        (Mode::DualBg, "双底合成"),
        (Mode::Fx, "半透明保留"),
    ];

    fn from_cli(s: &str) -> Option<Mode> {
        Some(match s {
            "auto" | "smart" => Mode::Auto,
            "green" => Mode::Green,
            "blue" => Mode::Blue,
            "white" => Mode::White,
            "corner" => Mode::Corner,
            "custom" => Mode::Custom,
            "dual" => Mode::Dual,
            "dualbg" => Mode::DualBg,
            "fx" => Mode::Fx,
            _ => return None,
        })
    }

    /// 对应算法里的 mode 编号；Auto/DualBg/Fx 走独立函数
    fn id(self) -> u32 {
        match self {
            Mode::Green => 0,
            Mode::Blue => 1,
            Mode::White => 2,
            Mode::Custom => 3,
            Mode::Dual => 4,
            Mode::Corner => 5,
            Mode::Auto | Mode::DualBg | Mode::Fx => u32::MAX,
        }
    }
}

/// 结果预览底色
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PreviewBg {
    Checker,
    Black,
    White,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Black,
    White,
}

/// 更新检查状态
#[derive(Clone, PartialEq)]
enum UpdState {
    Idle,
    Checking,
    Latest(String),
    Failed(String),
}

/// Toast（白底细框 + 黄旗，2.5s 自动消失）
struct Toast {
    msg: String,
    warn: bool,
    born: Instant,
}

/// 参数快照（撤销 / 重做）
#[derive(Clone, Copy, Debug)]
struct Snapshot {
    mode: Mode,
    tol: f32,
    feather: f32,
    suppress: f32,
    c1: Color32,
    c2: Color32,
    key_green: Color32,
    key_blue: Color32,
    ck_erode: f32,
    ck_smooth: f32,
    db_tol: f32,
    db_edge: f32,
    db_erode: f32,
    fx_model: FxModel,
    fx_bg_type: FxBgType,
    fx_bg: Color32,
    fx_ctol: f32,
    fx_bpoint: f32,
    fx_white: f32,
    fx_density: f32,
    fx_despill: f32,
}

/// 持久化（会话记忆）
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Persisted {
    mode: Option<String>,
    tol: Option<f32>,
    feather: Option<f32>,
    suppress: Option<f32>,
    c1: Option<u32>,
    c2: Option<u32>,
    key_green: Option<u32>,
    key_blue: Option<u32>,
    ck_erode: Option<f32>,
    ck_smooth: Option<f32>,
    db_tol: Option<f32>,
    db_edge: Option<f32>,
    db_erode: Option<f32>,
    fx_model: Option<String>,
    fx_bg_type: Option<String>,
    fx_bg: Option<u32>,
    fx_ctol: Option<f32>,
    fx_bpoint: Option<f32>,
    fx_white: Option<f32>,
    fx_density: Option<f32>,
    fx_despill: Option<f32>,
    view_mask: Option<bool>,
    preview_bg: Option<String>,
    preview_custom: Option<u32>,
    compare: Option<bool>,
    split_x: Option<f32>,
    props_w: Option<f32>,
    skip_version: Option<String>,
    recent: Option<Vec<String>>,
    last_dir: Option<String>,
    last_export: Option<String>,
}

// ---------------------------------------------------------------------------
// 应用状态
// ---------------------------------------------------------------------------

struct App {
    orig: Option<Img>,
    black: Option<Img>,
    white: Option<Img>,
    proxy_main: Option<Img>,
    proxy_black: Option<Img>,
    proxy_white: Option<Img>,
    fast_main: Option<Img>,
    fast_black: Option<Img>,
    fast_white: Option<Img>,
    fast_done: bool,
    pending_fast: bool,
    scroll_top: bool,
    result: Option<Img>,
    tex_main: Option<TextureHandle>,
    tex_black: Option<TextureHandle>,
    tex_white: Option<TextureHandle>,
    tex_result: Option<TextureHandle>,
    mode: Mode,
    tol: f32,
    feather: f32,
    suppress: f32,
    c1: Color32,
    c2: Color32,
    key_green: Color32,
    key_blue: Color32,
    ck_erode: f32,
    ck_smooth: f32,
    db_tol: f32,
    db_edge: f32,
    db_erode: f32,
    fx_model: FxModel,
    fx_bg_type: FxBgType,
    fx_bg: Color32,
    fx_ctol: f32,
    fx_bpoint: f32,
    fx_white: f32,
    fx_density: f32,
    fx_despill: f32,
    auto_bg: Option<Color32>,
    auto_tried: bool,
    view_mask: bool,
    preview_bg: PreviewBg,
    preview_custom: Color32,
    view_scale: f32,
    view_offset: Vec2,
    peek: bool,
    compare: bool,
    split_x: f32,
    props_w: f32,
    upd_tx: Sender<update::Msg>,
    upd_rx: Receiver<update::Msg>,
    upd_info: Option<update::UpdateInfo>,
    upd_state: UpdState,
    upd_open: bool,
    upd_busy: bool,
    upd_manual: bool,
    skip_version: Option<String>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    toasts: Vec<Toast>,
    recent: Vec<PathBuf>,
    last_dir: Option<PathBuf>,
    last_export: Option<PathBuf>,
    dirty: bool,
    last_run: Instant,
    last_ms: f32,
    drop_paths: Vec<PathBuf>,
    hovering: usize,
    help_open: bool,
    last_title: String,
    status: String,
}

impl App {
    fn new(
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

    fn load_persisted(&mut self, cc: &eframe::CreationContext) {
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
    }

    fn save_persisted(&self, storage: &mut dyn eframe::Storage) {
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
        };
        if let Ok(json) = serde_json::to_string(&p) {
            storage.set_string("kuanggou_v2", json);
        }
    }

    // ---------------- 载入 / 清空 ----------------

    /// 设置主图（拖入 / 粘贴 / 打开 / 最近文件 → 一律替换主图）
    fn set_main(&mut self, img: Img) {
        // 预览代理：>2.2MP 降到长边 1600（约 1.7MP）；交互代理：>0.55MP 降到长边 760
        let mp = img.megapixels();
        self.proxy_main = if mp > 2.2 { Some(img.downscale_to_long(1600)) } else { None };
        self.fast_main = if mp > 0.55 {
            Some(match self.proxy_main.as_ref() {
                Some(p) => p.downscale_to_long(760),
                None => img.downscale_to_long(760),
            })
        } else {
            None
        };
        self.fast_done = false;
        self.pending_fast = true;
        self.orig = Some(img);
        self.result = None;
        self.tex_result = None;
        self.tex_main = None; // 在 ui 阶段用代理重建
        self.auto_bg = None;
        self.auto_tried = false;
        self.view_scale = 1.0;
        self.view_offset = Vec2::ZERO;
        self.dirty = true;
    }

    fn set_slot(&mut self, slot: Slot, img: Img) -> Result<(), String> {
        let other = match slot {
            Slot::Black => &self.white,
            Slot::White => &self.black,
        };
        if let Some(o) = other {
            if o.w != img.w || o.h != img.h {
                return Err("两张图尺寸不一致".into());
            }
        }
        match slot {
            Slot::Black => {
                let mp = img.megapixels();
                self.proxy_black = if mp > 2.2 { Some(img.downscale_to_long(1600)) } else { None };
                self.fast_black = if mp > 0.55 {
                    Some(match self.proxy_black.as_ref() {
                        Some(p) => p.downscale_to_long(760),
                        None => img.downscale_to_long(760),
                    })
                } else {
                    None
                };
                self.black = Some(img);
                self.tex_black = None;
            }
            Slot::White => {
                let mp = img.megapixels();
                self.proxy_white = if mp > 2.2 { Some(img.downscale_to_long(1600)) } else { None };
                self.fast_white = if mp > 0.55 {
                    Some(match self.proxy_white.as_ref() {
                        Some(p) => p.downscale_to_long(760),
                        None => img.downscale_to_long(760),
                    })
                } else {
                    None
                };
                self.white = Some(img);
                self.tex_white = None;
            }
        }
        self.dirty = true;
        Ok(())
    }

    fn clear_all(&mut self) {
        self.orig = None;
        self.black = None;
        self.white = None;
        self.proxy_main = None;
        self.proxy_black = None;
        self.proxy_white = None;
        self.fast_main = None;
        self.fast_black = None;
        self.fast_white = None;
        self.fast_done = false;
        self.result = None;
        self.tex_main = None;
        self.tex_black = None;
        self.tex_white = None;
        self.tex_result = None;
        self.auto_bg = None;
        self.view_scale = 1.0;
        self.view_offset = Vec2::ZERO;
        self.dirty = true;
    }

    fn fx_reset_params(&mut self) {
        // 原站 Xe(bg)：按底色类型复位参数，保留当前模型（黑底强制 Black）
        let preset = fx_preset_for(self.fx_bg_type);
        self.fx_ctol = preset.color_tolerance;
        self.fx_bpoint = preset.black_point;
        self.fx_white = preset.white_point;
        self.fx_density = preset.density;
        self.fx_despill = preset.despill;
        if preset.model == FxModel::Black {
            self.fx_model = FxModel::Black;
        }
        self.dirty = true;
    }

    /// 底色预设：自动取样 + 参数复位（原站 j(u)）
    fn fx_apply_bg(&mut self, bg: FxBgType) {
        let src = self.proxy_main.as_ref().or(self.orig.as_ref());
        let sampled = src.and_then(|m| auto_bg_color(m, bg));
        self.fx_bg_type = bg;
        self.fx_bg = match sampled {
            Some(c) => Color32::from_rgb(c[0], c[1], c[2]),
            None => Color32::from_rgb(bg.pure_color()[0], bg.pure_color()[1], bg.pure_color()[2]),
        };
        let preset = fx_preset_for(bg);
        self.fx_model = preset.model;
        self.fx_ctol = preset.color_tolerance;
        self.fx_bpoint = preset.black_point;
        self.fx_white = preset.white_point;
        self.fx_density = preset.density;
        self.fx_despill = preset.despill;
        self.dirty = true;
        let [r, g, b] = match sampled {
            Some(c) => c,
            None => bg.pure_color(),
        };
        self.toast(
            format!("{} · 已取样底色 #{r:02X}{g:02X}{b:02X}", bg.label()),
            false,
        );
    }

    fn push_recent(&mut self, p: PathBuf) {
        self.recent.retain(|x| x != &p);
        self.recent.insert(0, p);
        self.recent.truncate(5);
    }

    fn toast(&mut self, msg: impl Into<String>, warn: bool) {
        self.toasts.push(Toast { msg: msg.into(), warn, born: Instant::now() });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    // ---------------- 撤销 / 重做 ----------------

    fn snapshot(&self) -> Snapshot {
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
        }
    }

    fn push_undo(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn apply_snapshot(&mut self, s: Snapshot) {
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
        self.dirty = true;
    }

    fn undo_step(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.apply_snapshot(s);
            self.pending_fast = true;
            self.status = "已撤销".into();
        } else {
            self.status = "没有可撤销的参数修改".into();
        }
    }

    fn redo_step(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.apply_snapshot(s);
            self.pending_fast = true;
            self.status = "已恢复".into();
        } else {
            self.status = "没有可恢复的参数修改".into();
        }
    }

    fn switch_mode(&mut self, m: Mode) {
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

    fn run_chroma_on(&self, buf: &mut [u8], w: u32, h: u32) -> Result<(), String> {
        let c1 = rgb_of(match self.mode {
            Mode::Green => self.key_green,
            Mode::Blue => self.key_blue,
            _ => self.c1,
        });
        let c2 = rgb_of(self.c2);
        // 显示量纲 → 算法量纲：容差 0..150 → 0..100，羽化 0..50 → 0..100
        let tol_i = ((self.tol * 2.0 / 3.0) as u32).clamp(0, 100);
        let fea_i = ((self.feather * 2.0) as u32).clamp(0, 100);
        algo::chroma_key(buf, w, h, self.mode.id(), tol_i, fea_i, self.suppress as u32, c1, c2)?;
        algo::erode_alpha(buf, w, h, self.ck_erode);
        algo::smooth_alpha(buf, w, h, self.ck_smooth);
        Ok(())
    }

    fn run_fx_on(&self, buf: &mut [u8], w: u32, h: u32) -> Result<(), String> {
        let bg: [u8; 3] = if self.mode == Mode::Auto {
            self.auto_bg.map(|c| rgb_of(c)).unwrap_or([255, 255, 255])
        } else {
            rgb_of(self.fx_bg)
        };
        let params = FxParams {
            model: self.fx_model,
            bg,
            color_tolerance: self.fx_ctol,
            black_point: self.fx_bpoint,
            white_point: self.fx_white,
            density: self.fx_density,
            despill: self.fx_despill,
        };
        algo::fx_matte(buf, w, h, &params);
        Ok(())
    }

    fn run_dualbg_on(&self, blk: &mut [u8], wht: &[u8], w: u32, h: u32) -> Result<(), String> {
        algo::double_bg(blk, wht, w, h, self.db_tol as u32, self.db_edge as u32, self.db_erode as u32)
    }

    // ---------------- 处理 ----------------

    fn refresh(&mut self, ctx: &egui::Context, fast: bool) {
        self.dirty = false;
        self.last_run = Instant::now();
        // 自动识别只跑一次，必须用完整代理，不能吃快代理
        let auto_pending = self.mode == Mode::Auto && self.auto_bg.is_none() && !self.auto_tried;
        let fast = fast && !auto_pending;
        self.pending_fast = false;
        let t0 = Instant::now();

        match self.mode {
            Mode::DualBg => {
                let src_b = if fast { self.fast_black.as_ref() } else { None }
                    .or(self.proxy_black.as_ref())
                    .or(self.black.as_ref());
                let src_w = if fast { self.fast_white.as_ref() } else { None }
                    .or(self.proxy_white.as_ref())
                    .or(self.white.as_ref());
                let (Some(b), Some(w)) = (src_b, src_w) else {
                    self.result = None;
                    self.tex_result = None;
                    return;
                };
                if b.w != w.w || b.h != w.h {
                    self.status = "黑底图与白底图尺寸不一致，无法差分".into();
                    self.result = None;
                    return;
                }
                let mut out = b.buf.clone();
                if let Err(e) = self.run_dualbg_on(&mut out, &w.buf, b.w as u32, b.h as u32) {
                    self.status = e;
                    self.result = None;
                    return;
                }
                self.result = Some(Img::from_raw(b.w, b.h, out));
            }
            Mode::Fx | Mode::Auto => {
                // 智能抠底：首次处理时从边框环识别底色与推荐模型
                if self.mode == Mode::Auto && self.auto_bg.is_none() && !self.auto_tried {
                    self.auto_tried = true; // 只识别一次，避免每次刷新重采样
                    if let Some(src) = self.proxy_main.as_ref().or(self.orig.as_ref()) {
                        let probe = src.clone();
                        if let Some((c, model)) = algo::detect_bg_auto(&probe) {
                            self.auto_bg = Some(Color32::from_rgb(c[0], c[1], c[2]));
                            self.fx_model = model;
                            let [r, g, b] = c;
                            self.status = format!("已识别底色 #{r:02X}{g:02X}{b:02X}");
                        } else {
                            self.status = "未识别到纯色底，可改用色键模式或点击原图取色".into();
                        }
                    }
                }
                let src = if fast { self.fast_main.as_ref() } else { None }
                    .or(self.proxy_main.as_ref())
                    .or(self.orig.as_ref());
                let Some(o) = src else {
                    self.result = None;
                    self.tex_result = None;
                    return;
                };
                let mut buf = o.buf.clone();
                if let Err(e) = self.run_fx_on(&mut buf, o.w as u32, o.h as u32) {
                    self.status = e;
                    self.result = None;
                    return;
                }
                self.result = Some(Img::from_raw(o.w, o.h, buf));
            }
            _ => {
                let src = if fast { self.fast_main.as_ref() } else { None }
                    .or(self.proxy_main.as_ref())
                    .or(self.orig.as_ref());
                let Some(o) = src else {
                    self.result = None;
                    self.tex_result = None;
                    return;
                };
                let mut buf = o.buf.clone();
                if let Err(e) = self.run_chroma_on(&mut buf, o.w as u32, o.h as u32) {
                    self.status = e;
                    self.result = None;
                    return;
                }
                self.result = Some(Img::from_raw(o.w, o.h, buf));
            }
        }

        self.last_ms = t0.elapsed().as_secs_f32() * 1000.0;
        if self.status.trim().is_empty() {
            self.status = "参数已应用".into();
        }
        self.fast_done = fast;
        self.update_result_tex(ctx);
    }

    fn update_result_tex(&mut self, ctx: &egui::Context) {
        let Some(res) = &self.result else {
            self.tex_result = None;
            return;
        };
        let ci = image_to_color_image(res, self.view_mask);
        let opts = if self.view_scale >= 3.0 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
        match &mut self.tex_result {
            Some(h) => h.set(ci, opts),
            None => self.tex_result = Some(ctx.load_texture("result", ci, opts)),
        }
    }

    // ---------------- 导出 / 复制 / 定位 ----------------

    /// 全分辨率处理（导出用），返回 (w, h, rgba)
    fn export_to_buffer(&self) -> Result<(usize, usize, Vec<u8>), String> {
        match self.mode {
            Mode::DualBg => {
                let (Some(b), Some(w)) = (&self.black, &self.white) else {
                    return Err("双背景差分需要黑底和白底两张图".into());
                };
                if b.w != w.w || b.h != w.h {
                    return Err("黑底图与白底图尺寸不一致".into());
                }
                let mut out = b.buf.clone();
                self.run_dualbg_on(&mut out, &w.buf, b.w as u32, b.h as u32)?;
                Ok((b.w, b.h, out))
            }
            Mode::Fx | Mode::Auto => {
                let o = self.orig.as_ref().ok_or("请先载入原图")?;
                let mut buf = o.buf.clone();
                self.run_fx_on(&mut buf, o.w as u32, o.h as u32)?;
                Ok((o.w, o.h, buf))
            }
            _ => {
                let o = self.orig.as_ref().ok_or("请先载入原图")?;
                let mut buf = o.buf.clone();
                self.run_chroma_on(&mut buf, o.w as u32, o.h as u32)?;
                Ok((o.w, o.h, buf))
            }
        }
    }

    fn export(&mut self) {
        let (w, h, buf) = match self.export_to_buffer() {
            Ok(v) => v,
            Err(e) => {
                self.toast(e, true);
                return;
            }
        };
        // 自动命名 cutout_20261112_153000.png + 记住上次导出目录
        let (y, mo, d, hh, mm, ss) = civil_now();
        let default_name = format!("cutout_{y:04}{mo:02}{d:02}_{hh:02}{mm:02}{ss:02}.png");
        let mut dialog =
            rfd::FileDialog::new().set_file_name(&default_name).add_filter("PNG 图片", &["png"]);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return };
        let path = ensure_png_ext(path);
        match image::RgbaImage::from_raw(w as u32, h as u32, buf)
            .ok_or("缓冲区尺寸不符".to_string())
            .and_then(|im| im.save(&path).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.last_dir = path.parent().map(|p| p.to_path_buf());
                self.last_export = Some(path.clone());
                self.toast(format!("已导出 {}", file_name_of(&path)), false);
                self.status = format!("已导出 {}", path.display());
            }
            Err(e) => {
                self.toast(format!("导出失败：{e}"), true);
                self.status = format!("导出失败：{e}");
            }
        }
    }

    fn copy_result(&mut self) {
        let Some(res) = &self.result else {
            self.toast("还没有结果可复制", true);
            return;
        };
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                let data = arboard::ImageData {
                    width: res.w,
                    height: res.h,
                    bytes: res.buf.clone().into(),
                };
                match cb.set_image(data) {
                    Ok(()) => self.toast(format!("结果已复制到剪贴板（{}×{}）", res.w, res.h), false),
                    Err(e) => self.toast(format!("复制失败：{e}"), true),
                }
            }
            Err(e) => self.toast(format!("无法访问剪贴板：{e}"), true),
        }
    }

    fn reveal_last_export(&mut self) {
        let Some(p) = self.last_export.clone() else { return };
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", p.display()))
            .spawn();
    }

    // ---------------- 输入事件 ----------------

    fn handle_input(&mut self, ctx: &egui::Context) {
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
                match f.bytes() {
                    Ok(bytes) => self.load_dropped(bytes, p),
                    Err(e) => self.toast(format!("读取拖放文件失败：{e}"), true),
                }
            }
        }

        // V / Ctrl+V 粘贴图片（直接替换主图）
        if ctx.input(|i| i.key_pressed(egui::Key::V)) {
            self.paste_clipboard();
        }

        self.handle_hotkeys(ctx);
    }

    /// 拖入路由：双背景模式多文件 → 依次填黑/白槽（多余忽略并提示）；
    /// 其余一律直接替换主图
    fn load_dropped(&mut self, bytes: Vec<u8>, path: PathBuf) {
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

    fn paste_clipboard(&mut self) {
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

    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
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

    fn open_file_dialog(&mut self) {
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

    fn load_recent(&mut self, idx: usize) {
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

// ---------------------------------------------------------------------------
// eframe App
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.save_persisted(storage);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_input(ctx);
        self.poll_updates(ctx);

        // 自适应：拖动时用快代理 + 16ms 间隔；松手后 180ms 补一次全质量
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        let want_fast = pointer_down || self.pending_fast;
        if self.dirty {
            let gap = if want_fast {
                16
            } else if self.fast_done {
                90
            } else {
                45
            };
            if self.last_run.elapsed() >= Duration::from_millis(gap) {
                self.refresh(ctx, want_fast);
            }
            ctx.request_repaint();
        } else if self.fast_done {
            if self.last_run.elapsed() >= Duration::from_millis(180) {
                self.dirty = true;
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(Duration::from_millis(40));
            }
        }

        // 窗口标题 = 文件名 + 状态
        let file_part = match self.recent.first() {
            Some(p) if self.orig.is_some() => file_name_of(p),
            _ => "智能抠图处理".into(),
        };
        let state = if self.dirty { "处理中" } else { "就绪" };
        let title = format!("框构 // {file_part} · {state}");
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // Toast 过期
        self.toasts.retain(|t| t.born.elapsed() < Duration::from_millis(2500));
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 全手工精确布局：标题栏 / 三色条 / 深色命令栏 / 舞台 + 属性栏 / 状态栏
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(0.0).fill(C_PAPER))
            .show(ui, |ui| {
                let area = ui.max_rect();
                let (l, t, w, h) = (area.left(), area.top(), area.width(), area.height());
                let cmd = cmd_layout(ui.ctx(), pos2(l, t + TITLE_H + TRI_H), w);
                let body_top = t + TITLE_H + TRI_H + cmd.h;
                let body_h = (h - TITLE_H - TRI_H - cmd.h - STATUS_H).max(240.0);

                ui.painter().rect_filled(area, 0.0, C_PAPER);

                // 标题栏
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, t), vec2(w, TITLE_H))),
                    |ui| self.title_bar(ui),
                );
                // 三色条
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, t + TITLE_H), vec2(w, TRI_H))),
                    |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(w, TRI_H), Sense::hover());
                        let p = ui.painter();
                        for (i, c) in [C_TRI_A, C_TRI_B, C_TRI_C].into_iter().enumerate() {
                            p.rect_filled(
                                Rect::from_min_max(
                                    pos2(r.left() + w * i as f32 / 3.0, r.top()),
                                    pos2(r.left() + w * (i + 1) as f32 / 3.0, r.bottom()),
                                ),
                                0.0,
                                c,
                            );
                        }
                    },
                );
                // 命令栏
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, t + TITLE_H + TRI_H), vec2(w, cmd.h))),
                    |ui| self.command_bar(ui, w),
                );

                // 属性栏宽度（舞台至少留 520）
                let props_max = PROPS_MAX.min(w - 520.0).max(PROPS_MIN);
                let props_w = self.props_w.clamp(PROPS_MIN, props_max);
                let props_rect = Rect::from_min_size(pos2(l + w - props_w, body_top), vec2(props_w, body_h));
                let stage_rect = Rect::from_min_max(
                    pos2(l + BODY_PAD, body_top + BODY_PAD),
                    pos2(l + w - props_w - SPLITTER, body_top + body_h - BODY_PAD),
                );

                ui.painter().line_segment(
                    [pos2(props_rect.left(), body_top), pos2(props_rect.left(), body_top + body_h)],
                    Stroke::new(1.0, C_RULE),
                );

                self.stage(ui, stage_rect);

                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(props_rect.shrink2(vec2(18.0, 14.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                    |ui| self.props_panel(ui),
                );

                // 可拖分隔条
                let sx = l + w - props_w;
                let hit = Rect::from_min_max(pos2(sx - SPLITTER, body_top), pos2(sx, body_top + body_h));
                let sp = ui.interact(hit, egui::Id::new("props_splitter"), Sense::click_and_drag());
                if sp.hovered() || sp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    let x = hit.center().x;
                    let w_line = if sp.dragged() { 3.0 } else { 2.0 };
                    ui.painter().rect_filled(
                        Rect::from_min_size(pos2(x - w_line / 2.0, body_top + 8.0), vec2(w_line, body_h - 16.0)),
                        0.0,
                        if sp.dragged() { C_SIGNAL } else { C_INK },
                    );
                }
                if sp.dragged() {
                    if let Some(pp) = sp.interact_pointer_pos() {
                        self.props_w = (l + w - pp.x).clamp(PROPS_MIN, props_max);
                    }
                }
                if sp.double_clicked() {
                    self.props_w = PROPS_DEF;
                }

                // 状态栏
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(l, t + h - STATUS_H), vec2(w, STATUS_H))),
                    |ui| self.footer(ui),
                );

                // 浮层
                if self.help_open {
                    self.help_window(ui.ctx());
                }
                self.update_window(ui.ctx());
                draw_toasts(ui.ctx(), &self.toasts);
                if self.hovering > 0 {
                    draw_drop_hint(ui.ctx(), self.hovering, self.mode, &self.black, &self.white);
                }
            });
    }
}

impl App {
    // ---------------- 标题栏 ----------------

    /// 自绘标题栏：品牌 + 文件名 + 窗口键
    fn title_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, C_BAR);
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.add_space(12.0);
                let (mark, _) = ui.allocate_exact_size(vec2(4.0, 14.0), Sense::hover());
                ui.painter().rect_filled(mark, 0.0, C_SIGNAL);
                ui.add_space(7.0);
                ui.label(egui::RichText::new("框构").size(14.0).strong().color(C_BAR_TXT));
                ui.add_space(7.0);
                ui.label(egui::RichText::new("// FRAMEFORGE").size(9.5).color(C_INK3_DARK));
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                        .size(9.0)
                        .color(C_INK3_DARK),
                );
                ui.add_space(13.0);
                rule_v_dark(ui);
                ui.add_space(13.0);
                let name = match self.recent.first() {
                    Some(p) if self.orig.is_some() => file_name_of(p),
                    _ => String::new(),
                };
                ui.allocate_ui_with_layout(
                    vec2((rect.width() * 0.38).clamp(110.0, 440.0), TITLE_H),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(name).size(12.5).color(C_BAR_TXT)).truncate());
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if window_btn(ui, "✕", C_SIGNAL, C_INK).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                    if window_btn(ui, if maximized { "❐" } else { "□" }, C_HOVER_DARK, C_BAR_TXT).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if window_btn(ui, "─", C_HOVER_DARK, C_BAR_TXT).clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    let (_r, drag) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                    if drag.drag_started() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag.double_clicked() {
                        let m = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!m));
                    }
                });
            },
        );
    }

    // ---------------- 深色命令栏：模式 + 动作 ----------------

    fn command_bar(&mut self, ui: &mut egui::Ui, w: f32) {
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

    fn footer(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, C_BAR);
        let proxy = match self.mode {
            Mode::DualBg => self.proxy_black.is_some() || self.proxy_white.is_some(),
            _ => self.proxy_main.is_some(),
        };
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(4.0, 4.0), Sense::hover());
                ui.painter().rect_filled(dot, 0.0, if self.dirty { C_SIGNAL } else { C_INK3_DARK });
                ui.add_space(7.0);
                ui.label(
                    egui::RichText::new(if self.dirty { "处理中" } else { "就绪" })
                        .size(11.5)
                        .strong()
                        .color(C_BAR_TXT),
                );
                ui.add_space(11.0);
                rule_v_dark(ui);
                ui.add_space(11.0);
                ui.allocate_ui_with_layout(
                    vec2((rect.width() * 0.42).clamp(120.0, 620.0), STATUS_H),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if self.status.contains("失败") || self.status.contains("错误") || self.status.contains("不一致") {
                            let (bar, _) = ui.allocate_exact_size(vec2(3.0, 12.0), Sense::hover());
                            ui.painter().rect_filled(bar, 0.0, C_SIGNAL);
                            ui.add_space(5.0);
                        }
                        ui.add(egui::Label::new(egui::RichText::new(&self.status).size(11.0).color(C_INK3_DARK)).truncate());
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if link_btn(ui, "快捷键 F1").clicked() {
                        self.help_open = true;
                    }
                    ui.add_space(12.0);
                    if let Some(info) = &self.upd_info {
                        let label = format!("新版本 v{}", info.version);
                        if upd_chip(ui, &label).clicked() {
                            self.upd_open = true;
                        }
                    } else if self.upd_state == UpdState::Checking {
                        ui.label(
                            egui::RichText::new("检查更新…")
                                .size(10.0)
                                .color(C_INK3_DARK),
                        );
                    } else if link_btn(ui, "检查更新").clicked() {
                        self.check_updates(true);
                    }
                    if self.last_export.is_some() {
                        ui.add_space(12.0);
                        if link_btn(ui, "显示文件").clicked() {
                            self.reveal_last_export();
                        }
                    }
                    if self.orig.is_some() || self.black.is_some() || self.white.is_some() {
                        ui.add_space(12.0);
                        if link_btn(ui, "清空").clicked() {
                            self.clear_all();
                            self.status = "已清空".into();
                            self.toast("已清空", false);
                        }
                    }
                    ui.add_space(12.0);
                    field_dark(ui, "耗时", &format!("{:.1} ms", self.last_ms));
                    ui.add_space(12.0);
                    field_dark(ui, "引擎", "RUST");
                    if proxy {
                        ui.add_space(12.0);
                        field_dark(ui, "预览", "代理");
                    }
                });
            },
        );
    }

    // ---------------- 属性栏 ----------------

    fn props_panel(&mut self, ui: &mut egui::Ui) {
        let mode = self.mode;
        ui.scope(|ui| {
            {
                let v = &mut ui.style_mut().visuals.widgets;
                v.inactive.bg_fill = C_INK;
                v.inactive.fg_stroke = Stroke::new(1.0, C_INK);
                v.hovered.bg_fill = C_INK;
                v.hovered.fg_stroke = Stroke::new(1.0, C_INK);
                v.active.bg_fill = C_SIGNAL;
                v.active.fg_stroke = Stroke::new(1.0, C_SIGNAL);
            }
            let reset = self.scroll_top;
            self.scroll_top = false;
            let mut sa = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
            if reset {
                sa = sa.vertical_scroll_offset(0.0);
            }
            sa.show(ui, |ui| {
                    self.props_head(ui);
                    match mode {
                        Mode::DualBg => self.dualbg_params(ui),
                        Mode::Fx => self.fx_params(ui),
                        Mode::Auto => self.auto_params(ui),
                        _ => self.chroma_params(ui),
                    }
                    ui.add_space(14.0);
                    self.recents(ui);
                    ui.add_space(10.0);
                });
        });
    }

    fn props_head(&mut self, ui: &mut egui::Ui) {
        let (en, zh, desc) = mode_meta(self.mode);
        let n = mode_idx(self.mode) + 1;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("// {en}")).size(10.0).color(C_INK2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("No.{:02}", n)).size(10.0).color(C_INK2));
            });
        });
        ui.add_space(1.0);
        ui.label(egui::RichText::new(zh).size(19.0).strong().color(C_INK));
        ui.add_space(3.0);
        ui.label(egui::RichText::new(desc).size(11.5).color(C_INK2));
        ui.add_space(9.0);
        ui.separator();
        if self.orig.is_some()
            && matches!(
                self.mode,
                Mode::Green | Mode::Blue | Mode::Custom | Mode::Dual | Mode::Fx | Mode::Auto
            )
        {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("点画布取色")
                        .size(11.0)
                        .strong()
                        .color(C_INK)
                        .background_color(C_SIGNAL),
                );
            });
        }
    }

    fn recents(&mut self, ui: &mut egui::Ui) {
        section(ui, "RECENTS", "最近文件", Some(format!("{} 条", self.recent.len())));
        if self.recent.is_empty() {
            ui.label(
                egui::RichText::new("还没有记录。打开或拖入图片后会出现在这里，点一下即可重新载入。")
                    .size(11.0)
                    .color(C_INK2),
            );
            return;
        }
        let recent = self.recent.clone();
        for (i, p) in recent.iter().enumerate() {
            self.recent_row(ui, i, p);
        }
    }

    fn recent_row(&mut self, ui: &mut egui::Ui, idx: usize, p: &Path) {
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
        let xr = Rect::from_min_size(pos2(rect.right() - 22.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        let del = ui.interact(xr, ui.id().with(("recent_del", idx)), Sense::click());
        {
            let painter = ui.painter();
            if resp.hovered() {
                painter.rect_filled(
                    Rect::from_min_size(pos2(rect.left(), rect.top() + 2.0), vec2(3.0, rect.height() - 4.0)),
                    0.0,
                    C_INK,
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let num = format!("{:02}", idx + 1);
            let ng = painter.layout_no_wrap(num, FontId::proportional(10.0), C_TAG_FG);
            let chip = Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.0), vec2(ng.size().x + 10.0, 16.0));
            painter.rect_filled(chip, 0.0, C_TAG_BG);
            painter.galley(pos2(chip.left() + 5.0, chip.center().y - ng.size().y / 2.0), ng, C_TAG_FG);
            let font = FontId::proportional(12.0);
            let text = ellipsize(painter, &file_name_of(p), font.clone(), w - chip.width() - 50.0);
            painter.text(
                pos2(chip.right() + 8.0, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                font,
                if resp.hovered() { C_INK } else { C_INK2 },
            );
            if resp.hovered() || del.hovered() {
                if del.hovered() {
                    painter.rect_filled(xr, 0.0, C_SIGNAL);
                }
                painter.text(
                    xr.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(12.0),
                    C_INK,
                );
                if del.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            painter.line_segment(
                [pos2(rect.left(), rect.bottom()), pos2(rect.right(), rect.bottom())],
                Stroke::new(1.0, C_RULE),
            );
        }
        if del.clicked() {
            self.recent.remove(idx);
            self.toast("已从最近文件移除", false);
        } else if resp.clicked() {
            self.load_recent(idx);
        }
    }

    // ---------------- 色键参数 ----------------

    fn chroma_params(&mut self, ui: &mut egui::Ui) {
        section(ui, "PARAMETERS", "去底参数", None);
        let s = slider_row(ui, "容差", &mut self.tol, 0.0..=150.0, 80.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "柔边", &mut self.feather, 0.0..=50.0, 5.0, 1.0, 0);
        self.slider_changed(&s);
        if matches!(self.mode, Mode::Green | Mode::Blue) {
            let s = slider_row(ui, "抑色", &mut self.suppress, 0.0..=100.0, 60.0, 1.0, 0);
            self.slider_changed(&s);
        }
        if matches!(self.mode, Mode::Green | Mode::Blue) {
            let is_green = self.mode == Mode::Green;
            let pure = if is_green {
                Color32::from_rgb(0, 255, 0)
            } else {
                Color32::from_rgb(0, 0, 255)
            };
            let mut key = if is_green { self.key_green } else { self.key_blue };
            section(ui, "KEY COLOR", if is_green { "绿幕颜色" } else { "蓝幕颜色" }, None);
            let mut changed = false;
            ui.horizontal(|ui| {
                if cut_block(
                    ui,
                    if is_green { "重置纯绿" } else { "重置纯蓝" },
                    false,
                    vec2(66.0, 25.0),
                    CUT_SMALL,
                )
                .clicked()
                {
                    key = pure;
                    changed = true;
                }
                if ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut key).changed() {
                    changed = true;
                }
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", key.r(), key.g(), key.b()))
                        .size(11.0)
                        .color(C_INK),
                );
            });
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(if is_green {
                    "背景绿不纯时，点一下画布里的绿幕就能取到实际颜色。"
                } else {
                    "背景蓝不纯时，点一下画布里的蓝幕就能取到实际颜色。"
                })
                .size(10.5)
                .color(C_INK2),
            );
            if changed {
                self.push_undo();
                if is_green {
                    self.key_green = key;
                } else {
                    self.key_blue = key;
                }
                self.dirty = true;
            }
        }
        if matches!(self.mode, Mode::Custom | Mode::Dual) {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("键色 1").size(11.5).color(C_INK2));
                ui.add_space(6.0);
                if ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut self.c1).changed() {
                    self.push_undo();
                    self.dirty = true;
                }
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", self.c1.r(), self.c1.g(), self.c1.b()))
                        .size(10.5)
                        .color(C_INK2),
                );
            });
        }
        if self.mode == Mode::Dual {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("键色 2").size(11.5).color(C_INK2));
                ui.add_space(6.0);
                if ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut self.c2).changed() {
                    self.push_undo();
                    self.dirty = true;
                }
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", self.c2.r(), self.c2.g(), self.c2.b()))
                        .size(10.5)
                        .color(C_INK2),
                );
            });
        }

        section(ui, "EDGE", "边缘处理", None);
        let s = slider_row(ui, "边缘内缩", &mut self.ck_erode, 0.0..=10.0, 0.0, 0.1, 1);
        self.slider_changed(&s);
        let s = slider_row(ui, "边缘平滑", &mut self.ck_smooth, 0.0..=10.0, 0.0, 0.1, 1);
        self.slider_changed(&s);
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("内缩让边缘更干净，去锯齿让边缘更柔和。")
                .size(10.5)
                .color(C_INK2),
        );
        ui.label(
            egui::RichText::new("预览和导出用同一套参数。")
                .size(10.5)
                .color(C_INK2),
        );
    }

    fn slider_changed(&mut self, s: &SliderOut) {
        if s.drag_started {
            self.push_undo();
        }
        if s.changed {
            self.dirty = true;
        }
    }

    // ---------------- 半透明参数 ----------------

    fn fx_params(&mut self, ui: &mut egui::Ui) {
        let black_bg = self.fx_bg_type == FxBgType::Black;
        section(ui, "METHOD", "处理方式", None);
        if black_bg {
            ui.horizontal(|ui| {
                swatch(ui, C_INK);
                ui.label(egui::RichText::new("黑屏光效").size(12.0).strong().color(C_INK));
            });
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new("纯黑背景直接变透明，并补回光晕颜色。适合闪光、火焰，不用取色。")
                    .size(10.5)
                    .color(C_INK2),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                if cut_block(ui, "彩色光效", self.fx_model == FxModel::Color, vec2(86.0, 26.0), CUT_SMALL).clicked() {
                    self.push_undo();
                    self.fx_model = FxModel::Color;
                    self.dirty = true;
                }
                if cut_block(ui, "烟雾 / 薄纱", self.fx_model == FxModel::Smoke, vec2(96.0, 26.0), CUT_SMALL).clicked() {
                    self.push_undo();
                    self.fx_model = FxModel::Smoke;
                    self.dirty = true;
                }
            });
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(match self.fx_model {
                    FxModel::Smoke => "保留灰白烟雾、薄纱的层次；白底会自动改用光效算法。",
                    _ => "优先保留弱光和彩色渐变。和背景同色的部分无法区分。",
                })
                .size(10.5)
                .color(C_INK2),
            );
        }

        section(ui, "KEY", "底色", None);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("底色").size(11.5).color(C_INK2));
            if !black_bg && ui.ctx().cumulative_pass_nr() > 4 && ui.color_edit_button_srgba(&mut self.fx_bg).changed() {
                self.push_undo();
                self.dirty = true;
            }
            if black_bg {
                ui.label(egui::RichText::new("纯黑，无需取色").size(10.5).color(C_INK2));
            } else {
                ui.label(
                    egui::RichText::new(format!("#{:02X}{:02X}{:02X}", self.fx_bg.r(), self.fx_bg.g(), self.fx_bg.b()))
                        .size(10.5)
                        .color(C_INK2),
                );
            }
        });
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            for bg in [FxBgType::Blue, FxBgType::Green, FxBgType::Black, FxBgType::White, FxBgType::Magenta] {
                let selected = self.fx_bg_type == bg;
                if cut_block(ui, bg.label(), selected, vec2(56.0, 25.0), CUT_SMALL).clicked() {
                    self.push_undo();
                    self.fx_apply_bg(bg);
                }
            }
            if cut_block(ui, "重置", false, vec2(48.0, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_reset_params();
                self.toast("特效参数已按底色复位", false);
            }
        });

        section(ui, "FINE TUNE", "参数调节", Some("4–5 项".into()));
        if !black_bg {
            let s = slider_row(ui, "底色范围", &mut self.fx_ctol, 0.0..=20.0, 3.0, 1.0, 0);
            self.slider_changed(&s);
        }
        let s = slider_row(ui, "背景清理", &mut self.fx_bpoint, 0.0..=20.0, 1.0, 0.1, 1);
        self.slider_changed(&s);
        let s = slider_row(ui, "主体实心度", &mut self.fx_white, 50.0..=100.0, 100.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "浓淡", &mut self.fx_density, 0.2..=3.0, 1.0, 0.05, 2);
        self.slider_changed(&s);
        if !black_bg && self.fx_bg_type != FxBgType::White {
            let s = slider_row(ui, "去色边", &mut self.fx_despill, 0.0..=100.0, 20.0, 1.0, 0);
            self.slider_changed(&s);
        }
    }

    // ---------------- 智能抠底参数 ----------------

    fn auto_params(&mut self, ui: &mut egui::Ui) {
        section(ui, "KEY", "识别底色", None);
        ui.horizontal(|ui| {
            match self.auto_bg {
                Some(c) => {
                    swatch(ui, c);
                    ui.label(
                        egui::RichText::new(format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()))
                            .size(11.0)
                            .color(C_INK),
                    );
                }
                None => {
                    ui.label(egui::RichText::new("自动").size(10.5).color(C_INK2));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.orig.is_some() {
                    if cut_block(ui, "重新识别", false, vec2(72.0, 24.0), CUT_SMALL).clicked() {
                        self.auto_bg = None;
                        self.auto_tried = false;
                        self.dirty = true;
                    }
                }
            });
        });

        section(ui, "METHOD", "处理方式", None);
        let cw = ((ui.available_width() - 8.0) / 3.0).max(64.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if cut_block(ui, "彩色光效", self.fx_model == FxModel::Color, vec2(cw, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_model = FxModel::Color;
                self.dirty = true;
            }
            if cut_block(ui, "烟雾薄纱", self.fx_model == FxModel::Smoke, vec2(cw, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_model = FxModel::Smoke;
                self.dirty = true;
            }
            if cut_block(ui, "黑屏光效", self.fx_model == FxModel::Black, vec2(cw, 25.0), CUT_SMALL).clicked() {
                self.push_undo();
                self.fx_model = FxModel::Black;
                self.dirty = true;
            }
        });

        section(ui, "FINE TUNE", "参数微调", None);
        if self.fx_model != FxModel::Black {
            let s = slider_row(ui, "底色范围", &mut self.fx_ctol, 0.0..=20.0, 3.0, 1.0, 0);
            self.slider_changed(&s);
        }
        let s = slider_row(ui, "背景清理", &mut self.fx_bpoint, 0.0..=20.0, 1.0, 0.1, 1);
        self.slider_changed(&s);
        let s = slider_row(ui, "主体实心度", &mut self.fx_white, 50.0..=100.0, 100.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "浓淡", &mut self.fx_density, 0.2..=3.0, 1.0, 0.05, 2);
        self.slider_changed(&s);
        if self.fx_model != FxModel::Black {
            let s = slider_row(ui, "去色边", &mut self.fx_despill, 0.0..=100.0, 20.0, 1.0, 0);
            self.slider_changed(&s);
        }
    }

    // ---------------- 双背景差分参数 ----------------

    fn dualbg_params(&mut self, ui: &mut egui::Ui) {
        section(ui, "SLOTS", "输入槽位", None);
        for (label, slot, loaded) in [
            ("黑底图", Slot::Black, self.black.is_some()),
            ("白底图", Slot::White, self.white.is_some()),
        ] {
            let size_text = match slot {
                Slot::Black => self.black.as_ref().map(|b| format!("{} × {}", b.w, b.h)),
                Slot::White => self.white.as_ref().map(|b| format!("{} × {}", b.w, b.h)),
            };
            ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(vec2(3.0, 14.0), Sense::hover());
                ui.painter().rect_filled(bar, 0.0, if loaded { C_INK } else { C_RULE });
                ui.label(egui::RichText::new(label).size(11.5).strong().color(if loaded { C_INK } else { C_INK2 }));
                ui.add_space(2.0);
                if let Some(s) = size_text {
                    ui.label(egui::RichText::new(s).size(10.5).color(C_INK2));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if loaded {
                        if cut_block(ui, "清除", false, vec2(48.0, 23.0), CUT_SMALL).clicked() {
                            match slot {
                                Slot::Black => {
                                    self.black = None;
                                    self.proxy_black = None;
                                    self.tex_black = None;
                                }
                                Slot::White => {
                                    self.white = None;
                                    self.proxy_white = None;
                                    self.tex_white = None;
                                }
                            }
                            self.dirty = true;
                        }
                    } else {
                        ui.label(egui::RichText::new("点画布槽位载入").size(10.0).color(C_INK2));
                    }
                });
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);
        }
        ui.label(
            egui::RichText::new("两张要同尺寸；一次拖两张会自动填好，也可以点画布上的槽位。")
                .size(10.5)
                .color(C_INK2),
        );

        section(ui, "FINE TUNE", "差分参数", None);
        let s = slider_row(ui, "容差", &mut self.db_tol, 0.0..=100.0, 20.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "边缘对比", &mut self.db_edge, 0.0..=100.0, 30.0, 1.0, 0);
        self.slider_changed(&s);
        let s = slider_row(ui, "边缘内缩", &mut self.db_erode, 0.0..=100.0, 0.0, 1.0, 0);
        self.slider_changed(&s);
    }

    fn pick_slot_image(&mut self, slot: Slot) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("图片文件", &["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "webp"]);
        if let Some(dir) = &self.last_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(p) = dialog.pick_file() else { return };
        match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| Img::from_bytes(&b)) {
            Ok(img) => match self.set_slot(slot, img) {
                Ok(()) => {
                    self.toast(
                        format!("已载入{}", if slot == Slot::Black { "黑底图" } else { "白底图" }),
                        false,
                    );
                    self.last_dir = p.parent().map(|x| x.to_path_buf());
                }
                Err(e) => self.toast(e, true),
            },
            Err(e) => self.toast(format!("载入失败：{e}"), true),
        }
    }

    // ---------------- 预览舞台 ----------------

    fn stage(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, C_PANEL);
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
        let inner = rect.shrink(1.0);
        let two_row = inner.width() < 760.0 && self.result.is_some();
        let head_h = if two_row { STAGE_HEAD + 32.0 } else { STAGE_HEAD };
        let head = Rect::from_min_size(inner.min, vec2(inner.width(), head_h));
        let foot = Rect::from_min_size(pos2(inner.left(), inner.bottom() - STAGE_FOOT), vec2(inner.width(), STAGE_FOOT));
        let canvas = Rect::from_min_max(pos2(inner.left(), head.bottom()), pos2(inner.right(), foot.top()));
        p.line_segment([pos2(inner.left(), head.bottom()), pos2(inner.right(), head.bottom())], Stroke::new(1.0, C_RULE));
        p.line_segment([pos2(inner.left(), foot.top()), pos2(inner.right(), foot.top())], Stroke::new(1.0, C_RULE));
        self.stage_head(ui, head, two_row);
        let st = self.stage_canvas(ui, canvas);
        self.stage_foot(ui, foot, &st);
    }

    fn stage_head(&mut self, ui: &mut egui::Ui, rect: Rect, two_row: bool) {
        let mut flip = false;
        let has = self.result.is_some();
        let row1 = Rect::from_min_size(rect.min, vec2(rect.width(), STAGE_HEAD));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(row1.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                draw_tag_chip_widget(ui, if self.mode == Mode::DualBg { "01 / 差分结果" } else { "01 / 结果" });
                ui.add_space(10.0);
                let mut info = match &self.result {
                    Some(r) => format!("{} × {}", r.w, r.h),
                    None => "— × —".to_string(),
                };
                if self.mode != Mode::DualBg {
                    if let Some(o) = self.orig.as_ref() {
                        info = format!("{info}   │   {:.2} MP", o.w as f32 * o.h as f32 / 1_000_000.0);
                    }
                }
                ui.label(egui::RichText::new(info).size(12.0).color(C_INK2));
                if has && !two_row {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.ctl_compare(ui).clicked() {
                            flip = true;
                        }
                        ui.add_space(12.0);
                        rule_v(ui);
                        ui.add_space(12.0);
                        self.ctl_views(ui);
                        ui.add_space(12.0);
                        rule_v(ui);
                        ui.add_space(12.0);
                        self.ctl_backdrop(ui);
                    });
                }
            },
        );
        if two_row {
            let row2 = Rect::from_min_max(pos2(rect.left(), row1.bottom()), rect.max);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(row2.shrink2(vec2(14.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    if self.ctl_compare(ui).clicked() {
                        flip = true;
                    }
                    ui.add_space(12.0);
                    rule_v(ui);
                    ui.add_space(12.0);
                    self.ctl_views(ui);
                    ui.add_space(12.0);
                    rule_v(ui);
                    ui.add_space(12.0);
                    self.ctl_backdrop(ui);
                },
            );
        }
        if flip {
            self.compare = !self.compare;
        }
    }

    fn ctl_compare(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let can = self.orig.is_some() && self.mode != Mode::DualBg && self.result.is_some();
        check_chip(ui, "对比", self.compare && can)
    }

    fn ctl_views(&mut self, ui: &mut egui::Ui) {
        if seg_block(ui, "蒙版", self.view_mask, vec2(44.0, 24.0)).clicked() {
            self.view_mask = true;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "彩色", !self.view_mask, vec2(44.0, 24.0)).clicked() {
            self.view_mask = false;
            self.update_result_tex(ui.ctx());
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("视图").size(10.0).color(C_INK2));
    }

    fn ctl_backdrop(&mut self, ui: &mut egui::Ui) {
        let mut custom = self.preview_custom;
        let changed = ui
            .scope(|ui| {
                ui.spacing_mut().interact_size = vec2(24.0, 24.0);
                ui.color_edit_button_srgba(&mut custom)
            })
            .inner;
        if ui.ctx().cumulative_pass_nr() > 4 && changed.changed() {
            self.preview_custom = custom;
            self.preview_bg = PreviewBg::Custom;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "黑", self.preview_bg == PreviewBg::Black, vec2(30.0, 24.0)).clicked() {
            self.preview_bg = PreviewBg::Black;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "白", self.preview_bg == PreviewBg::White, vec2(30.0, 24.0)).clicked() {
            self.preview_bg = PreviewBg::White;
            self.update_result_tex(ui.ctx());
        }
        if seg_block(ui, "棋盘", self.preview_bg == PreviewBg::Checker, vec2(44.0, 24.0)).clicked() {
            self.preview_bg = PreviewBg::Checker;
            self.update_result_tex(ui.ctx());
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("底色").size(10.0).color(C_INK2));
    }

    /// 绿/蓝幕：图片外圈颜色与当前键色的平均色差（越大越可能取错色）
    fn key_color_gap(&self) -> Option<f32> {
        let img = self.proxy_main.as_ref().or(self.orig.as_ref())?;
        let key = match self.mode {
            Mode::Green => self.key_green,
            Mode::Blue => self.key_blue,
            _ => return None,
        };
        let (w, h) = (img.w, img.h);
        if w < 16 || h < 16 {
            return None;
        }
        let (kr, kg, kb) = (key.r() as f32, key.g() as f32, key.b() as f32);
        let spots = [
            (0, 0),
            (w / 2, 0),
            (w - 8, 0),
            (0, h / 2),
            (w - 8, h / 2),
            (0, h - 8),
            (w / 2, h - 8),
            (w - 8, h - 8),
        ];
        let mut sum = 0.0f32;
        let mut n = 0.0f32;
        for (sx, sy) in spots {
            for y in sy..(sy + 8).min(h) {
                for x in sx..(sx + 8).min(w) {
                    let o = (y * w + x) * 4;
                    let dr = img.buf[o] as f32 - kr;
                    let dg = img.buf[o + 1] as f32 - kg;
                    let db = img.buf[o + 2] as f32 - kb;
                    sum += (dr * dr + dg * dg + db * db).sqrt();
                    n += 1.0;
                }
            }
        }
        Some(sum / n.max(1.0))
    }

    fn stage_foot(&mut self, ui: &mut egui::Ui, rect: Rect, st: &StageStat) {
        let mismatch = self.key_color_gap().map(|g| g > 55.0).unwrap_or(false);
        let hint = if !st.has_img {
            String::new()
        } else if mismatch && st.hover.is_none() && !self.peek {
            if self.mode == Mode::Green {
                "背景没去干净？点一下画布里的绿幕重新取色".to_string()
            } else {
                "背景没去干净？点一下画布里的蓝幕重新取色".to_string()
            }
        } else if let Some(h) = &st.hover {
            h.clone()
        } else if self.peek {
            "松开空格返回结果".to_string()
        } else if self.compare && self.orig.is_some() {
            "拖动分割线对比原图 · 空格速览".to_string()
        } else if matches!(
            self.mode,
            Mode::Green | Mode::Blue | Mode::Custom | Mode::Dual | Mode::Fx | Mode::Auto
        ) {
            "点击画布取色 · 滚轮缩放 · 双击 1:1 · 空格速览原图".to_string()
        } else {
            "滚轮缩放 · 拖动平移 · 双击 1:1 · 空格速览原图".to_string()
        };
        let tx = if mismatch && st.has_img {
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(rect.left() + 14.0, rect.center().y - 6.0), vec2(3.0, 12.0)),
                0.0,
                C_SIGNAL,
            );
            rect.left() + 25.0
        } else {
            rect.left() + 14.0
        };
        ui.painter().text(
            pos2(tx, rect.center().y),
            Align2::LEFT_CENTER,
            hint,
            FontId::proportional(10.5),
            if mismatch { C_INK } else { C_INK2 },
        );
        let fit = st.fit;
        let canvas = st.canvas;
        let img_w = st.img_w;
        let zoom = st.zoom;
        if !st.has_img {
            return;
        }
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 2.0))).layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                if cut_block(ui, "1:1", false, vec2(40.0, 22.0), 6.0).clicked() && fit.x > 0.0 && img_w > 0 {
                    self.view_scale = (img_w as f32 / fit.x).clamp(0.3, 16.0);
                    self.view_offset = Vec2::ZERO;
                }
                if cut_block(ui, "适应", false, vec2(46.0, 22.0), 6.0).clicked() {
                    self.view_scale = 1.0;
                    self.view_offset = Vec2::ZERO;
                }
                if cut_block(ui, "+", false, vec2(28.0, 22.0), 6.0).clicked() {
                    self.view_scale = (self.view_scale * 1.25).clamp(0.3, 16.0);
                    self.clamp_offset(fit * self.view_scale, canvas);
                }
                let zl = format!("{:.0}%", zoom);
                if cut_block(ui, &zl, false, vec2(52.0, 22.0), 6.0).clicked() {
                    self.view_scale = 1.0;
                    self.view_offset = Vec2::ZERO;
                }
                if cut_block(ui, "−", false, vec2(28.0, 22.0), 6.0).clicked() {
                    self.view_scale = (self.view_scale / 1.25).clamp(0.3, 16.0);
                    self.clamp_offset(fit * self.view_scale, canvas);
                }
            },
        );
    }

    fn stage_canvas(&mut self, ui: &mut egui::Ui, rect: Rect) -> StageStat {
        let p = ui.painter().clone().with_clip_rect(rect);
        p.rect_filled(rect, 0.0, C_PAPER);

        if self.orig.is_some() && self.tex_main.is_none() {
            let src = self.proxy_main.as_ref().or(self.orig.as_ref()).unwrap().clone();
            update_tex(ui.ctx(), &mut self.tex_main, "main", &src);
        }
        if self.mode == Mode::DualBg {
            if let Some(b) = self.proxy_black.as_ref().or(self.black.as_ref()) {
                if self.tex_black.is_none() {
                    update_tex(ui.ctx(), &mut self.tex_black, "black", b);
                }
            }
            if let Some(w) = self.proxy_white.as_ref().or(self.white.as_ref()) {
                if self.tex_white.is_none() {
                    update_tex(ui.ctx(), &mut self.tex_white, "white", w);
                }
            }
        }

        if self.mode == Mode::DualBg && self.result.is_none() {
            self.stage_dual_slots(ui, rect);
            return StageStat { hover: None, zoom: 100.0, fit: Vec2::ZERO, img_w: 0, canvas: rect, has_img: false };
        }
        let dims = if self.mode == Mode::DualBg {
            self.result.as_ref().map(|m| (m.w, m.h))
        } else {
            self.proxy_main.as_ref().or(self.orig.as_ref()).map(|m| (m.w, m.h))
        };
        let Some((mw, mh)) = dims else {
            self.stage_empty(ui, rect);
            return StageStat { hover: None, zoom: 100.0, fit: Vec2::ZERO, img_w: 0, canvas: rect, has_img: false };
        };

        let fit = fit_size(rect.shrink(18.0).size(), mw, mh);
        let resp = ui.interact(rect, egui::Id::new("stage_canvas"), Sense::click_and_drag());
        let show_cmp = self.compare && self.orig.is_some() && self.mode != Mode::DualBg && self.result.is_some();
        let div_x = rect.left() + rect.width() * self.split_x;
        let mut block_pan = false;
        let mut div_hover = false;
        if show_cmp {
            let hit = Rect::from_min_max(pos2(div_x - 11.0, rect.top()), pos2(div_x + 11.0, rect.bottom()));
            let d = ui.interact(hit, egui::Id::new("stage_divider"), Sense::click_and_drag());
            div_hover = d.hovered() || d.dragged();
            block_pan = div_hover;
            if d.dragged() {
                if let Some(pp) = d.interact_pointer_pos() {
                    self.split_x = ((pp.x - rect.left()) / rect.width().max(1.0)).clamp(0.04, 0.96);
                }
            }
        }

        let picking = matches!(
            self.mode,
            Mode::Green | Mode::Blue | Mode::Custom | Mode::Dual | Mode::Fx | Mode::Auto
        );
        self.viewport_interaction(&resp, fit, rect, mw, !block_pan);
        if div_hover {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        } else if picking && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }

        let disp = fit * self.view_scale;
        let ir = Rect::from_center_size(rect.center() + self.view_offset, disp);
        let mut hover: Option<String> = None;
        let mut picked: Option<Color32> = None;
        let peeking = self.peek && self.tex_main.is_some();

        if peeking {
            paint_backdrop(&p, ir, self.preview_bg, self.preview_custom);
            if let Some(t) = &self.tex_main {
                p.image(t.id(), ir, uv01(), Color32::WHITE);
                p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
            }
            p.rect_filled(Rect::from_min_size(pos2(rect.left() + 14.0, rect.top() + 13.0), vec2(4.0, 16.0)), 0.0, C_SIGNAL);
            p.text(
                pos2(rect.left() + 26.0, rect.top() + 12.0),
                Align2::LEFT_TOP,
                "原图速览",
                FontId::proportional(11.5),
                C_INK,
            );
        } else if let (Some(_r), Some(t)) = (self.result.as_ref(), self.tex_result.as_ref()) {
            if self.view_mask {
                p.rect_filled(ir, 0.0, C_INK);
            } else {
                paint_backdrop(&p, ir, self.preview_bg, self.preview_custom);
            }
            p.image(t.id(), ir, uv01(), Color32::WHITE);
            p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
            if show_cmp {
                if let Some(tm) = self.tex_main.as_ref() {
                    let clip = Rect::from_min_max(rect.min, pos2(div_x, rect.max.y)).intersect(rect);
                    let pc = p.with_clip_rect(clip);
                    pc.image(tm.id(), ir, uv01(), Color32::WHITE);
                    pc.line_segment([pos2(div_x, rect.top()), pos2(div_x, rect.bottom())], Stroke::new(1.0, C_INK));
                }
                draw_tag_chip(&p, pos2(rect.left() + 14.0, rect.top() + 13.0), "原图");
                draw_tag_chip_right(&p, pos2(rect.right() - 14.0, rect.top() + 13.0), "结果");
                draw_divider_handle(&p, pos2(div_x, rect.center().y));
            }
            if self.view_mask {
                p.text(
                    pos2(rect.right() - 14.0, rect.bottom() - 14.0),
                    Align2::RIGHT_BOTTOM,
                    "蒙版中灰 = 半透明；导出始终为彩色透明 PNG",
                    FontId::proportional(10.5),
                    C_INK2,
                );
            }
        } else if let (Some(_o), Some(tm)) = (self.orig.as_ref(), self.tex_main.as_ref()) {
            paint_backdrop(&p, ir, self.preview_bg, self.preview_custom);
            p.image(tm.id(), ir, uv01(), Color32::WHITE);
            p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
            p.text(
                pos2(rect.center().x, rect.bottom() - 16.0),
                Align2::CENTER_BOTTOM,
                "原图已就绪，等待处理结果",
                FontId::proportional(11.0),
                C_INK2,
            );
        }

        // 取色放大镜
        if picking && !div_hover {
            if let Some(pp) = resp.hover_pos() {
                if rect.contains(pp) && ir.contains(pp) {
                    if let Some(m) = self.proxy_main.as_ref().or(self.orig.as_ref()) {
                        if let Some([x, y]) = pixel_at(ir, pp, m) {
                            draw_loupe(&p, rect, pp, m, [x, y]);
                        }
                    }
                }
            }
        }

        if let Some(pp) = resp.hover_pos() {
            if rect.contains(pp) && ir.contains(pp) {
                let under = if show_cmp && pp.x < div_x {
                    self.proxy_main.as_ref().or(self.orig.as_ref())
                } else {
                    self.result.as_ref()
                };
                if let Some(m) = under {
                    if let Some([x, y]) = pixel_at(ir, pp, m) {
                        let i = (y * m.w + x) * 4;
                        if i + 3 < m.buf.len() {
                            let (r, g, b, a) = (m.buf[i], m.buf[i + 1], m.buf[i + 2], m.buf[i + 3]);
                            hover = Some(format!("{x} , {y}   │   {r}, {g}, {b}, {a}   │   #{r:02X}{g:02X}{b:02X}"));
                        }
                    }
                }
            }
        }

        if picking && resp.clicked() && !block_pan {
            if let Some(pp) = resp.interact_pointer_pos() {
                if ir.contains(pp) {
                    if let Some(m) = self.proxy_main.as_ref().or(self.orig.as_ref()) {
                        if let Some([x, y]) = pixel_at(ir, pp, m) {
                            let i = (y * m.w + x) * 4;
                            if i + 2 < m.buf.len() {
                                picked = Some(Color32::from_rgb(m.buf[i], m.buf[i + 1], m.buf[i + 2]));
                            }
                        }
                    }
                }
            }
        }
        if let Some(c) = picked {
            self.push_undo();
            match self.mode {
                Mode::Fx => self.fx_bg = c,
                Mode::Auto => {
                    self.auto_bg = Some(c);
                    self.auto_tried = true;
                }
                Mode::Green => self.key_green = c,
                Mode::Blue => self.key_blue = c,
                _ => self.c1 = c,
            }
            self.dirty = true;
            self.toast(format!("已取色 #{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()), false);
        }

        let zoom = if fit.x > 0.0 { fit.x / mw as f32 * self.view_scale * 100.0 } else { 100.0 };
        StageStat { hover, zoom, fit, img_w: mw, canvas: rect, has_img: true }
    }

    /// 双背景：画布即两个落区（点击 / 拖入）
    fn stage_dual_slots(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter().clone().with_clip_rect(rect);
        p.rect_filled(rect, 0.0, C_PAPER);
        let pad = 18.0;
        p.text(
            pos2(rect.left() + pad, rect.top() + 14.0),
            Align2::LEFT_TOP,
            "// 需要两张同尺寸照片：黑底一张、白底一张",
            FontId::proportional(11.0),
            C_INK2,
        );
        let top = rect.top() + 42.0;
        let gap = 12.0;
        let zh = ((rect.bottom() - 16.0 - top - gap) / 2.0).max(56.0);
        let zw = rect.width() - pad * 2.0;
        let slots = [
            ("黑底图", Slot::Black, self.black.as_ref().map(|m| (m.w, m.h)), self.tex_black.as_ref().map(|t| t.id())),
            ("白底图", Slot::White, self.white.as_ref().map(|m| (m.w, m.h)), self.tex_white.as_ref().map(|t| t.id())),
        ];
        let mut pick: Option<Slot> = None;
        for (i, (label, slot, dims, tex)) in slots.into_iter().enumerate() {
            let z = Rect::from_min_size(pos2(rect.left() + pad, top + i as f32 * (zh + gap)), vec2(zw, zh));
            let resp = ui.interact(z, egui::Id::new(("dual_zone", i)), Sense::click());
            match (dims, tex) {
                (Some((iw, ih)), Some(t)) => {
                    p.rect_filled(z, 0.0, C_PANEL);
                    p.rect_stroke(z, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
                    let fit = fit_size(z.shrink(52.0).size(), iw, ih);
                    let ir = Rect::from_center_size(z.center(), fit);
                    p.image(t, ir, uv01(), Color32::WHITE);
                    p.rect_stroke(ir, 0.0, Stroke::new(1.0, C_RULE), StrokeKind::Inside);
                    p.text(
                        pos2(z.left() + 12.0, z.top() + 10.0),
                        Align2::LEFT_TOP,
                        label,
                        FontId::proportional(11.0),
                        C_INK2,
                    );
                    p.text(
                        pos2(z.right() - 12.0, z.top() + 10.0),
                        Align2::RIGHT_TOP,
                        format!("{iw} × {ih} · 点击替换"),
                        FontId::proportional(10.5),
                        C_INK2,
                    );
                }
                _ => {
                    hatch(&p, z);
                    p.rect_stroke(z, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
                    p.rect_filled(
                        Rect::from_min_size(pos2(z.left(), z.center().y - 26.0), vec2(3.0, 52.0)),
                        0.0,
                        C_RULE,
                    );
                    p.text(
                        pos2(z.left() + 18.0, z.center().y - 12.0),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(15.0),
                        C_INK,
                    );
                    p.text(
                        pos2(z.left() + 18.0, z.center().y + 12.0),
                        Align2::LEFT_CENTER,
                        "点击载入，或把图片拖到窗口",
                        FontId::proportional(10.5),
                        C_INK2,
                    );
                }
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                p.rect_stroke(z, 0.0, Stroke::new(2.0, C_INK), StrokeKind::Inside);
            }
            if resp.clicked() {
                pick = Some(slot);
            }
        }
        if let Some(slot) = pick {
            self.pick_slot_image(slot);
        }
    }

    fn stage_empty(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = ui.painter().clone().with_clip_rect(rect);
        p.rect_filled(rect, 0.0, C_PAPER);
        hatch(&p, rect);
        let bw = (rect.width() - 80.0).clamp(240.0, 430.0);
        let band = Rect::from_center_size(rect.center(), vec2(bw, 210.0));
        let plate = band.shrink2(vec2(-14.0, -6.0));
        p.rect_filled(plate, 0.0, C_PAPER);
        p.rect_stroke(plate, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
        let dual = self.mode == Mode::DualBg;
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(band).layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                ui.add_space(18.0);
                ui.label(egui::RichText::new("// 等待输入").size(10.0).color(C_INK2));
                ui.add_space(5.0);
                ui.label(
                    egui::RichText::new(if dual { "载入黑底 + 白底两张图" } else { "把图片拖进窗口" })
                        .size(21.0)
                        .strong()
                        .color(C_INK),
                );
                ui.add_space(7.0);
                ui.label(
                    egui::RichText::new(if dual {
                        "同一主体、同一尺寸，一次拖入两张会自动填槽"
                    } else {
                        "Ctrl+V 粘贴 · 按 O 键打开 · 支持 PNG / JPG / WEBP / BMP"
                    })
                    .size(11.0)
                    .color(C_INK2),
                );
                ui.add_space(18.0);
                if dual {
                    ui.horizontal(|ui| {
                        if primary_block(ui, "载入黑底", vec2(102.0, 32.0)).clicked() {
                            self.pick_slot_image(Slot::Black);
                        }
                        ui.add_space(6.0);
                        if primary_block(ui, "载入白底", vec2(102.0, 32.0)).clicked() {
                            self.pick_slot_image(Slot::White);
                        }
                    });
                } else if primary_block(ui, "打开图片", vec2(126.0, 34.0)).clicked() {
                    self.open_file_dialog();
                }
            },
        );
    }

    /// 视口公共交互：滚轮缩放（鼠标中心）/ 拖动平移 / 双击 1:1 ↔ 适应窗口
    fn viewport_interaction(&mut self, resp: &egui::Response, fit: Vec2, vp: Rect, img_w: usize, allow_pan: bool) {
        if resp.hovered() {
            let scroll = resp.ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old_s = self.view_scale;
                let new_s = (old_s * (1.0 + scroll * 0.0018)).clamp(0.3, 16.0);
                if let Some(ptr) = resp.hover_pos() {
                    let c0 = vp.center();
                    let u = (ptr - c0 - self.view_offset) / (fit * old_s);
                    self.view_offset = ptr - c0 - u * (fit * new_s);
                }
                self.view_scale = new_s;
                let disp = fit * new_s;
                self.clamp_offset(disp, vp);
            }
            if allow_pan && self.view_scale > 1.0 && !resp.dragged() {
                resp.ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
        if allow_pan && self.view_scale > 1.0 && resp.dragged() {
            self.view_offset += resp.drag_delta();
            self.clamp_offset(fit * self.view_scale, vp);
            resp.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        // 中键：任何缩放级别都能拖
        if allow_pan && resp.dragged_by(egui::PointerButton::Middle) {
            self.view_offset += resp.drag_delta();
            self.clamp_offset(fit * self.view_scale, vp);
            resp.ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.double_clicked() {
            if (self.view_scale - 1.0).abs() < 0.01 {
                self.view_scale = (img_w as f32 / fit.x).clamp(1.0, 16.0);
                self.view_offset = Vec2::ZERO;
            } else {
                self.view_scale = 1.0;
                self.view_offset = Vec2::ZERO;
            }
        }
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.upd_rx.try_recv() {
            match m {
                update::Msg::Found(i) => {
                    self.upd_state = UpdState::Idle;
                    if self.skip_version.as_deref() != Some(i.version.as_str()) {
                        self.toast(format!("发现新版本 v{}", i.version), false);
                        self.upd_info = Some(i);
                    }
                }
                update::Msg::NoUpdate(v) => {
                    self.upd_state = UpdState::Latest(v);
                    if self.upd_manual {
                        self.toast("已是最新版本", false);
                    }
                    self.upd_manual = false;
                }
                update::Msg::Failed(e) => {
                    self.upd_state = UpdState::Failed(e.clone());
                    if self.upd_manual {
                        self.toast(format!("检查更新失败：{e}"), true);
                    }
                    self.upd_manual = false;
                }
                update::Msg::Ready(p) => {
                    self.upd_state = UpdState::Idle;
                    match update::apply(&p) {
                        Ok(()) => {
                            self.toast("正在替换并重启…", false);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(e) => {
                            self.upd_busy = false;
                            self.toast(format!("替换失败：{e}"), true);
                        }
                    }
                }
            }
        }
    }

    fn check_updates(&mut self, manual: bool) {
        if self.upd_state == UpdState::Checking {
            return;
        }
        self.upd_state = UpdState::Checking;
        self.upd_manual = manual;
        update::check(self.upd_tx.clone());
    }

    fn update_window(&mut self, ctx: &egui::Context) {
        if !self.upd_open {
            return;
        }
        let Some(info) = self.upd_info.clone() else {
            self.upd_open = false;
            return;
        };
        let busy = self.upd_busy;
        let mut open = true;
        let (mut go, mut later, mut skip) = (false, false, false);
        egui::Window::new("发现有新版本 // UPDATE")
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
                ui.label(
                    egui::RichText::new(format!("当前 v{}", update::current_version()))
                        .size(11.0)
                        .color(C_INK2),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(format!("最新 v{}", info.version))
                        .size(19.0)
                        .strong()
                        .color(C_INK),
                );
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(170.0).show(ui, |ui| {
                    let notes = if info.notes.trim().is_empty() {
                        "这个版本没有写更新说明。".to_string()
                    } else {
                        info.notes.clone()
                    };
                    ui.label(egui::RichText::new(notes).size(11.0).color(C_INK2));
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if busy {
                        ui.label(
                            egui::RichText::new("正在下载新版本…")
                                .size(12.0)
                                .strong()
                                .color(C_INK),
                        );
                    } else if primary_block(ui, "下载并安装", vec2(120.0, 32.0)).clicked() {
                        go = true;
                    }
                    ui.add_space(6.0);
                    if cut_block(ui, "以后再说", false, vec2(78.0, 32.0), CUT_SMALL).clicked() {
                        later = true;
                    }
                    if cut_block(ui, "忽略此版本", false, vec2(90.0, 32.0), CUT_SMALL).clicked() {
                        skip = true;
                    }
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("不会强制更新。下载完成后程序自动重启，旧版本留备份，失败会回滚。")
                        .size(10.0)
                        .color(C_INK2),
                );
            });
        if go {
            self.upd_busy = true;
            update::download(info.clone(), self.upd_tx.clone());
        }
        if skip {
            self.skip_version = Some(info.version.clone());
            self.upd_info = None;
            self.upd_open = false;
            self.toast(format!("已忽略 v{}", info.version), false);
        }
        if later || !open {
            self.upd_open = false;
        }
    }

    fn clamp_offset(&mut self, disp: Vec2, vp: Rect) {
        let max_x = ((disp.x - vp.width()) / 2.0 + 24.0).max(0.0);
        let max_y = ((disp.y - vp.height()) / 2.0 + 24.0).max(0.0);
        self.view_offset.x = self.view_offset.x.clamp(-max_x, max_x);
        self.view_offset.y = self.view_offset.y.clamp(-max_y, max_y);
    }

    // ---------------- 帮助弹窗 ----------------

    fn help_window(&mut self, ctx: &egui::Context) {
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
                const ROWS: [(&str, &str); 18] = [
                    ("1 – 9", "切换处理模式"),
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
                    egui::RichText::new("拖入图片直接替换主图；双背景模式下先拖两张会自动填入黑底、白底槽位。")
                        .size(10.5)
                        .color(C_INK2),
                );
            });
        self.help_open = open;
    }
}

// ---------------------------------------------------------------------------
// UI 部件（终末地部件）
// ---------------------------------------------------------------------------

/// uv 全图
fn uv01() -> Rect {
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
}

/// 切角多边形（右下切角）
fn cut_polygon(rect: Rect, cut: f32) -> Vec<Pos2> {
    vec![
        pos2(rect.left(), rect.top()),
        pos2(rect.right(), rect.top()),
        pos2(rect.right(), rect.bottom() - cut),
        pos2(rect.right() - cut, rect.bottom()),
        pos2(rect.left(), rect.bottom()),
    ]
}

fn fill_path(p: &egui::Painter, pts: Vec<Pos2>, fill: Color32, stroke: Stroke) {
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

const TITLE_H: f32 = 34.0;
const TRI_H: f32 = 3.0;
const TAB_H: f32 = 38.0;
const STATUS_H: f32 = 30.0;
const STAGE_HEAD: f32 = 38.0;
const STAGE_FOOT: f32 = 30.0;
const PROPS_DEF: f32 = 300.0;
const PROPS_MIN: f32 = 264.0;
const PROPS_MAX: f32 = 460.0;
const BODY_PAD: f32 = 12.0;
const SPLITTER: f32 = 10.0;

/// 舞台一帧的测量结果，供脚注渲染与缩放交互复用
struct StageStat {
    hover: Option<String>,
    zoom: f32,
    fit: Vec2,
    img_w: usize,
    canvas: Rect,
    has_img: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Open,
    Paste,
    Copy,
    Export,
}

impl Act {
    fn label(self) -> &'static str {
        match self {
            Act::Open => "打开",
            Act::Paste => "粘贴",
            Act::Copy => "复制",
            Act::Export => "导出 PNG",
        }
    }
    fn id(self) -> u8 {
        match self {
            Act::Open => 1,
            Act::Paste => 2,
            Act::Copy => 3,
            Act::Export => 4,
        }
    }
}

struct CmdLayout {
    tabs: Vec<(Mode, Rect)>,
    acts: Vec<(Act, Rect)>,
    h: f32,
}

fn text_w(ctx: &egui::Context, s: &str, size: f32) -> f32 {
    ctx.fonts_mut(|f| {
        f.layout_no_wrap(s.to_owned(), FontId::proportional(size), C_PANEL)
            .size()
            .x
    })
}

fn mode_idx(m: Mode) -> usize {
    Mode::GRID.iter().position(|(x, _)| *x == m).unwrap_or(0)
}

/// 模式的中英名与一句说明
fn mode_meta(m: Mode) -> (&'static str, &'static str, &'static str) {
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
    }
}

/// 命令栏排版：模式标签自动换行，动作固定在右上
fn cmd_layout(ctx: &egui::Context, origin: Pos2, w: f32) -> CmdLayout {
    const AC: [Act; 4] = [Act::Open, Act::Paste, Act::Copy, Act::Export];
    let bw: Vec<f32> = AC.iter().map(|a| text_w(ctx, a.label(), 13.0) + 26.0).collect();
    let acts_w: f32 = bw.iter().sum::<f32>() + 8.0 * 3.0;
    let pad_l = 14.0;
    let pad_r = 12.0;
    let avail = (w - pad_l - pad_r - acts_w - 18.0).max(150.0);

    let widths: Vec<f32> = Mode::GRID.iter().map(|(_, n)| text_w(ctx, n, 13.0) + 44.0).collect();
    let sum: f32 = widths.iter().sum();
    // 分组间距：自动 | 色键组 | 双背景 | 半透明
    let gx = 12.0 * 3.0;
    let single = sum + gx + 8.0 * 6.0 <= avail;
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
        let breaks = (start..end.saturating_sub(1)).filter(|i| matches!(i, 0 | 6 | 7)).count() as f32;
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
            if matches!(i, 0 | 6 | 7) {
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
fn paint_mode_tab(p: &egui::Painter, rect: Rect, num: usize, label: &str, selected: bool, hovered: bool) {
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
fn paint_dark_btn(p: &egui::Painter, rect: Rect, label: &str, t: f32, primary: bool, enabled: bool) {
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
fn window_btn(ui: &mut egui::Ui, glyph: &str, hover_fill: Color32, hover_text: Color32) -> egui::Response {
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
fn rule_v(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 12.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, C_RULE);
}

/// 深色细竖线
fn rule_v_dark(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 10.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, C_RULE_DARK);
}

/// 状态栏字段（在 right_to_left 里调用：先值后标签，视觉顺序为「标签 值」）
fn field_dark(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(value).size(11.5).strong().color(C_BAR_TXT));
    ui.add_space(5.0);
    ui.label(egui::RichText::new(label).size(10.0).color(C_INK3_DARK));
}

/// 状态栏文字链（悬停黄块墨字）
fn link_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
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
fn upd_chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
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
fn swatch(ui: &mut egui::Ui, c: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, c);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, C_INK), StrokeKind::Inside);
}

/// 参数分组标题：`// EN` + 中文粗体 + 右侧字段 + 发丝线（单行版，节省面板高度）
fn section(ui: &mut egui::Ui, en: &str, zh: &str, right: Option<String>) {
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
fn cut_block(ui: &mut egui::Ui, label: &str, selected: bool, size: Vec2, cut: f32) -> egui::Response {
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
fn seg_block(ui: &mut egui::Ui, label: &str, selected: bool, size: Vec2) -> egui::Response {
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
fn check_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
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
fn primary_block(ui: &mut egui::Ui, label: &str, size: Vec2) -> egui::Response {
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

struct SliderOut {
    #[allow(dead_code)]
    resp: egui::Response,
    drag_started: bool,
    changed: bool,
}

/// 参数行：标签 + 右对齐数值 + 通栏细轨（点击跳值、拖动、双击复位、滚轮微调）
fn slider_row(
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
fn draw_tag_chip_widget(ui: &mut egui::Ui, text: &str) {
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

fn draw_tag_chip(p: &egui::Painter, pos: Pos2, text: &str) {
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
fn draw_tag_chip_right(p: &egui::Painter, right_top: Pos2, text: &str) {
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
fn draw_divider_handle(p: &egui::Painter, c: Pos2) {
    let r = Rect::from_center_size(c, vec2(30.0, 36.0));
    p.rect_filled(r, 0.0, C_INK);
    let s = Stroke::new(1.5, C_PANEL);
    p.line_segment([pos2(c.x - 4.0, c.y - 5.0), pos2(c.x - 8.0, c.y)], s);
    p.line_segment([pos2(c.x - 8.0, c.y), pos2(c.x - 4.0, c.y + 5.0)], s);
    p.line_segment([pos2(c.x + 4.0, c.y - 5.0), pos2(c.x + 8.0, c.y)], s);
    p.line_segment([pos2(c.x + 8.0, c.y), pos2(c.x + 4.0, c.y + 5.0)], s);
}

/// 45° 斜线纹（空状态）
fn hatch(p: &egui::Painter, rect: Rect) {
    let pc = p.with_clip_rect(rect);
    let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x16, 0x16, 0x16, HATCH_ALPHA));
    let mut x = rect.left();
    while x < rect.right() + rect.height() {
        pc.line_segment([pos2(x, rect.bottom()), pos2(x + rect.height(), rect.top())], stroke);
        x += 7.0;
    }
}

/// 按可用宽度截断文字并补省略号
fn ellipsize(p: &egui::Painter, s: &str, font: FontId, max_w: f32) -> String {
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
fn pixel_at(img_rect: Rect, pos: Pos2, img: &Img) -> Option<[usize; 2]> {
    if !img_rect.contains(pos) || img.w == 0 || img.h == 0 {
        return None;
    }
    let fx = ((pos.x - img_rect.left()) / img_rect.width()).clamp(0.0, 0.9999);
    let fy = ((pos.y - img_rect.top()) / img_rect.height()).clamp(0.0, 0.9999);
    Some([(fx * img.w as f32) as usize, (fy * img.h as f32) as usize])
}

/// 取色放大镜：9×9 邻域放大 + 十字线 + HEX 芯片
fn draw_loupe(p: &egui::Painter, viewport: Rect, hp: Pos2, img: &Img, ipx: [usize; 2]) {
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
fn draw_toasts(ctx: &egui::Context, toasts: &[Toast]) {
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
fn draw_drop_hint(
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

fn rgb_of(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

fn color32_from(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}

fn fit_size(max: Vec2, w: usize, h: usize) -> Vec2 {
    if w == 0 || h == 0 {
        return Vec2::ZERO;
    }
    let s = (max.x / w as f32).min(max.y / h as f32).min(8.0);
    Vec2::new(w as f32 * s, h as f32 * s)
}

fn ensure_png_ext(path: PathBuf) -> PathBuf {
    const KNOWN: [&str; 17] = [
        "png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "tga", "pam",
        "webp", "ico", "ff", "qoi", "dds", "hdr", "exr", "avif",
    ];
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) {
        Some(ext) if KNOWN.contains(&ext.as_str()) => path,
        _ => {
            let mut os = path.into_os_string();
            os.push(".png");
            PathBuf::from(os)
        }
    }
}

/// 生成/更新预览纹理
fn update_tex(ctx: &egui::Context, slot: &mut Option<TextureHandle>, name: &str, img: &Img) {
    let ci = image_to_color_image(img, false);
    match slot {
        Some(h) => h.set(ci, TextureOptions::LINEAR),
        None => *slot = Some(ctx.load_texture(name, ci, TextureOptions::LINEAR)),
    }
}

/// 纹理数据：mask → 灰度蒙版；否则直出带透明度的 RGBA（底色改由画布在屏幕空间绘制）
fn image_to_color_image(img: &Img, mask: bool) -> ColorImage {
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
fn paint_backdrop(p: &egui::Painter, ir: Rect, bg: PreviewBg, custom: Color32) {
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

/// 本机时间（按 UTC+8 处理，本工具面向国内用户）
fn civil_now() -> (i32, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 + 8 * 3600)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    (
        y as i32,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// 天数 → 年月日（Howard Hinnant 算法）
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// 字体与主题
// ---------------------------------------------------------------------------

/// 应用图标（任务栏 / 窗口）
fn app_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/app_256.png")).ok()?.to_rgba8();
    Some(egui::IconData {
        rgba: img.as_raw().clone(),
        width: img.width(),
        height: img.height(),
    })
}

/// 安装字体：SairaWide（拉丁宽体）+ 微软雅黑（中文黑体）
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("saira_wide".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../assets/SairaWide.ttf"))));
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
fn apply_theme(ctx: &egui::Context) {
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
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "框构 // 智能抠图处理",
        options,
        Box::new(move |cc| {
            Ok(Box::new(App::new(cc, arg_img, arg_mode, arg_bg)) as Box<dyn eframe::App>)
        }),
    )
}
