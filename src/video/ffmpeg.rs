//! FFmpeg / FFprobe 进程封装：可执行文件探测、媒体信息解析、解码与编码命令。
//!
//! 视频工作区不内置解码器，而是调用外部 FFmpeg（本机已装或随 exe 放同目录）。
//! 所有帧以 `rawvideo / rgba` 通过管道传输，程序内逐帧调用 `algo` 抠底，
//! 再交给编码器。这样视频编解码能力由 FFmpeg 提供，抠图算法仍完全自研。

use crate::video::types::{FitMode, OutFormat, OutputSpec, VideoInfo, VideoSettings};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

/// 探测到的工具路径。
#[derive(Clone, Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: Option<PathBuf>,
}

fn works(p: &Path) -> bool {
    Command::new(p)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) { format!("{base}.exe") } else { base.to_string() }
}

/// 按优先级找 FFmpeg：用户指定 → exe 同目录 → 环境变量 → PATH → 常见安装位置。
pub fn locate(override_path: &str) -> Option<Tools> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if !override_path.trim().is_empty() {
        let p = PathBuf::from(override_path.trim());
        if p.is_dir() {
            candidates.push(p.join(exe_name("ffmpeg")));
        } else {
            candidates.push(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(exe_name("ffmpeg")));
            candidates.push(dir.join("ffmpeg").join(exe_name("ffmpeg")));
        }
    }
    if let Ok(v) = std::env::var("FFMPEG_PATH") {
        let p = PathBuf::from(v);
        candidates.push(if p.is_dir() { p.join(exe_name("ffmpeg")) } else { p });
    }
    candidates.push(PathBuf::from(exe_name("ffmpeg"))); // PATH
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Program Files\FFmpeg\bin\ffmpeg.exe"));
        candidates.push(PathBuf::from(r"C:\Program Files\FFmpeg\ffmpeg.exe"));
        candidates.push(PathBuf::from(r"C:\ffmpeg\bin\ffmpeg.exe"));
        if let Ok(home) = std::env::var("USERPROFILE") {
            candidates.push(PathBuf::from(&home).join("scoop/apps/ffmpeg/current/bin/ffmpeg.exe"));
            candidates.push(PathBuf::from(&home).join("AppData/Local/Microsoft/WinGet/Packages"));
        }
    }

    for c in candidates {
        if c.is_file() && works(&c) {
            let mut probe = c.clone();
            probe.set_file_name(exe_name("ffprobe"));
            if !probe.is_file() {
                probe = PathBuf::from(exe_name("ffprobe"));
            }
            let ffprobe = if works(&probe) { Some(probe) } else { None };
            return Some(Tools { ffmpeg: c, ffprobe });
        }
    }
    // 没找到具体文件，但 PATH 里也许能用
    let generic = PathBuf::from(exe_name("ffmpeg"));
    if works(&generic) {
        let probe = PathBuf::from(exe_name("ffprobe"));
        return Some(Tools { ffmpeg: generic, ffprobe: if works(&probe) { Some(probe) } else { None } });
    }
    None
}

/// 启动子进程并把 stderr 收进共享字符串（只保留末尾，便于报错）。
pub fn spawn(cmd: &mut Command) -> Result<(Child, Arc<Mutex<String>>), String> {
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 FFmpeg：{e}"))?;
    let err = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let sink = err.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Ok(mut s) = sink.lock() {
                    s.push_str(&line);
                    s.push('\n');
                    let len = s.len();
                    if len > 4000 {
                        let cut = len - 4000;
                        s.drain(..cut);
                    }
                }
            }
        });
    }
    Ok((child, err))
}

fn parse_rate(v: &str) -> Option<f64> {
    if let Some((a, b)) = v.split_once('/') {
        let (n, d) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
        if d > 0.0 { Some(n / d) } else { None }
    } else {
        v.parse::<f64>().ok()
    }
}

fn json_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

fn json_f64(v: &serde_json::Value, key: &str) -> Option<f64> {
    let x = v.get(key)?;
    x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse::<f64>().ok()))
}

/// 用 ffprobe 读取视频信息；缺 ffprobe 时退化为解析 `ffmpeg -i` 的输出。
pub fn probe(tools: &Tools, path: &Path) -> Result<VideoInfo, String> {
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if let Some(fp) = &tools.ffprobe {
        let out = Command::new(fp)
            .args([
                "-v", "error", "-select_streams", "v:0",
                "-show_entries", "stream=width,height,r_frame_rate,avg_frame_rate,nb_frames,codec_name",
                "-show_entries", "format=duration,size",
                "-of", "json",
            ])
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| format!("ffprobe 启动失败：{e}"))?;
        if out.status.success() {
            let v: serde_json::Value = serde_json::from_slice(&out.stdout)
                .map_err(|e| format!("ffprobe 输出解析失败：{e}"))?;
            let st = v.get("streams").and_then(|x| x.get(0)).cloned().unwrap_or_default();
            let fm = v.get("format").cloned().unwrap_or_default();
            let w = json_f64(&st, "width").unwrap_or(0.0) as u32;
            let h = json_f64(&st, "height").unwrap_or(0.0) as u32;
            if w > 0 && h > 0 {
                let fps = json_str(&st, "avg_frame_rate")
                    .and_then(parse_rate)
                    .or_else(|| json_str(&st, "r_frame_rate").and_then(parse_rate))
                    .unwrap_or(25.0);
                let duration = json_f64(&fm, "duration")
                    .or_else(|| json_str(&fm, "duration").and_then(|s| s.parse().ok()))
                    .unwrap_or(0.0);
                let frames = json_str(&st, "nb_frames").and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or_else(|| (duration.max(0.0) * fps) as u64);
                let codec = json_str(&st, "codec_name").unwrap_or("未知").to_string();
                let size = json_f64(&fm, "size").unwrap_or(bytes as f64) as u64;
                return Ok(VideoInfo { path: path.into(), w, h, fps: fps.clamp(1.0, 240.0), duration: duration.max(0.01), frames, codec, bytes: size });
            }
        }
    }
    // 兜底：从 ffmpeg -i 的 stderr 里抓分辨率与时长
    let out = Command::new(&tools.ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(path)
        .output()
        .map_err(|e| format!("FFmpeg 启动失败：{e}"))?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut w = 0u32;
    let mut h = 0u32;
    let mut fps = 25.0f64;
    let mut duration = 0.0f64;
    let mut codec = String::from("视频");
    for line in text.lines() {
        if line.contains("Video:") {
            // 形如 `Stream #0:0: Video: h264, yuv420p, 1920x1080, 30 fps`
            if let Some(p) = line.find("Video:") {
                let name = line[p + 6..].trim_start();
                let name = name.split([',', ' ', '(']).next().unwrap_or("").trim();
                if !name.is_empty() { codec = name.to_string(); }
            }
            for part in line.split([',', ' ']) {
                if let Some((a, b)) = part.split_once('x') {
                    if let (Ok(ww), Ok(hh)) = (a.parse::<u32>(), b.parse::<u32>()) {
                        if ww > 16 && hh > 16 { w = ww; h = hh; }
                    }
                }
                if part.trim().ends_with("fps") {
                    if let Ok(f) = part.trim().trim_end_matches("fps").trim().parse::<f64>() { fps = f; }
                }
            }
        }
        // `Duration: 00:00:12.34, start: ...` —— 必须存到循环外，流信息在其后一行。
        if let Some(pos) = line.find("Duration:") {
            let d = &line[pos + 9..];
            let t: Vec<&str> = d.trim().split(':').collect();
            if t.len() == 3 {
                let hh = t[0].trim().parse::<f64>().unwrap_or(0.0);
                let mm = t[1].trim().parse::<f64>().unwrap_or(0.0);
                let ss = t[2].split(',').next().unwrap_or("0").trim().parse::<f64>().unwrap_or(0.0);
                duration = hh * 3600.0 + mm * 60.0 + ss;
            }
        }
    }
    if w > 0 && h > 0 {
        let duration = duration.max(0.01);
        return Ok(VideoInfo {
            path: path.into(), w, h, fps: fps.clamp(1.0, 240.0), duration,
            frames: (duration * fps) as u64, codec, bytes,
        });
    }
    Err("无法解析视频信息，确认文件可播放且 FFmpeg 可用".into())
}

/// 构造解码命令：输出固定宽高、固定帧率的 RGBA 裸帧到 stdout。
pub fn decode_cmd(
    tools: &Tools,
    path: &Path,
    w: u32,
    h: u32,
    fps: f64,
    start: f64,
    duration: f64,
    single: bool,
    fit: FitMode,
) -> Command {
    let mut c = Command::new(&tools.ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error"]);
    if start > 0.001 {
        c.args(["-ss", &format!("{start:.3}")]);
    }
    c.arg("-i").arg(path);
    if !single && duration > 0.0 {
        c.args(["-t", &format!("{duration:.3}")]);
    }
    // 适配在解码阶段完成：留边用透明 pad、填充用居中 crop，输出始终是固定 W×H。
    let base = match fit {
        FitMode::Stretch => format!("scale={w}:{h}:flags=bilinear"),
        FitMode::Contain => format!(
            "scale={w}:{h}:flags=bilinear:force_original_aspect_ratio=decrease,format=rgba,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=0x00000000"
        ),
        FitMode::Cover => format!(
            "scale={w}:{h}:flags=bilinear:force_original_aspect_ratio=increase,crop={w}:{h}"
        ),
    };
    let filter = if single { base } else { format!("{base},fps={fps:.4}") };
    c.args(["-an", "-sn", "-vf", &filter]);
    if single {
        c.args(["-frames:v", "1"]);
    }
    c.args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    c.stdout(Stdio::piped());
    c
}

/// 顺序读取固定尺寸的 RGBA 帧；流结束或出错时返回 None。
pub struct FrameReader {
    pub child: Child,
    stdout: ChildStdout,
    pub w: u32,
    pub h: u32,
    stderr: Arc<Mutex<String>>,
}

impl FrameReader {
    pub fn open(tools: &Tools, path: &Path, w: u32, h: u32, fps: f64, start: f64, duration: f64, single: bool, fit: FitMode) -> Result<Self, String> {
        let mut cmd = decode_cmd(tools, path, w, h, fps, start, duration, single, fit);
        let (mut child, stderr) = spawn(&mut cmd)?;
        let stdout = child.stdout.take().ok_or("解码器没有 stdout")?;
        Ok(Self { child, stdout, w, h, stderr })
    }
    pub fn next(&mut self) -> Option<Vec<u8>> {
        let n = (self.w as usize) * (self.h as usize) * 4;
        let mut buf = vec![0u8; n];
        match self.stdout.read_exact(&mut buf) {
            Ok(()) => Some(buf),
            Err(_) => None,
        }
    }
    pub fn error_text(&self) -> String {
        self.stderr.lock().map(|s| s.trim().to_string()).unwrap_or_default()
    }
    pub fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 编码器 ffmpeg 参数（输入固定为 `rawvideo rgba pipe:0`）。
pub fn encode_cmd(tools: &Tools, out: &Path, spec: &OutputSpec, s: &VideoSettings, fmt: OutFormat) -> Command {
    let mut c = Command::new(&tools.ffmpeg);
    c.args([
        "-hide_banner", "-loglevel", "error", "-y",
        "-f", "rawvideo", "-pix_fmt", "rgba",
        "-s", &format!("{}x{}", spec.w, spec.h),
        "-r", &format!("{:.4}", spec.fps),
        "-i", "pipe:0", "-an",
    ]);
    let crf = s.crf.clamp(0, 63).to_string();
    match fmt {
        OutFormat::WebmAlpha => {
            c.args(["-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-auto-alt-ref", "0",
                    "-b:v", "0", "-crf", &crf, "-row-mt", "1"]);
        }
        OutFormat::MovProres => {
            c.args(["-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuva444p10le"]);
        }
        OutFormat::MovQtrle => {
            c.args(["-c:v", "qtrle", "-pix_fmt", "argb"]);
        }
        OutFormat::Apng => {
            let plays = if s.loop_count == 0 { "0".to_string() } else { s.loop_count.to_string() };
            c.args(["-f", "apng", "-plays", &plays, "-c:v", "png", "-pix_fmt", "rgba"]);
        }
        OutFormat::Mp4Flat => {
            c.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", &crf,
                    "-preset", "medium", "-movflags", "+faststart"]);
        }
        OutFormat::Gif | OutFormat::PngSeq => {}
    }
    c.arg(out);
    c.stdin(Stdio::piped());
    c.stdout(Stdio::null());
    c
}
