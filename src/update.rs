//! 自动检查更新（走 GitHub Releases API）
//!
//! 流程：查最新 release → 比版本号 → 用户点「下载并安装」→ 下载 kuanggou.exe →
//! 写一个批处理，等本进程退出后备份旧 exe、覆盖、重启。更新永远不强制。

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const REPO: &str = "118coder/kuanggou";
const ASSET: &str = "kuanggou.exe";

#[derive(Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
    pub notes: String,
    pub size: u64,
}

pub enum Msg {
    NoUpdate(String),
    Found(UpdateInfo),
    Failed(String),
    Ready(PathBuf),
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("kuanggou/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn parse_ver(s: &str) -> (u32, u32, u32) {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let mut it = s.split('.').map(|x| x.trim().parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn is_newer(remote: &str) -> bool {
    parse_ver(remote) > parse_ver(current_version())
}

/// 后台查最新版本（不阻塞 UI）
pub fn check(tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let r = (|| -> Result<Option<UpdateInfo>, String> {
            let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
            let resp = agent()
                .get(&url)
                .set("Accept", "application/vnd.github+json")
                .call()
                .map_err(|e| format!("{e}"))?;
            let v: serde_json::Value = resp.into_json().map_err(|e| format!("{e}"))?;
            let tag = v["tag_name"].as_str().unwrap_or("");
            if tag.is_empty() {
                let msg = v["message"].as_str().unwrap_or("接口返回异常");
                return Err(msg.to_string());
            }
            if !is_newer(tag) {
                return Ok(None);
            }
            let empty = Vec::new();
            let assets = v["assets"].as_array().unwrap_or(&empty);
            let pick = assets
                .iter()
                .find(|a| a["name"].as_str() == Some(ASSET))
                .or_else(|| {
                    assets.iter().find(|a| {
                        a["name"]
                            .as_str()
                            .map(|n| n.to_ascii_lowercase().ends_with(".exe"))
                            .unwrap_or(false)
                    })
                });
            let Some(a) = pick else { return Ok(None) };
            let url = a["browser_download_url"].as_str().unwrap_or("").to_string();
            if url.is_empty() {
                return Ok(None);
            }
            Ok(Some(UpdateInfo {
                version: tag.trim_start_matches(['v', 'V']).to_string(),
                url,
                notes: v["body"].as_str().unwrap_or("").to_string(),
                size: a["size"].as_u64().unwrap_or(0),
            }))
        })();
        match r {
            Ok(Some(i)) => {
                let _ = tx.send(Msg::Found(i));
            }
            Ok(None) => {
                let _ = tx.send(Msg::NoUpdate(current_version().to_string()));
            }
            Err(e) => {
                let _ = tx.send(Msg::Failed(e));
            }
        }
    });
}

/// 后台下载新版 exe 到临时目录
pub fn download(info: UpdateInfo, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let r = (|| -> Result<PathBuf, String> {
            let dir = std::env::temp_dir().join("kuanggou_update");
            std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
            let path = dir.join("kuanggou.new.exe");
            let resp = agent().get(&info.url).call().map_err(|e| format!("下载失败：{e}"))?;
            let mut f = std::fs::File::create(&path).map_err(|e| format!("写文件失败：{e}"))?;
            std::io::copy(&mut resp.into_reader(), &mut f).map_err(|e| format!("下载中断：{e}"))?;
            f.sync_all().ok();
            drop(f);
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if len < 1_000_000 {
                return Err(format!("下载不完整（{len} 字节）"));
            }
            if info.size > 0 && len != info.size {
                return Err(format!("大小不符：期望 {}，实际 {len}", info.size));
            }
            let mut head = [0u8; 2];
            std::fs::File::open(&path)
                .and_then(|mut h| h.read_exact(&mut head))
                .map_err(|e| format!("校验失败：{e}"))?;
            if &head != b"MZ" {
                return Err("下载到的不是 Windows 程序".into());
            }
            Ok(path)
        })();
        match r {
            Ok(p) => {
                let _ = tx.send(Msg::Ready(p));
            }
            Err(e) => {
                let _ = tx.send(Msg::Failed(e));
            }
        }
    });
}

/// 等本进程退出后替换 exe 并重启（旧版本留一份 .old，覆盖失败可回滚）
pub fn apply(new_exe: &std::path::Path) -> Result<(), String> {
    let cur = std::env::current_exe().map_err(|e| format!("取当前程序路径失败：{e}"))?;
    let dir = cur.parent().ok_or("取程序目录失败")?.to_path_buf();
    let bat = std::env::temp_dir().join("kuanggou_update.bat");
    let body = format!(
        "@echo off\r\n\
         cd /d \"{dir}\"\r\n\
         timeout /t 3 /nobreak >nul\r\n\
         move /y \"{cur}\" \"{cur}.old\" >nul 2>&1\r\n\
         move /y \"{new}\" \"{cur}\" >nul 2>&1\r\n\
         if exist \"{cur}\" (start \"\" \"{cur}\") else (move /y \"{cur}.old\" \"{cur}\" >nul 2>&1 & start \"\" \"{cur}\")\r\n\
         del \"{cur}.old\" >nul 2>&1\r\n\
         del \"%~f0\" >nul 2>&1\r\n",
        dir = dir.display(),
        cur = cur.display(),
        new = new_exe.display()
    );
    std::fs::write(&bat, body).map_err(|e| format!("写更新脚本失败：{e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(bat.as_os_str());
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn().map_err(|e| format!("启动更新脚本失败：{e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = bat;
        Err("仅支持 Windows 自动替换".into())
    }
}
