//! 把图标和版本信息写进 exe 的 Windows 资源（Explorer 里显示的就是这个图标）。
//! 用 mingw 的 windres 生成 COFF 资源对象，再作为链接参数塞进最终 exe。

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let obj = format!("{out}/app-res.o");
    let candidates = ["windres", "x86_64-w64-mingw32-windres"];
    for tool in candidates {
        let ok = Command::new(tool)
            .args([
                "--codepage=936",
                "-I",
                "assets",
                "-O",
                "coff",
                "assets/app.rc",
                "-o",
                &obj,
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("cargo:rustc-link-arg={obj}");
            return;
        }
    }
    println!("cargo:warning=找不到 windres，exe 图标与版本资源未写入（窗口图标不受影响）");
}
