//! 视频工作区的数据契约：媒体信息、键色模式、输出格式、可持久化设置、导出规格。
//!
//! 本文件只定义「数据」，不含任何 IO 或绘制逻辑。`VideoSettings` 是视频工作区
//! 唯一需要持久化的对象，由 `app::Persisted` 统一存取；图片工作区的参数与它完全隔离。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// ffprobe 探测到的源视频信息。
#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub duration: f64,
    pub frames: u64,
    pub codec: String,
    pub bytes: u64,
}

impl VideoInfo {
    pub fn label(&self) -> String {
        format!("{}×{} · {:.2} fps · {:.1}s · {}", self.w, self.h, self.fps, self.duration, self.codec)
    }
}

/// 去底色模式。除 Unmult 外，全部走色度键控；键色可一键取样覆盖。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyMode {
    Auto,
    Green,
    Blue,
    White,
    Black,
    Magenta,
    Custom,
    Unmult,
}

impl KeyMode {
    pub const ALL: [KeyMode; 8] = [
        KeyMode::Auto, KeyMode::Green, KeyMode::Blue, KeyMode::White,
        KeyMode::Black, KeyMode::Magenta, KeyMode::Custom, KeyMode::Unmult,
    ];
    pub fn label(self) -> &'static str {
        match self {
            KeyMode::Auto => "自动识别",
            KeyMode::Green => "绿幕",
            KeyMode::Blue => "蓝幕",
            KeyMode::White => "白底",
            KeyMode::Black => "黑底",
            KeyMode::Magenta => "洋红",
            KeyMode::Custom => "自选颜色",
            KeyMode::Unmult => "去黑底 Unmult",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            KeyMode::Auto => "auto",
            KeyMode::Green => "green",
            KeyMode::Blue => "blue",
            KeyMode::White => "white",
            KeyMode::Black => "black",
            KeyMode::Magenta => "magenta",
            KeyMode::Custom => "custom",
            KeyMode::Unmult => "unmult",
        }
    }
    pub fn from_key(s: &str) -> Self {
        Self::ALL.iter().copied().find(|m| m.key() == s).unwrap_or(KeyMode::Auto)
    }
    /// 该模式预设的键色（RGB）；Auto / Unmult 不依赖键色。
    pub fn default_rgb(self) -> [u8; 3] {
        match self {
            KeyMode::Green => [0, 255, 0],
            KeyMode::Blue => [0, 0, 255],
            KeyMode::White => [255, 255, 255],
            KeyMode::Black => [0, 0, 0],
            KeyMode::Magenta => [255, 0, 255],
            _ => [0, 255, 0],
        }
    }
    /// 是否需要在画布上取色。
    pub fn needs_pick(self) -> bool {
        matches!(self, KeyMode::Custom | KeyMode::Green | KeyMode::Blue | KeyMode::Magenta)
    }
}

/// 导出容器格式。透明视频优先 WebM / MOV；GIF 为 1-bit 透明；MP4 为合成底色。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutFormat {
    Gif,
    WebmAlpha,
    MovProres,
    MovQtrle,
    Apng,
    Mp4Flat,
    PngSeq,
}

impl OutFormat {
    pub const ALL: [OutFormat; 7] = [
        OutFormat::Gif, OutFormat::WebmAlpha, OutFormat::MovProres,
        OutFormat::MovQtrle, OutFormat::Apng, OutFormat::Mp4Flat, OutFormat::PngSeq,
    ];
    pub fn label(self) -> &'static str {
        match self {
            OutFormat::Gif => "GIF 动图",
            OutFormat::WebmAlpha => "WebM（透明）",
            OutFormat::MovProres => "MOV ProRes 4444（透明）",
            OutFormat::MovQtrle => "MOV 动画（透明）",
            OutFormat::Apng => "APNG（透明）",
            OutFormat::Mp4Flat => "MP4（合成底色）",
            OutFormat::PngSeq => "PNG 序列（透明）",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            OutFormat::Gif => "GIF",
            OutFormat::WebmAlpha => "WebM",
            OutFormat::MovProres => "ProRes",
            OutFormat::MovQtrle => "QTRLE",
            OutFormat::Apng => "APNG",
            OutFormat::Mp4Flat => "MP4",
            OutFormat::PngSeq => "PNG 序列",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            OutFormat::Gif => "gif",
            OutFormat::WebmAlpha => "webm",
            OutFormat::MovProres | OutFormat::MovQtrle => "mov",
            OutFormat::Apng => "png",
            OutFormat::Mp4Flat => "mp4",
            OutFormat::PngSeq => "png",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            OutFormat::Gif => "gif",
            OutFormat::WebmAlpha => "webm",
            OutFormat::MovProres => "prores",
            OutFormat::MovQtrle => "qtrle",
            OutFormat::Apng => "apng",
            OutFormat::Mp4Flat => "mp4",
            OutFormat::PngSeq => "seq",
        }
    }
    pub fn from_key(s: &str) -> Self {
        Self::ALL.iter().copied().find(|f| f.key() == s).unwrap_or(OutFormat::Gif)
    }
    pub fn is_gif(self) -> bool { matches!(self, OutFormat::Gif) }
    pub fn has_alpha(self) -> bool { !matches!(self, OutFormat::Mp4Flat) }
}

/// 画面适配方式：留边 / 裁切 / 拉伸。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FitMode {
    Contain,
    Cover,
    Stretch,
}
impl FitMode {
    pub fn label(self) -> &'static str {
        match self { FitMode::Contain => "适应（留边）", FitMode::Cover => "填充（裁切）", FitMode::Stretch => "拉伸" }
    }
    pub fn key(self) -> &'static str {
        match self { FitMode::Contain => "contain", FitMode::Cover => "cover", FitMode::Stretch => "stretch" }
    }
    pub fn from_key(s: &str) -> Self {
        match s { "cover" => FitMode::Cover, "stretch" => FitMode::Stretch, _ => FitMode::Contain }
    }
}

/// GIF 体积超标时的优先降级方向。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LimitStrategy {
    FramesFirst,
    ResolutionFirst,
    Balanced,
}
impl LimitStrategy {
    pub fn label(self) -> &'static str {
        match self {
            LimitStrategy::FramesFirst => "优先保清晰度（先抽帧）",
            LimitStrategy::ResolutionFirst => "优先保流畅（先降尺寸）",
            LimitStrategy::Balanced => "两者均衡",
        }
    }
    pub fn key(self) -> &'static str {
        match self { LimitStrategy::FramesFirst => "frames", LimitStrategy::ResolutionFirst => "size", LimitStrategy::Balanced => "balanced" }
    }
    pub fn from_key(s: &str) -> Self {
        match s { "frames" => LimitStrategy::FramesFirst, "size" => LimitStrategy::ResolutionFirst, _ => LimitStrategy::Balanced }
    }
}

/// 视频工作区全部可调参数；`Serialize` 让会话记忆只依赖一个字段。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoSettings {
    pub key_mode: String,
    pub key_color: u32,
    pub tol: f32,
    pub feather: f32,
    pub suppress: f32,
    pub erode: f32,
    pub smooth: f32,
    pub un_black: f32,
    pub un_white: f32,
    pub un_density: f32,
    pub un_floor: f32,
    pub un_gain: f32,
    pub un_luma: bool,
    pub out_format: String,
    pub preset: String,
    pub custom_w: u32,
    pub custom_h: u32,
    pub fps_preset: String,
    pub fps_custom: f32,
    pub fit: String,
    pub trim_start: f32,
    pub trim_end: f32,
    pub gif_limit_mb: f32,
    pub limit_strategy: String,
    pub strict_limit: bool,
    pub gif_colors: u16,
    pub gif_dither: bool,
    pub alpha_threshold: u8,
    pub loop_count: u16,
    pub crf: u32,
    pub bitrate_kbps: u32,
    pub flat_color: u32,
    pub ffmpeg_path: String,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            key_mode: "green".into(),
            key_color: 0x00FF00,
            tol: 80.0,
            feather: 5.0,
            suppress: 60.0,
            erode: 0.0,
            smooth: 0.0,
            un_black: 0.0,
            un_white: 255.0,
            un_density: 1.0,
            un_floor: 2.0,
            un_gain: 12.0,
            un_luma: false,
            out_format: "gif".into(),
            preset: "720p".into(),
            custom_w: 640,
            custom_h: 360,
            fps_preset: "15".into(),
            fps_custom: 15.0,
            fit: "contain".into(),
            trim_start: 0.0,
            trim_end: 0.0,
            gif_limit_mb: 0.0,
            limit_strategy: "balanced".into(),
            strict_limit: true,
            gif_colors: 128,
            gif_dither: true,
            alpha_threshold: 96,
            loop_count: 0,
            crf: 30,
            bitrate_kbps: 4000,
            flat_color: 0x000000,
            ffmpeg_path: String::new(),
        }
    }
}

/// 由设置 + 源信息解析出的实际输出规格。
#[derive(Clone, Copy, Debug)]
pub struct OutputSpec {
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub start: f64,
    pub end: f64,
    pub duration: f64,
    pub frames: u64,
    pub delay_ms: u32,
}

/// 体积上限介入后的最终导出计划（可能比界面设定更保守）。
#[derive(Clone, Debug)]
pub struct ExportPlan {
    pub spec: OutputSpec,
    pub colors: u16,
    pub speed: i32,
    pub budget: Option<u64>,
    /// 给用户看的自动调整说明，例如「为控制体积：15→9 fps，1280→960 宽」。
    pub note: String,
}
