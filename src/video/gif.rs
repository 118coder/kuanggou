//! GIF 编码：把处理好的 RGBA 帧序列化成动图字节流。
//!
//! GIF 只支持 1-bit 透明（alpha=0 才透），因此编码前必须做二值化；
//! 这里用 Bayer 有序抖动保留半透明观感，并用预量化减少颜色数以提高 LZW 压缩率。
//! 编码目标固定为 `Vec<u8>`，方便「先试编码再决定写不写盘」的体积上限逻辑。

use crate::video::pipeline::{gif_alpha_binary, quantize_colors};
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 带写入字节计数的 `Write` 适配器。
///
/// GIF 编码器持有输出流的独占借用，无法从外部读 `Vec::len()`；
/// 这里用原子计数把「已写字节」暴露出来，供体积上限逻辑实时判断。
struct Counting<W: Write> {
    inner: W,
    bytes: Arc<AtomicU64>,
}

impl<W: Write> Counting<W> {
    fn new(inner: W) -> Self {
        Self { inner, bytes: Arc::new(AtomicU64::new(0)) }
    }
    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// GIF 帧预处理参数。
#[derive(Clone, Copy, Debug)]
pub struct GifPrep {
    pub w: u32,
    pub h: u32,
    pub colors: u16,
    pub threshold: u8,
    pub dither: bool,
}

/// 颜色数 → NeuQuant 速度（越大越快越粗糙）。
pub fn speed_for(colors: u16) -> i32 {
    match colors {
        0..=32 => 12,
        33..=64 => 8,
        65..=128 => 4,
        _ => 1,
    }
}

/// 一帧写入 GIF 之前的统一预处理：二值透明 → 预量化。
pub fn prep_frame(buf: &mut [u8], p: GifPrep) {
    gif_alpha_binary(buf, p.w, p.h, p.threshold, p.dither);
    quantize_colors(buf, p.colors);
}

/// 编码一段帧序列。
///
/// `next` 是帧供给闭包：`Some((rgba, 帧延时毫秒))` / `None` 表示结束。
/// 闭包形式让「体积超标后重新解码重编码」不需要把全部帧缓存进内存。
/// `budget` 给定时，若中途体积已明显超出按进度分配的预算，提前中止。
pub fn encode_gif<F>(
    w: u32,
    h: u32,
    colors: u16,
    loop_count: u16,
    budget: Option<u64>,
    total_frames: u64,
    mut next: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut() -> Option<(Vec<u8>, u32)>,
{
    let bytes = Arc::new(AtomicU64::new(0));
    let mut sink = Counting { inner: Vec::with_capacity(64 * 1024), bytes: Arc::clone(&bytes) };
    {
        let mut enc = GifEncoder::new_with_speed(&mut sink, speed_for(colors));
        let repeat = if loop_count == 0 { Repeat::Infinite } else { Repeat::Finite(loop_count) };
        enc.set_repeat(repeat).map_err(|e| format!("GIF 循环设置失败：{e}"))?;
        let mut idx = 0u64;
        while let Some((rgba, delay_ms)) = next() {
            let img = RgbaImage::from_raw(w, h, rgba).ok_or("GIF 帧尺寸与缓冲区不符")?;
            let delay = Delay::from_numer_denom_ms(delay_ms.max(10), 1);
            let frame = Frame::from_parts(img, 0, 0, delay);
            enc.encode_frame(frame).map_err(|e| format!("GIF 编码失败：{e}"))?;
            idx += 1;
            if let Some(b) = budget {
                // 按进度线性分配预算，留 45% 余量（GIF 后段帧通常更小）。
                let allow = (b as f64 * (idx as f64 / total_frames.max(1) as f64) * 1.45
                    + 64.0 * 1024.0) as u64;
                let written = bytes.load(Ordering::Relaxed);
                if written > allow {
                    return Err(format!("体积超出预算（已写 {written} 字节 / 上限 {b}）"));
                }
            }
        }
    } // drop encoder → gif crate 自动写 trailer
    let out = sink.into_inner();
    if out.len() < 32 {
        return Err("没有可编码的帧".into());
    }
    Ok(out)
}
