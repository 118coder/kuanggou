//! 视频工作区的运行时状态与消息轮询。
//!
//! 这是 UI 与后端之间唯一的「中枢」：持有素材信息、预览帧与纹理、后台任务句柄，
//! 负责把预览/试编码/导出三个后台通道的消息收敛到 UI 线程。
//! 它不依赖 `app::` 或 `ui::`，因此图片工作区完全不知道视频工作区的存在。

use crate::algo::{FxModel, Img};
use crate::video::estimate::{self, Estimate, Measured};
use crate::video::export::{self, ExportHandle, ExportMsg, SampleMsg};
use crate::video::ffmpeg::{FrameReader, Tools, locate, probe};
use crate::video::pipeline::FrameProcessor;
use crate::video::presets::resolve_spec;
use crate::video::types::{ExportPlan, FitMode, KeyMode, OutFormat, VideoInfo, VideoSettings};
use eframe::egui;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;
use std::time::Instant;

/// 预览解码线程回传的消息。
pub enum PreviewMsg {
    Frame(Img, f64),
    Done,
    Failed(String),
}

/// 预览底色（与图片工作区的 PreviewBg 相互独立）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VBackdrop {
    Checker,
    Black,
    White,
    Green,
}

impl VBackdrop {
    pub fn label(self) -> &'static str {
        match self { VBackdrop::Checker => "棋盘", VBackdrop::Black => "黑", VBackdrop::White => "白", VBackdrop::Green => "绿" }
    }
}

/// 视频工作区全部状态。
pub struct VideoState {
    /// 可持久化参数。
    pub s: VideoSettings,
    pub info: Option<VideoInfo>,
    pub tools: Option<Tools>,
    pub status: String,
    pub status_warn: bool,
    pub status_born: Instant,
    pub last_dir: Option<PathBuf>,
    pub last_export: Option<PathBuf>,

    // 预览
    pub frame_orig: Option<Img>,
    pub frame_result: Option<Img>,
    pub tex_orig: Option<TextureHandle>,
    pub tex_result: Option<TextureHandle>,
    pub backdrop: VBackdrop,
    pub show_orig: bool,
    pub time: f64,
    pub playing: bool,
    pub preview_rx: Option<Receiver<PreviewMsg>>,
    preview_cancel: Arc<AtomicBool>,
    pub preview_w: u32,
    pub preview_h: u32,
    pub(crate) auto_bg: Option<[u8; 3]>,
    pub(crate) auto_model: FxModel,
    pub(crate) auto_tried: bool,

    // 估算
    pub estimate: Option<Estimate>,
    pub measured: Option<Measured>,
    pub plan: Option<ExportPlan>,
    pub sample_rx: Option<Receiver<SampleMsg>>,
    pub sampling: bool,

    // 导出
    pub export: Option<ExportHandle>,
    pub exporting: bool,
    pub progress: f32,
    pub progress_text: String,

    /// 参数快照，用于检测 UI 里的改动并触发重算。
    last_sig: String,
}

impl Default for VideoState {
    fn default() -> Self {
        let s = VideoSettings::default();
        let tools = locate(&s.ffmpeg_path);
        let status = if tools.is_some() {
            "FFmpeg 已就绪 · 拖入视频或点「打开视频」".into()
        } else {
            "未找到 FFmpeg：请在右栏「FFmpeg 设置」里指定 ffmpeg.exe，或把它放到本程序同目录".into()
        };
        Self {
            s,
            info: None,
            tools,
            status,
            status_warn: false,
            status_born: Instant::now(),
            last_dir: None,
            last_export: None,
            frame_orig: None,
            frame_result: None,
            tex_orig: None,
            tex_result: None,
            backdrop: VBackdrop::Checker,
            show_orig: false,
            time: 0.0,
            playing: false,
            preview_rx: None,
            preview_cancel: Arc::new(AtomicBool::new(false)),
            preview_w: 0,
            preview_h: 0,
            auto_bg: None,
            auto_model: FxModel::Color,
            auto_tried: false,
            estimate: None,
            measured: None,
            plan: None,
            sample_rx: None,
            sampling: false,
            export: None,
            exporting: false,
            progress: 0.0,
            progress_text: String::new(),
            last_sig: String::new(),
        }
    }
}

impl VideoState {
    pub fn settings(&self) -> &VideoSettings { &self.s }
    pub fn apply_settings(&mut self, s: VideoSettings) {
        self.s = s;
        self.tools = locate(&self.s.ffmpeg_path);
        self.last_sig.clear();
    }

    pub fn toast(&mut self, msg: impl Into<String>, warn: bool) {
        self.status = msg.into();
        self.status_warn = warn;
        self.status_born = Instant::now();
    }

    /// 重新探测 FFmpeg（用户手动指定路径后调用）。
    pub fn relink_ffmpeg(&mut self) {
        self.tools = locate(&self.s.ffmpeg_path);
        self.toast(
            if self.tools.is_some() { "FFmpeg 已就绪" } else { "这个路径下没有可用的 ffmpeg.exe" },
            self.tools.is_none(),
        );
    }

    pub fn has_video(&self) -> bool { self.info.is_some() }

    /// 载入视频：探测信息 → 设置裁剪区间 → 抽首帧 → 重算预估。
    pub fn load_video(&mut self, path: PathBuf, ctx: &egui::Context) {
        let Some(tools) = self.tools.clone() else {
            self.toast("先配置 FFmpeg 才能载入视频", true);
            return;
        };
        match probe(&tools, &path) {
            Ok(info) => {
                self.last_dir = path.parent().map(|p| p.to_path_buf());
                if self.s.trim_end <= 0.01 || self.s.trim_end as f64 > info.duration {
                    self.s.trim_end = info.duration as f32;
                }
                self.s.trim_start = self.s.trim_start.min(info.duration.max(0.0) as f32);
                let name = path.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                self.info = Some(info);
                self.time = self.s.trim_start as f64;
                self.auto_bg = None;
                self.auto_tried = false;
                self.measured = None;
                self.recompute();
                self.toast(format!("已载入 {name} · {}", self.info.as_ref().unwrap().label()), false);
                self.seek(self.s.trim_start as f64, ctx, false);
            }
            Err(e) => self.toast(format!("载入失败：{e}"), true),
        }
    }

    /// 参数/规格变化后重算预估与自动降级计划（纯本地，不阻塞）。
    pub fn recompute(&mut self) {
        let Some(info) = &self.info else { self.estimate = None; self.plan = None; return };
        let plan = estimate::plan(&self.s, info, self.measured.as_ref());
        let fmt = OutFormat::from_key(&self.s.out_format);
        let mut est = estimate::estimate(fmt, &plan.spec, &self.s, self.measured.as_ref());
        if !plan.note.is_empty() {
            est.note = format!("{}；{}", est.note, plan.note);
        }
        self.estimate = Some(est);
        self.plan = Some(plan);
    }

    /// 每帧调用：检测参数改动、收取后台消息、刷新预览纹理。
    pub fn poll(&mut self, ctx: &egui::Context) {
        let sig = serde_json::to_string(&self.s).unwrap_or_default();
        if sig != self.last_sig {
            self.last_sig = sig;
            if self.info.is_some() {
                self.recompute();
                self.auto_bg = None;
                self.auto_tried = false;
                self.reprocess(ctx);
            }
            if self.tools.is_none() && !self.s.ffmpeg_path.is_empty() {
                self.relink_ffmpeg();
            }
        }
        self.poll_preview(ctx);
        self.poll_sample(ctx);
        self.poll_export(ctx);
        if self.playing {
            ctx.request_repaint();
        }
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.preview_rx.take() else { return };
        let mut got = false;
        loop {
            match rx.try_recv() {
                Ok(PreviewMsg::Frame(img, t)) => {
                    self.frame_orig = Some(img);
                    self.time = t;
                    got = true;
                }
                Ok(PreviewMsg::Done) => {
                    self.playing = false;
                    ctx.request_repaint();
                }
                Ok(PreviewMsg::Failed(e)) => {
                    self.playing = false;
                    self.toast(e, true);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.playing = false;
                    break;
                }
            }
        }
        self.preview_rx = Some(rx);
        if got {
            self.reprocess(ctx);
        }
    }

    fn poll_sample(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.sample_rx.take() else { return };
        match rx.try_recv() {
            Ok(SampleMsg::Done(m)) => {
                self.measured = Some(m);
                self.sampling = false;
                self.recompute();
                self.toast(format!("精确预估已更新：样本每帧 {:.0} B", m.bytes_per_frame), false);
            }
            Ok(SampleMsg::Failed(e)) => {
                self.sampling = false;
                self.toast(format!("精确预估失败：{e}"), true);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => self.sample_rx = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.sampling = false,
        }
        ctx.request_repaint();
    }

    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(handle) = self.export.as_ref() else { return };
        let mut msgs = Vec::new();
        loop {
            match handle.rx.try_recv() {
                Ok(m) => msgs.push(m),
                Err(_) => break,
            }
        }
        for m in msgs {
            match m {
                ExportMsg::Stage(s) => self.progress_text = s,
                ExportMsg::Progress { done, total, .. } => {
                    self.progress = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                    self.progress_text = format!("{done} / {total} 帧");
                }
                ExportMsg::Done { path, bytes, note } => {
                    self.exporting = false;
                    self.progress = 1.0;
                    self.last_export = Some(path.clone());
                    self.export = None;
                    let extra = if note.is_empty() { String::new() } else { format!(" · {note}") };
                    self.toast(format!("已导出 {}（{}）{extra}", path.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default(), estimate::human(bytes)), false);
                }
                ExportMsg::Failed(e) => {
                    self.exporting = false;
                    self.progress = 0.0;
                    self.export = None;
                    self.toast(format!("导出失败：{e}"), true);
                }
            }
        }
        ctx.request_repaint();
    }

    /// 用当前参数重新处理已缓存的那帧（不重新解码，参数拖动时即时反馈）。
    pub fn reprocess(&mut self, ctx: &egui::Context) {
        let Some(orig) = self.frame_orig.clone() else { return };
        let mut buf = orig.buf.clone();
        let mut proc = FrameProcessor::new(&self.s);
        proc.auto_bg = self.auto_bg;
        proc.auto_model = self.auto_model;
        proc.auto_tried = self.auto_tried;
        proc.process(&mut buf, orig.w as u32, orig.h as u32);
        self.auto_bg = proc.auto_bg;
        self.auto_model = proc.auto_model;
        self.auto_tried = proc.auto_tried;
        let w = orig.w;
        let h = orig.h;
        self.frame_result = Some(Img::from_raw(w, h, buf.clone()));
        let ci_orig = ColorImage::from_rgba_unmultiplied([w, h], &orig.buf);
        let ci_res = ColorImage::from_rgba_unmultiplied([w, h], &buf);
        match &mut self.tex_orig {
            Some(t) => t.set(ci_orig, TextureOptions::LINEAR),
            None => self.tex_orig = Some(ctx.load_texture("video_orig", ci_orig, TextureOptions::LINEAR)),
        }
        match &mut self.tex_result {
            Some(t) => t.set(ci_res, TextureOptions::LINEAR),
            None => self.tex_result = Some(ctx.load_texture("video_result", ci_res, TextureOptions::LINEAR)),
        }
        ctx.request_repaint();
    }

    /// 重新自动取样底色（自动识别模式用）。
    /// 清除自动识别结果（切换键色模式 / 换素材时调用）。
    pub fn reset_auto(&mut self) {
        self.auto_bg = None;
        self.auto_tried = false;
    }

    pub fn resample_auto(&mut self, ctx: &egui::Context) {
        self.auto_bg = None;
        self.auto_tried = false;
        self.reprocess(ctx);
        match self.auto_bg {
            Some([r, g, b]) => self.toast(format!("已重新取样底色 #{r:02X}{g:02X}{b:02X}"), false),
            None => self.toast("自动识别失败，可改用固定键色或点画布取色", true),
        }
    }

    fn preview_dims(&self) -> (u32, u32) {
        let Some(info) = &self.info else { return (640, 360) };
        let k = (720.0 / info.w.max(info.h) as f64).min(1.0);
        (((info.w as f64 * k) as u32).max(2) & !1, ((info.h as f64 * k) as u32).max(2) & !1)
    }

    fn cancel_preview(&mut self) {
        self.preview_cancel.store(true, Ordering::Relaxed);
        self.playing = false;
    }

    /// 跳到指定时间抽一帧；`play_after=true` 时改为连续播放。
    pub fn seek(&mut self, t: f64, ctx: &egui::Context, play_after: bool) {
        let Some(tools) = self.tools.clone() else { return };
        let Some(info) = self.info.clone() else { return };
        self.cancel_preview();
        let (w, h) = self.preview_dims();
        self.preview_w = w;
        self.preview_h = h;
        let fps = if play_after { self.s.fps_preset.parse::<f64>().unwrap_or(info.fps).clamp(4.0, 30.0) } else { info.fps };
        let spec = resolve_spec(&self.s, &info);
        let dur = (spec.end - t).max(0.0);
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel = cancel.clone();
        let (tx, rx) = sync_channel::<PreviewMsg>(3);
        self.preview_rx = Some(rx);
        self.playing = play_after;
        self.time = t;
        let ctx2 = ctx.clone();
        thread::spawn(move || {
            let rd = FrameReader::open(&tools, &info.path, w, h, fps, t, dur, !play_after, FitMode::Contain);
            let mut rd = match rd {
                Ok(r) => r,
                Err(e) => { let _ = tx.send(PreviewMsg::Failed(e)); ctx2.request_repaint(); return; }
            };
            let mut idx = 0u64;
            let clock = std::time::Instant::now();
            loop {
                if cancel.load(Ordering::Relaxed) { break; }
                // 播放时按墙钟节流，避免「解码多快就播多快」变成快进。
                if play_after {
                    let due = idx as f64 / fps.max(1.0);
                    loop {
                        let lag = due - clock.elapsed().as_secs_f64();
                        if lag <= 0.0 || cancel.load(Ordering::Relaxed) { break; }
                        thread::sleep(std::time::Duration::from_millis((lag * 1000.0).min(20.0) as u64));
                    }
                    if cancel.load(Ordering::Relaxed) { break; }
                }
                match rd.next() {
                    Some(buf) => {
                        let tt = t + idx as f64 / fps.max(1.0);
                        if tx.send(PreviewMsg::Frame(Img::from_raw(w as usize, h as usize, buf), tt)).is_err() { break; }
                        idx += 1;
                        ctx2.request_repaint();
                    }
                    None => break,
                }
            }
            rd.close();
            let _ = tx.send(PreviewMsg::Done);
            ctx2.request_repaint();
        });
    }

    pub fn toggle_play(&mut self, ctx: &egui::Context) {
        if self.playing {
            self.cancel_preview();
            self.toast("已暂停", false);
        } else {
            let t = self.time;
            self.seek(t, ctx, true);
        }
    }

    /// 画布点选颜色：写入当前模式的键色（视频工作区自己的取色，不改图片参数）。
    pub fn pick_color(&mut self, img_pos: egui::Pos2, img_rect: egui::Rect, ctx: &egui::Context) {
        let Some(orig) = &self.frame_orig else { return };
        let u = ((img_pos.x - img_rect.left()) / img_rect.width()).clamp(0.0, 1.0);
        let v = ((img_pos.y - img_rect.top()) / img_rect.height()).clamp(0.0, 1.0);
        let x = ((u * orig.w as f32) as usize).min(orig.w - 1);
        let y = ((v * orig.h as f32) as usize).min(orig.h - 1);
        let o = (y * orig.w + x) * 4;
        let (r, g, b) = (orig.buf[o], orig.buf[o + 1], orig.buf[o + 2]);
        self.s.key_color = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        if KeyMode::from_key(&self.s.key_mode) == KeyMode::Auto {
            self.s.key_mode = KeyMode::Custom.key().to_string();
        }
        self.toast(format!("已取样键色 #{r:02X}{g:02X}{b:02X}"), false);
        self.reprocess(ctx);
    }

    /// 后台抽帧试编码，得到更准的体积预估。
    pub fn request_measure(&mut self, ctx: &egui::Context) {
        let (Some(tools), Some(info)) = (self.tools.clone(), self.info.clone()) else {
            self.toast("先载入视频再精确预估", true);
            return;
        };
        if self.sampling { return; }
        self.sampling = true;
        let spec = match &self.plan { Some(p) => p.spec, None => resolve_spec(&self.s, &info) };
        self.sample_rx = Some(export::measure_sample(tools, info, self.s.clone(), spec, ctx.clone()));
        self.toast("正在抽 8 帧试编码…", false);
    }

    /// 打开保存对话框并启动导出。
    pub fn start_export(&mut self, ctx: &egui::Context) {
        if self.exporting { return; }
        let (Some(tools), Some(info)) = (self.tools.clone(), self.info.clone()) else {
            self.toast("先载入视频并确认 FFmpeg 可用", true);
            return;
        };
        self.recompute();
        let Some(plan) = self.plan.clone() else { return };
        let fmt = OutFormat::from_key(&self.s.out_format);
        let stamp = {
            let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            format!("{n}")
        };
        let default_name = format!("cutout_{stamp}.{}", fmt.ext());
        let mut dlg = rfd::FileDialog::new().set_file_name(&default_name).add_filter(fmt.short(), &[fmt.ext()]);
        if let Some(dir) = &self.last_dir { dlg = dlg.set_directory(dir); }
        let Some(path) = dlg.save_file() else { return };
        self.last_dir = path.parent().map(|p| p.to_path_buf());
        self.exporting = true;
        self.progress = 0.0;
        self.progress_text = "准备中…".into();
        self.export = Some(export::start(tools, info, self.s.clone(), plan, path, ctx.clone()));
        self.toast("导出已开始", false);
    }

    pub fn cancel_export(&mut self) {
        if let Some(h) = &self.export {
            h.cancel.store(true, Ordering::Relaxed);
        }
        self.toast("正在取消导出…", false);
    }

    /// 清理资源（切换工作区时可调用，避免后台线程继续跑）。
    pub fn pause_all(&mut self) {
        self.cancel_preview();
    }

    /// 当前应该预览的纹理（原图速览 / 结果）。
    pub fn active_tex(&self) -> Option<&TextureHandle> {
        if self.show_orig { self.tex_orig.as_ref() } else { self.tex_result.as_ref() }
    }

    pub fn backdrop_color(&self) -> Color32 {
        match self.backdrop {
            VBackdrop::Checker => Color32::from_rgb(0xF0, 0xF0, 0xEE),
            VBackdrop::Black => Color32::from_rgb(0x16, 0x16, 0x16),
            VBackdrop::White => Color32::WHITE,
            VBackdrop::Green => Color32::from_rgb(0x00, 0xB0, 0x40),
        }
    }
}
