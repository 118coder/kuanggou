//! 半透明抠底的参数类型与底色预设（模型 / 底色 / 参数量纲）。

#![allow(unused_imports)]
use crate::algo::Img;
use crate::algo::chroma::*;
use crate::algo::dual_bg::*;
use crate::algo::bg_field::*;
use crate::algo::fx_matte::*;
use crate::algo::unmult::*;


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxModel {
    Color,
    Smoke,
    Black,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxBgType {
    Blue,
    Green,
    Black,
    White,
    Magenta,
}

impl FxBgType {
    pub fn pure_color(self) -> [u8; 3] {
        match self {
            FxBgType::Blue => [0, 0, 255],
            FxBgType::Green => [0, 255, 0],
            FxBgType::Black => [0, 0, 0],
            FxBgType::White => [255, 255, 255],
            FxBgType::Magenta => [255, 0, 255],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FxBgType::Blue => "蓝底",
            FxBgType::Green => "绿底",
            FxBgType::Black => "黑底",
            FxBgType::White => "白底",
            FxBgType::Magenta => "洋红底",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FxParams {
    pub model: FxModel,
    pub bg: [u8; 3],
    pub color_tolerance: f32,
    pub black_point: f32,
    pub white_point: f32,
    pub density: f32,
    pub despill: f32,
}

#[inline]
pub(crate) fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// 原站 Xe(bg) 预设：切底色类型时的参数复位（黑底强制 Black 模型，其余 Color）
pub fn fx_preset_for(bg: FxBgType) -> FxParams {
    let black = bg == FxBgType::Black;
    FxParams {
        model: if black { FxModel::Black } else { FxModel::Color },
        bg: bg.pure_color(),
        color_tolerance: if black { 0.0 } else { 3.0 },
        black_point: if black { 0.0 } else { 1.0 },
        white_point: 100.0,
        density: 1.0,
        despill: if matches!(bg, FxBgType::Blue | FxBgType::Green | FxBgType::Magenta) { 20.0 } else { 0.0 },
    }
}

// ---------------------------------------------------------------------------
// 底色场估计（智能识别 + 半透明反解共用）
//
// 老问题：整幅只认「一个底色」。渐变底、暗角、双色底、主体压边、脏边都会让单色
// 失真；底色偏差 10 级，半透明烟雾的浓度就能偏 0.2 以上。
//
// 做法（纯手写、无依赖、O(边框长)）：
//   1) 边框环每条边切 16 段，每段取逐通道中位色 —— 抗噪点 / JPEG 脏边 / 局部异物；
//   2) 每条边用「本边中位色」做种子，剔除离群段（主体压边）后拟合二次曲线
//      c0 + c1·u + c2·u² —— 线性项管渐变，二次项管暗角 / 光照衰减；
//   3) 四条边互相在四角对齐校验，对不上的边判为不可靠（主体压住）；不可靠边用
//      相邻两条垂直边的角点值线性重建，仍不行才退成常数；
//   4) 内部取值 =「上下插值」与「左右插值」的平均 —— 径向暗角在四角自然收敛；
//   5) 置信度 = 场能解释边框像素的加权比例（双色底、杂乱背景会显著掉分）。
//
// 场只描述「形状」：使用时电平锚定到调用方给的底色，因此用户手选取色不会被
// 自动估计悄悄覆盖。
// ---------------------------------------------------------------------------

/// 每条边切分段数
pub(crate) const SEGS: usize = 16;
