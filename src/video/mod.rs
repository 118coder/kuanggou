//! 视频工作区后端门面（模块装配层）。
//!
//! 目录职责，每个模块各司其职：
//! | 模块 | 职责 |
//! |---|---|
//! | `types` | 数据契约：设置、媒体信息、输出规格、格式枚举 |
//! | `presets` | 规格预设表与「设置 → 实际输出规格」解析 |
//! | `ffmpeg` | FFmpeg / FFprobe 探测、解码与编码进程封装 |
//! | `pipeline` | 逐帧抠底、画面适配、GIF 透明二值化 |
//! | `gif` | GIF 编码（1-bit 透明 + 抖动 + 预量化） |
//! | `estimate` | 导出体积预估、样本实测、体积上限自动降级 |
//! | `export` | 后台导出任务、进度、取消、样本试编码 |
//! | `state` | UI 持有的完整状态与三个后台通道的消息轮询 |
//!
//! 本目录只依赖 `algo`（抠图算法）与 `egui`（纹理/上下文），不反向依赖
//! `app::` / `ui::`，因此图片工作区与视频工作区可以完全独立演进。

pub mod estimate;
pub mod export;
pub mod ffmpeg;
pub mod gif;
pub mod pipeline;
pub mod presets;
pub mod state;
pub mod types;

pub use state::VideoState;
pub use types::*;
