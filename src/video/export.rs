//! 后台导出任务：解码 → 逐帧抠底 → 编码 → 写盘，全程可取消、可报进度。
//!
//! GIF 走内存编码（便于体积超限后重编码），视频/APNG/PNG 序列走 FFmpeg 管道或
//! 逐帧文件。所有子进程在任务结束或取消时都会被回收，不留僵尸进程。

use crate::video::estimate::{Measured, human};
use crate::video::ffmpeg::{FrameReader, Tools, encode_cmd, spawn};
use crate::video::gif::{GifPrep, encode_gif, prep_frame};
use crate::video::pipeline::{FrameProcessor, composite_flat, rgb_of_u32};
use crate::video::presets::fit_size;
use crate::video::types::{ExportPlan, FitMode, OutFormat, OutputSpec, VideoInfo, VideoSettings};
use eframe::egui;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

#[derive(Clone, Debug)]
pub enum ExportMsg {
    Stage(String),
    Progress { done: u64, total: u64, bytes: u64 },
    Done { path: PathBuf, bytes: u64, note: String },
    Failed(String),
}

pub struct ExportHandle {
    pub rx: Receiver<ExportMsg>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
pub enum SampleMsg {
    Done(Measured),
    Failed(String),
}

/// 启动一次导出；`ctx` 只用来在后台线程结束时请求一次重绘。
pub fn start(
    tools: Tools,
    info: VideoInfo,
    s: VideoSettings,
    plan: ExportPlan,
    out: PathBuf,
    ctx: egui::Context,
) -> ExportHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    thread::spawn(move || {
        let result = run(&tools, &info, &s, &plan, &out, &c, &tx);
        let msg = match result {
            Ok((bytes, note)) => ExportMsg::Done { path: out, bytes, note },
            Err(e) => ExportMsg::Failed(e),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
    ExportHandle { rx, cancel }
}

/// 抽 8 帧实际编码一次，得到真实的每帧字节率，供 UI 做精确预估。
pub fn measure_sample(
    tools: Tools,
    info: VideoInfo,
    s: VideoSettings,
    spec: OutputSpec,
    ctx: egui::Context,
) -> Receiver<SampleMsg> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = sample_impl(&tools, &info, &s, &spec);
        let _ = tx.send(match result {
            Ok(m) => SampleMsg::Done(m),
            Err(e) => SampleMsg::Failed(e),
        });
        ctx.request_repaint();
    });
    rx
}

fn sample_impl(tools: &Tools, info: &VideoInfo, s: &VideoSettings, spec: &OutputSpec) -> Result<Measured, String> {
    let n = 8u32;
    let mut frames: Vec<(Vec<u8>, u32)> = Vec::new();
    let mut proc = FrameProcessor::new(s);
    let prep = GifPrep { w: spec.w, h: spec.h, colors: s.gif_colors, threshold: s.alpha_threshold, dither: s.gif_dither };
    for i in 0..n {
        let t = spec.start + spec.duration * (i as f64 + 0.5) / n as f64;
        let mut rd = FrameReader::open(tools, &info.path, spec.w, spec.h, spec.fps, t, 0.0, true, FitMode::Contain)?;
        if let Some(mut buf) = rd.next() {
            proc.process(&mut buf, spec.w, spec.h);
            prep_frame(&mut buf, prep);
            frames.push((buf, spec.delay_ms));
        }
        rd.close();
    }
    if frames.is_empty() {
        return Err("取不到样本帧，检查视频能否解码".into());
    }
    let cnt = frames.len() as f64;
    let mut it = frames.into_iter();
    let bytes = encode_gif(spec.w, spec.h, s.gif_colors, s.loop_count, None, n as u64, || it.next())?;
    let per = (bytes.len() as f64 - 2048.0).max(1024.0) / cnt;
    Ok(Measured { bytes_per_frame: per, pixels: spec.w as u64 * spec.h as u64, colors: s.gif_colors, dither: s.gif_dither })
}

fn tighten(p: &mut ExportPlan) {
    p.spec.fps = (p.spec.fps * 0.80).max(5.0);
    p.spec.frames = (p.spec.duration * p.spec.fps).ceil().max(1.0) as u64;
    p.spec.delay_ms = (1000.0 / p.spec.fps).round().clamp(10.0, 250.0) as u32;
    p.spec.w = ((p.spec.w as f64 * 0.86) as u32).max(160) & !1;
    p.spec.h = ((p.spec.h as f64 * 0.86) as u32).max(120) & !1;
    p.colors = if p.colors > 128 { 128 } else if p.colors > 64 { 64 } else { 32 };
    p.speed = crate::video::gif::speed_for(p.colors);
}

fn run(
    tools: &Tools,
    info: &VideoInfo,
    s: &VideoSettings,
    plan: &ExportPlan,
    out: &Path,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportMsg>,
) -> Result<(u64, String), String> {
    let fmt = OutFormat::from_key(&s.out_format);
    let _ = tx.send(ExportMsg::Stage(format!("准备导出 {}…", fmt.short())));

    if fmt == OutFormat::PngSeq {
        return run_png_seq(tools, info, s, plan, out, cancel, tx);
    }

    let mut cur = plan.clone();
    let mut last_note = plan.note.clone();
    for attempt in 0..3 {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        let _ = tx.send(ExportMsg::Stage(if attempt == 0 {
            format!("正在导出 {}…", fmt.short())
        } else {
            format!("体积超标，第 {} 次自动降规格重编码…", attempt + 1)
        }));

        let r = if fmt.is_gif() {
            encode_gif_once(tools, info, s, &cur, out, cancel, tx)
        } else {
            encode_video_once(tools, info, s, &cur, out, fmt, cancel, tx)
        };

        match r {
            Ok((bytes, note)) => {
                last_note = if note.is_empty() { cur.note.clone() } else { note };
                let over = cur.budget.map(|b| bytes > b).unwrap_or(false);
                if fmt.is_gif() && over && s.strict_limit && attempt < 2 {
                    let _ = std::fs::remove_file(out);
                    tighten(&mut cur);
                    continue;
                }
                if fmt.is_gif() && over {
                    last_note = format!("{}；实际 {} 仍略超上限，可放宽上限或再降规格", last_note, human(bytes));
                }
                return Ok((bytes, last_note));
            }
            Err(e) => {
                if (e.contains("体积超出预算") || e.contains("超标")) && attempt < 2 {
                    let _ = std::fs::remove_file(out);
                    tighten(&mut cur);
                    continue;
                }
                let _ = std::fs::remove_file(out);
                return Err(e);
            }
        }
    }
    Err("多次自动降级后仍无法完成导出".into())
}

/// 单轮 GIF 编码：解码 → 抠底 → 二值透明 → 写内存 → 落盘。
fn encode_gif_once(
    tools: &Tools,
    info: &VideoInfo,
    s: &VideoSettings,
    plan: &ExportPlan,
    out: &Path,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportMsg>,
) -> Result<(u64, String), String> {
    let spec = &plan.spec;
    let fit = FitMode::from_key(&s.fit);
    let mut reader = FrameReader::open(tools, &info.path, spec.w, spec.h, spec.fps, spec.start, spec.duration, false, fit)?;
    let mut proc = FrameProcessor::new(s);
    let prep = GifPrep { w: spec.w, h: spec.h, colors: plan.colors, threshold: s.alpha_threshold, dither: s.gif_dither };
    let mut done = 0u64;
    let total = spec.frames;
    let mut it = || -> Option<(Vec<u8>, u32)> {
        if cancel.load(Ordering::Relaxed) { return None; }
        let mut buf = reader.next()?;
        proc.process(&mut buf, spec.w, spec.h);
        prep_frame(&mut buf, prep);
        done += 1;
        if done % 4 == 0 || done == total {
            let _ = tx.send(ExportMsg::Progress { done, total, bytes: 0 });
        }
        Some((buf, spec.delay_ms))
    };
    let bytes = encode_gif(spec.w, spec.h, plan.colors, s.loop_count, plan.budget, total, &mut it);
    reader.close();
    let data = bytes?;
    if cancel.load(Ordering::Relaxed) { return Err("已取消".into()); }
    std::fs::write(out, &data).map_err(|e| format!("写入 GIF 失败：{e}"))?;
    let n = data.len() as u64;
    let _ = tx.send(ExportMsg::Progress { done: total, total, bytes: n });
    Ok((n, plan.note.clone()))
}

/// 单轮视频编码：把 RGBA 裸帧喂给 FFmpeg 编码器 stdin。
fn encode_video_once(
    tools: &Tools,
    info: &VideoInfo,
    s: &VideoSettings,
    plan: &ExportPlan,
    out: &Path,
    fmt: OutFormat,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportMsg>,
) -> Result<(u64, String), String> {
    let spec = &plan.spec;
    let fit = FitMode::from_key(&s.fit);
    let mut cmd = encode_cmd(tools, out, spec, s, fmt);
    let (mut enc, enc_err) = spawn(&mut cmd)?;
    let mut stdin = enc.stdin.take().ok_or("编码器没有 stdin")?;
    let mut reader = FrameReader::open(tools, &info.path, spec.w, spec.h, spec.fps, spec.start, spec.duration, false, fit)?;
    let mut proc = FrameProcessor::new(s);
    let bg = rgb_of_u32(s.flat_color);
    let mut done = 0u64;
    let mut io_err: Option<String> = None;

    while let Some(mut buf) = reader.next() {
        if cancel.load(Ordering::Relaxed) { break; }
        proc.process(&mut buf, spec.w, spec.h);
        if fmt == OutFormat::Mp4Flat {
            composite_flat(&mut buf, bg);
        }
        if let Err(e) = stdin.write_all(&buf) {
            io_err = Some(format!("写入编码器失败：{e}"));
            break;
        }
        done += 1;
        if done % 4 == 0 {
            let _ = tx.send(ExportMsg::Progress { done, total: spec.frames, bytes: 0 });
        }
    }
    reader.close();
    drop(stdin);
    let status = enc.wait();
    if let Some(e) = io_err { return Err(e); }
    if cancel.load(Ordering::Relaxed) { return Err("已取消".into()); }
    if !status.map(|s| s.success()).unwrap_or(false) {
        let extra = enc_err.lock().map(|x| x.trim().to_string()).unwrap_or_default();
        return Err(format!("FFmpeg 编码失败：{}", if extra.is_empty() { "未知错误（检查该编码器是否可用）".into() } else { extra }));
    }
    let n = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    Ok((n, String::new()))
}

/// PNG 序列：输出到 `<文件名>_frames/` 目录，逐帧带 alpha。
fn run_png_seq(
    tools: &Tools,
    info: &VideoInfo,
    s: &VideoSettings,
    plan: &ExportPlan,
    out: &Path,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportMsg>,
) -> Result<(u64, String), String> {
    let spec = &plan.spec;
    let dir = out.with_file_name(format!(
        "{}_frames",
        out.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|| "frames".into())
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建序列目录失败：{e}"))?;
    let mut reader = FrameReader::open(tools, &info.path, spec.w, spec.h, spec.fps, spec.start, spec.duration, false, FitMode::from_key(&s.fit))?;
    let mut proc = FrameProcessor::new(s);
    let mut done = 0u64;
    let mut total_bytes = 0u64;
    while let Some(mut buf) = reader.next() {
        if cancel.load(Ordering::Relaxed) { break; }
        proc.process(&mut buf, spec.w, spec.h);
        let path = dir.join(format!("frame_{:05}.png", done + 1));
        let im = ::image::RgbaImage::from_raw(spec.w, spec.h, buf).ok_or("帧尺寸不符")?;
        im.save(&path).map_err(|e| format!("保存帧失败：{e}"))?;
        total_bytes += std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        done += 1;
        if done % 4 == 0 {
            let _ = tx.send(ExportMsg::Progress { done, total: spec.frames, bytes: total_bytes });
        }
    }
    reader.close();
    Ok((total_bytes, format!("{} 帧 PNG，目录 {}", done, dir.display())))
}
