# Ruiss — 双机键鼠共享

两台电脑共用一套鼠标键盘：鼠标滑到屏幕边缘就"滑"到另一台电脑，键盘跟着走，剪贴板（文本 / 图片 / 文件）自动同步，还能跨屏直接拖拽文件。

支持 **Windows + macOS** 混用（Win↔Mac、Win↔Win、Mac↔Mac）。系统托盘常驻：Windows 托盘图标 / Mac 菜单栏图标，右键可打开设置窗口或退出。

## 功能特性

- 🖱️ **鼠标跨屏**：光标滑到屏幕边缘停留片刻即切到对端；再滑回即返回，支持反向跨屏
- ⌨️ **键盘共享**：一套键盘控制两台电脑，自动映射 Windows / Mac 键位差异（Ctrl↔Command、Win↔Command）
- 📋 **剪贴板同步**：文本、图片、文件路径自动跨机同步，防回环设计
- 📁 **文件传输**：分块可靠传输，接收端自动写入下载目录（重名自动改名），完成后路径写入剪贴板
- 🧲 **跨屏拖拽**：从本机把文件/图片拖到对端，对端光标处直接粘贴落盘
- 🖥️ **设置窗口**：Tauri 2 界面，可配置网络端口、键位映射、开机自启等

## 技术栈

| 部分 | 技术 |
|------|------|
| 核心逻辑 | Rust（事件捕获/注入、网络、剪贴板、文件传输） |
| 界面/托盘 | Tauri 2（前端 HTML/JS/CSS） |
| Windows 事件 | `SetWindowsHookEx` 捕获 + `SendInput` 注入 |
| macOS 事件 | `CGEventTap` 捕获 + `CGEventPost` 注入 |
| 剪贴板 | `arboard` |
| 网络 | `tokio` + 自定义二进制协议（TCP：按键/剪贴板/文件，UDP：高频鼠标移动） |

## 快速开始

需要 Rust 工具链（[rustup.rs](https://rustup.rs)）和 Node.js（仅打包时需要）。

```bash
# 开发运行（弹出托盘图标 + 设置窗口）
cargo run --manifest-path src-tauri/Cargo.toml

# 编译检查
cargo check --manifest-path src-tauri/Cargo.toml

# 打包 Windows 安装程序（NSIS exe）
npx tauri build --bundles nsis
```

> 详细的平台环境安装、双机联调、自测/双实例测试、常见问题见 **[BUILD.md](BUILD.md)**。

## 使用方式

1. 两台电脑各装一份 Ruiss，并处于同一局域网
2. 在设置窗口中填写对端 IP 与端口（默认端口见设置界面）
3. 连接成功后，把鼠标滑到屏幕边缘即跨屏

macOS 首次使用需要在「系统设置 → 隐私与安全性 → 辅助功能」中授权。

## 工作原理

采用**动态主控仲裁**，不设固定主控/被控端：谁的鼠标在动，谁就是 Source（控制方），另一端作为 Sink（被控方）注入事件。

- 跨屏判定：本机鼠标超过屏幕边缘并停留 100–200ms（防误触）→ 通知对端从另一侧注入光标进入
- 剪贴板防回环：消息带标记位，收到自己发出的内容不再转发
- 光标隐藏：Windows 用 `SetSystemCursor` 替换系统光标为透明图；macOS 用 `CGDisplayHideCursor` + 私有 CGS API 后台权限（Synergy 同款方案），保证跨屏时本机不出现双鼠标

## 项目结构

```
ruiss/
├── src-tauri/           # Rust 侧全部代码（Tauri 2 标准布局）
│   └── src/
│       ├── core/        # 共享逻辑：协议、坐标换算、键位映射、跨屏仲裁
│       ├── platform/    # 平台相关：win.rs / mac.rs（事件捕获与注入）
│       ├── net/         # TCP + UDP 通道
│       ├── clipboard/   # 剪贴板同步（监听 + 防回环）
│       └── file_transfer/ # 文件分块传输 + 跨屏拖拽
├── gui/                 # 前端：设置窗口
└── scripts/             # 工具脚本（图标生成等）
```

## 开发路线

- [x] M1 单机自测：事件捕获→注入链路
- [x] M2 双机打通：网络层 + 跨屏判定 + 键位映射
- [~] M3 剪贴板 + 文件传输 + 跨屏拖拽 + 托盘 UI（进行中）
- [ ] M4 打磨：DPI 适配、鼠标平滑、延迟优化、断线重连

## 协议

本项目以 **MIT 协议** 开源，详见 [LICENSE](LICENSE)。

## 相关文档

- [BUILD.md](BUILD.md) — 环境安装、编译、双机联调指南
- [CHANGELOG.md](CHANGELOG.md) — 变更记录
- [AGENTS.md](AGENTS.md) — 协作规范（面向 AI 辅助开发）
