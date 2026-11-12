//! 抠图算法门面（Facade）。
//!
//! 所有子模块只处理 RGBA8 缓冲区，不依赖 egui / 文件 / 网络，因此可以独立单测。
//! 上层（`app/`、`video/`）只通过本文件的重导出调用算法，不关心内部文件划分。
//!
//! | 子模块 | 职责 |
//! |---|---|
//! | `image_buffer` | RGBA8 容器、解码、预览降采样 |
//! | `chroma` | 色度键控（绿/蓝/白/自定义/双色/左上角）+ 腐蚀平滑 |
//! | `dual_bg` | 黑底 + 白底双图差分反解 alpha |
//! | `fx` | 半透明抠底的参数类型与底色预设 |
//! | `bg_field` | 边框采样 / 底色场拟合 / 自动识别底色与模型 |
//! | `fx_matte` | 半透明主算法（光效 / 烟雾 / 黑屏光效） |
//! | `unmult` | 黑底 Unmult（alpha=亮度 + 反预乘），用于光效、烟雾素材 |
//! | `tests` | 全部算法单元测试 |

pub mod bg_field;
pub mod chroma;
pub mod dual_bg;
pub mod fx;
pub mod fx_matte;
pub mod image_buffer;
pub mod unmult;

#[cfg(test)]
mod tests;

pub use bg_field::{auto_bg_color, detect_bg_auto};
pub use chroma::{chroma_key, erode_alpha, smooth_alpha};
pub use dual_bg::double_bg;
pub use fx::{FxBgType, FxModel, FxParams, fx_preset_for};
pub use fx_matte::fx_matte;
pub use image_buffer::Img;
pub use unmult::{UnmultParams, unmult_black};
