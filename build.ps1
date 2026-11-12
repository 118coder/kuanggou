# 构建发行包：dist\kuanggou.exe（自动更新下载的就是这个文件名）
$ErrorActionPreference = "Stop"
cargo build --release
New-Item -ItemType Directory -Force -Path dist | Out-Null
Copy-Item target\release\cutout_tool.exe dist\kuanggou.exe -Force
Copy-Item target\release\cutout_tool.exe 框构.exe -Force
Write-Host "OK -> dist\kuanggou.exe / 框构.exe"
