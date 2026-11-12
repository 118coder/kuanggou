# 代码架构（v1.2）

> 目标：图片与视频两个工作区完全独立；每个文件只干一件事；新功能只新增文件、不改老文件。
> 所有模块首行都有 `//!` 文档注释，说明「这个文件负责什么、不负责什么」，本文件是整体的地图。

## 1. 目录树

```
src/
├── main.rs                程序入口：窗口参数、字体/图标、启动 eframe
├── update.rs              自动检查更新（GitHub Releases API）
├── algo/                  ── 纯算法层（无 UI、无 IO）──
│   ├── mod.rs             门面：对外只暴露函数与类型，屏蔽内部分文件
│   ├── image_buffer.rs    Img：RGBA8 容器、解码、预览降采样
│   ├── chroma.rs          色度键控：绿/蓝幕、白底泛洪、自选色、双色键、左上角连通域
│   ├── bg_field.rs        「底色场」智能识别：边框分段采样 → 二次曲面拟合 → 逐点反解
│   ├── dual_bg.rs         双背景差分（黑底图 + 白底图 → 真实 alpha）
│   ├── fx.rs              半透明抠底的参数类型与底色预设
│   ├── fx_matte.rs        半透明抠底主算法（trimap 投影剔除 + 溢色抑制）
│   ├── unmult.rs          Unmult 去黑底（AE Unmult 同思路：亮度反解 + 反预乘）
│   └── tests.rs           算法单测（37 项，含 Unmult）
├── app/                   ── 状态与逻辑层（不绘制像素）──
│   ├── mod.rs             eframe::App 实现：save / logic / ui 三件事的接线
│   ├── types.rs           枚举与持久化结构（Mode / Workspace / Persisted / Snapshot…）
│   ├── state.rs           App 字段 + 构造 / 存读盘 / 撤销重做 / 工作区切换
│   ├── image.rs           图片工作区逻辑：载入、预览代理、算法调度、全分辨率导出
│   ├── input.rs           输入路由：拖放、剪贴板、快捷键、文件对话框
│   └── update_ui.rs       更新检查的 UI 侧状态机
├── ui/                    ── 绘制层（只读状态 + 写回参数）──
│   ├── mod.rs             整窗精确布局：标题栏/三色条/工作区条/命令栏/舞台+参数栏/状态栏
│   ├── theme.rs           设计令牌：颜色、切角、高度、字体、图标
│   ├── widgets.rs         通用部件：切角按钮、分段控件、滑杆、芯片、Toast、放大镜
│   ├── title_bar.rs       自绘标题栏
│   ├── workspace_bar.rs   图片 / 视频工作区切换条
│   ├── command_bar.rs     图片模式命令栏（10 个模式标签 + 4 个动作）
│   ├── stage.rs           图片舞台：画布、对比线、缩放平移、取色、双底槽位
│   ├── props.rs           图片参数栏（按模式切换参数组）
│   ├── footer.rs          状态栏
│   ├── help.rs            F1 帮助
│   ├── video.rs           视频命令栏 + 视频舞台（画布、时间轴、取色）
│   └── video_props.rs     视频参数栏（键色 / Unmult / 规格 / GIF 高级 / 体积预估）
└── video/                 ── 视频后端（可单独测试，不依赖 UI）──
    ├── mod.rs             门面：模块装配 + re-export
    ├── types.rs           数据契约：VideoInfo / KeyMode / OutFormat / VideoSettings / OutputSpec
    ├── presets.rs         规格预设表 + 设置 → 实际输出规格解析
    ├── ffmpeg.rs          FFmpeg/FFprobe 封装：探测、解码命令、编码命令、帧读取器
    ├── pipeline.rs        逐帧处理：去底 → 适配 → GIF 预处理（二值透明 / 预量化）
    ├── gif.rs             GIF 编码（含体积预算的提前中止）
    ├── estimate.rs        体积预估 + 体积上限的参数规划（自动降帧/降尺寸/降色）
    ├── export.rs          后台导出任务（可取消、可报进度、超限自动重编码）
    └── state.rs           视频工作区运行时状态（预览线程、消息轮询、纹理）
```

## 2. 依赖方向（只允许单向）

```
main → app → ui → widgets/theme
        app → algo
        app → video → algo
        ui  → video（只读状态、调命令）
```

- `algo` 不认识 `ui`、`app`、`video`，可单独跑测试。
- `video` 不认识 `ui`、`app`，只依赖 `algo` 与 ffmpeg 命令行。
- `ui` 不写业务逻辑：拖滑杆只改 `App` 的字段，逻辑由 `app/*` 或 `video/state.rs` 的轮询接管。

## 3. 两个工作区为什么互不干扰

| | 图片工作区 | 视频工作区 |
|---|---|---|
| 状态 | `App` 上的一堆字段（`mode`、`tol`、`black/white`…） | 独立的 `App::video: VideoState` |
| 参数持久化 | `Persisted` 的图片字段 | `Persisted::video: Option<VideoSettings>` |
| 撤销栈 | `App::undo` / `redo` | 无（视频按参数实时重算，不做快照） |
| 刷新 | `App::refresh()` 按脏标记走快/慢代理 | `VideoState::poll()` 检测参数签名变化 |
| 渲染 | `ui/stage.rs` + `ui/props.rs` | `ui/video.rs` + `ui/video_props.rs` |

切换工作区只改 `App::workspace`，两边状态都原样保留；视频工作区不参与图片的高分辨率刷新，
播放/导出时才请求重绘。

## 4. 线程模型

- **UI 线程**：egui 绘制 + 参数写回。
- **预览线程**（`video/state.rs::seek`）：一条 ffmpeg 解码进程 → 有界通道（3 帧）→ UI 收帧后原地重算。
  播放时按墙钟节流，保证「播放」是真实速度；拖动时间轴会 `cancel` 旧线程再开新的。
- **试编码线程**（`video/export.rs::measure_sample`）：抽 8 帧走完整 GIF 编码，回填 `Measured`。
- **导出线程**（`video/export.rs::start`）：解码 → 抠底 → 编码 → 写盘；`Arc<AtomicBool>` 取消，
  `Sender<ExportMsg>` 回传阶段/进度/结果；GIF 超预算时在内存里降规格重编码，最多 3 轮。
- 所有子进程在任务结束或取消时 `kill + wait`，不留僵尸。

## 5. 新增一个功能该改哪里

- 新增抠图算法 → `algo/` 加文件 + `algo/mod.rs` 挂一行，参数进 `app/types.rs`（图片）或 `video/types.rs`（视频）。
- 新增导出格式 → `OutFormat` 加一个枚举项（`label/short/ext/key`）+ `video/ffmpeg.rs::encode_cmd` 加分支。
- 新增规格预设 → `video/presets.rs` 的 `SPECS` 表加一行，UI 自动出现。
- 新增 UI 区块 → `ui/` 加文件，只在 `ui/mod.rs` 里调用一次。

## 6. 构建

本项目由云端编译（GitHub Actions，`.github/workflows/build-windows.yml`）：推送 `main` 后自动
`cargo test` → `cargo build --release`，产物 `cutout_tool.exe` 在 Run 的 Artifacts 里。
本地只做 `cargo check` 快速验证，不做 release 打包。
