//! 应用级枚举与可持久化数据结构。
//!
//! 只放「数据」，不放行为：Mode（图片模式）、PreviewBg、Slot、Workspace、
//! Toast、Snapshot（撤销快照）、Persisted（会话持久化）。

#![allow(unused_imports)]
use eframe::egui::Color32;
use std::time::Instant;
use crate::algo::{FxBgType, FxModel};


/// 顶层工作区：图片与视频各自持有独立素材、参数与导出设置，互不干扰。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Workspace {
    #[default]
    Image,
    Video,
}

impl Workspace {
    pub(crate) fn label(self) -> &'static str {
        match self { Workspace::Image => "图片", Workspace::Video => "视频" }
    }
    pub(crate) fn from_key(s: &str) -> Self {
        if s.eq_ignore_ascii_case("video") { Workspace::Video } else { Workspace::Image }
    }
    pub(crate) fn key(self) -> &'static str {
        match self { Workspace::Image => "image", Workspace::Video => "video" }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Auto,
    Green,
    Blue,
    White,
    Corner,
    Custom,
    Dual,
    DualBg,
    Fx,
    /// 新增：Unmult 一键去黑底（黑底光效 / 烟雾素材）。
    Unmult,
}

impl Mode {
    pub(crate) const GRID: [(Mode, &'static str); 10] = [
        (Mode::Auto, "自动去底"),
        (Mode::Green, "绿幕去底"),
        (Mode::Blue, "蓝幕去底"),
        (Mode::White, "去白底"),
        (Mode::Corner, "左上角去底"),
        (Mode::Custom, "自选颜色"),
        (Mode::Dual, "双色去底"),
        (Mode::DualBg, "双底合成"),
        (Mode::Fx, "半透明保留"),
        (Mode::Unmult, "去黑底 Unmult"),
    ];

    pub(crate) fn from_cli(s: &str) -> Option<Mode> {
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
            "unmult" | "black" => Mode::Unmult,
            _ => return None,
        })
    }

    /// 对应算法里的 mode 编号；Auto/DualBg/Fx 走独立函数
    pub(crate) fn id(self) -> u32 {
        match self {
            Mode::Green => 0,
            Mode::Blue => 1,
            Mode::White => 2,
            Mode::Custom => 3,
            Mode::Dual => 4,
            Mode::Corner => 5,
            Mode::Auto | Mode::DualBg | Mode::Fx | Mode::Unmult => u32::MAX,
        }
    }
}

/// 结果预览底色
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewBg {
    Checker,
    Black,
    White,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Black,
    White,
}

/// 更新检查状态
#[derive(Clone, PartialEq)]
pub enum UpdState {
    Idle,
    Checking,
    Latest(String),
    Failed(String),
}

/// Toast（白底细框 + 黄旗，2.5s 自动消失）
pub struct Toast {
    pub(crate) msg: String,
    pub(crate) warn: bool,
    pub(crate) born: Instant,
}

/// 参数快照（撤销 / 重做）
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
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
    /// Unmult 去黑底参数（与视频工作区同名参数互不影响）
    pub(crate) un_black: f32,
    pub(crate) un_white: f32,
    pub(crate) un_density: f32,
    pub(crate) un_floor: f32,
    pub(crate) un_gain: f32,
    pub(crate) un_luma: bool,
}
