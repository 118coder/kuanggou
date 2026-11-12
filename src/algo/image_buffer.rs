//! RGBA8 图像容器：解码、原始缓冲构造、预览降采样。
//!
//! 全工程统一约定：缓冲区为 RGBA8、每像素 4 字节、行优先。


/// RGBA8 图像缓冲（含大图预览代理工具）
#[derive(Clone)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    pub buf: Vec<u8>,
}

impl Img {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| format!("图片解码失败: {e}"))?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(Self { w: w as usize, h: h as usize, buf: rgba.into_raw() })
    }

    pub fn from_raw(w: usize, h: usize, buf: Vec<u8>) -> Self {
        Self { w, h, buf }
    }

    pub fn megapixels(&self) -> f32 {
        (self.w * self.h) as f32 / 1_000_000.0
    }

    /// 大图降采样代理（长边 ≤2048，仅用于预览；导出仍走全分辨率）
    pub fn downscale_proxy(&self) -> Img {
        self.downscale_to_long(2048)
    }

    /// 按长边降采样（交互预览用；长边已小于目标则原样克隆）
    pub fn downscale_to_long(&self, long: u32) -> Img {
        let (w, h) = (self.w as u32, self.h as u32);
        if w.max(h) <= long {
            return self.clone();
        }
        let s = long as f32 / w.max(h) as f32;
        let nw = ((w as f32 * s) as u32).max(1);
        let nh = ((h as f32 * s) as u32).max(1);
        let full = image::RgbaImage::from_raw(w, h, self.buf.clone()).expect("缓冲区尺寸正确");
        let resized = image::imageops::resize(&full, nw, nh, image::imageops::FilterType::Triangle);
        Img::from_raw(nw as usize, nh as usize, resized.into_raw())
    }
}
