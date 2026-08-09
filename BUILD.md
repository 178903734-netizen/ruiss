# Ruiss 编译与测试文档

本文档覆盖：环境安装、Windows / Mac 编译、运行配置、自测模式、
单机双实例测试、双机联调、环境变量速查、常见问题排查。

> 当前进度：M0 骨架 ✅ / M1 本机自测 ✅ / M2 双机打通（代码完成，待双机实测）。
> Mac 端平台代码已写，但只在 Mac 上编译过才算验证。

---

## 一、项目结构（编译相关）

```
ruiss/
├── src-tauri/                # Rust 侧全部代码（Tauri 2）
│   ├── Cargo.toml            # 依赖（Windows/Mac 平台依赖按 target 分开）
│   ├── tauri.conf.json       # 窗口、托盘、打包配置
│   ├── .cargo/config.toml    # target 目录、crates.io 镜像（本机 Windows 用）
│   ├── examples/input_probe.rs  # 输入探针（开发调试工具）
│   └── src/                  # lib.rs + core / platform / net / clipboard
├── gui/                      # 前端（纯 HTML/JS，无需 npm 构建）
├── scripts/set-ruiss-env.ps1 # Windows 环境变量持久化脚本
└── PROJECT.md                # 项目规划与里程碑
```

前端无任何构建步骤（无 npm 依赖），`tauri.conf.json` 直接指向 `gui/` 目录。

---

## 二、Windows 编译

### 2.1 环境要求（本机已装好，新机器按此装）

| 组件 | 说明 |
|---|---|
| Rust 工具链 | 装在 **D 盘**（C 盘空间不足）：`RUSTUP_HOME=D:\rustup`、`CARGO_HOME=D:\cargo` |
| 工具链 target | `stable-x86_64-pc-windows-gnu`（GNU 版，配 w64devkit 的 gcc 链接器） |
| 链接器 | `D:\w64devkit\w64devkit\bin`（gcc/ld） |
| 编译产物目录 | `CARGO_TARGET_DIR=D:\ruiss-target`（环境变量，见下；Mac 不设置） |
| crates.io 镜像 | `src-tauri/.cargo/config.toml` 里已配 rsproxy 国内镜像（Win/Mac 共用，直连慢） |

环境变量持久化（新机器装好后执行一次）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/set-ruiss-env.ps1
# 然后重开终端生效
```

> `.cargo/config.toml` 是 Win/Mac **共用**文件（只含镜像等通用配置），
> Windows 的 target 目录用环境变量 `CARGO_TARGET_DIR` 控制，**不要**写进该文件
> （Windows 的 `D:/` 路径在 Mac 上会导致 DYLD 报错）。

### 2.2 编译命令

```bash
# 编译（debug，首次 10~20 分钟，之后增量几秒~几十秒）
# 必须固定用这一条命令 + 固定 target 目录，才能命中增量缓存（见下方铁律）
cd "C:/Users/23764/Desktop/ruiss"
CARGO_TARGET_DIR="D:/ruiss-target" cargo build --manifest-path src-tauri/Cargo.toml

# 带调试日志运行
RUST_LOG=ruiss_lib=debug cargo run
```

> ⚠️ 增量缓存铁律（2026-08-08 踩坑后固化，以后别再纠结"为什么编译慢"）：
>
> - 永远只用上面这一条 build 命令，不要换写法、不要混用默认 target 目录；
> - **不删 `D:\ruiss-target`、不做 `cargo clean`**——删了/清了 = 全量重编所有依赖（20 分钟+）；
> - 跑任何 cargo 命令（包括 cargo check）都要带 `CARGO_TARGET_DIR="D:/ruiss-target"`，
>   否则会在 `src-tauri\target` 下另起炉灶，把全部依赖再全量编译一遍；
> - 正常改代码后重跑 build，只重编 ruiss 自己，一分钟内完成；
> - cargo 的增量指纹很敏感：命令写法、环境变量、cargo 版本变化都可能让缓存失效 → 全量重编。

### 2.3 产物位置

```
D:\ruiss-target\debug\ruiss.exe          # 主程序（debug 约 245MB）
D:\ruiss-target\debug\WebView2Loader.dll # 运行必需，必须和 exe 放同目录！
```

拷到别的电脑测试时**两个文件都要拷**，放同一个文件夹。

### 2.4 常见编译报错

| 报错 | 原因与解决 |
|---|---|
| `error: failed to remove file ... ruiss.exe` (拒绝访问) | 程序正在运行，先关掉（任务管理器或托盘退出）再编译 |
| `export ordinal too large` | 老问题已修复（Cargo.toml 去掉 cdylib）。如再出现检查 crate-type |
| 下载依赖超时 | 检查 `.cargo/config.toml` 的 rsproxy 镜像配置是否还在 |

### 2.5 关于单元测试（cargo test）

**已知问题：`cargo test` 的测试二进制在本机 GNU 工具链下启动失败**
（`0xc0000139` 入口点找不到，与 WebView2Loader.dll 的加载环境有关），
**不影响主程序运行**。功能验证请走下文的自测模式 / 单机双实例 / 探针，
不要依赖 `cargo test`。

---

## 三、Mac 编译

### 3.1 环境安装（终端执行）

```bash
# 1. Xcode 命令行工具（弹窗点"安装"）
xcode-select --install

# 2. Rust（按提示选默认即可）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# 安装后让配置生效
source "$HOME/.cargo/env"
```

> Mac 上不要拷贝 Windows 的 `.cargo/config.toml`（里面的 target-dir 和镜像
> 是 Windows 专用的；Mac 用默认配置即可，依赖会下到 `~/.cargo`）。

### 3.2 编译

```bash
cd ruiss/src-tauri
cargo build          # 首次下依赖 + 编译，10~20 分钟
cargo run            # 运行（弹出托盘图标）
```

### 3.3 辅助功能权限（必须）

Mac 的全局事件捕获需要授权：

1. 首次 `cargo run` 后，日志会提示"未授予辅助功能权限"
2. 打开 **系统设置 → 隐私与安全性 → 辅助功能**
3. 勾选本应用（终端里跑就是勾选"终端"，打了包就是勾选 app 名）
4. 重新运行，日志出现 `CGEventTap 已启动` 即捕获生效

> 未授权时软件照常启动，但收不到任何键鼠事件，自测/跨屏全都不生效。

### 3.4 编译报错处理

Mac 端代码（`src/platform/mac.rs`）依赖 core-graphics 0.24 的 API，
如编译报错（个别 API 名/签名差异），**把完整错误信息发给开发者**，
修改后重新编译。

---

## 四、运行与配置

托盘图标右键菜单：**显示设置窗口** / **退出**。

设置窗口字段：

| 字段 | 说明 |
|---|---|
| 本机名字 | 消息来源标识（两端要不同） |
| 对方 IP | 对端电脑的局域网 IP（`ipconfig` / `ifconfig` 查） |
| 屏幕布局 | 对方在我哪边：右边 / 左边（两端互为镜像：A 选右边，B 必须选左边） |
| 剪贴板共享 / 开机自启 | M3 功能，暂未生效 |

保存后底部"网络"栏会显示：`已连接 / 未连接（重连中）`、`主控 / 被控`、`跨屏中`、
收发计数。

网络端口（默认，防火墙需放行）：

```
TCP 5200  — 按键/点击/滚轮/控制消息（可靠）
UDP 5300  — 鼠标移动（高频，可丢帧）
```

---

## 五、M1 自测模式（单机验证捕获→注入链路）

开发验证工具。设置窗口 → **M1 自测** 勾选开启：

- 统计数字随操作跳动 = 捕获链路通
- 打字/点击松开后出现"回声"（双字符/双击）= 注入链路通
- **测完务必关闭**（回环期间操作会翻倍）

命令行方式（启动即开启）：

```bash
RUISS_SELF_TEST=1 cargo run
```

---

## 六、单机双实例测试（验证网络 + 令牌 + 注入链路）

没有第二台电脑时，在一台机器上跑两个实例互相对接，除"光标出现在另一台屏幕"
的视觉效果外，网络层、跨屏判定、令牌交换、注入链路全部真实执行。

### Windows

```bash
# 终端 1：实例 A（默认端口，配置存默认位置）
cargo run

# 终端 2：实例 B（端口 +1，独立配置文件，避免两个实例互相覆盖设置）
RUISS_TCP_PORT=5201 RUISS_UDP_PORT=5301 RUISS_CONFIG_FILE=D:\ruiss-b.json cargo run
```

### Mac

```bash
# 终端 1
cargo run
# 终端 2
RUISS_TCP_PORT=5201 RUISS_UDP_PORT=5301 RUISS_CONFIG_FILE=~/ruiss-b.json cargo run
```

### 两边设置

| 实例 | 本机名字 | 对方 IP | 布局 |
|---|---|---|---|
| A | A | `127.0.0.1` | 右边 |
| B | B | `127.0.0.1` | 左边 |

保存后两边都应显示"网络：已连接"。

### 验证步骤（在 A 的操作）

1. A 光标移到**右边缘**停住 ~0.2 秒
2. A 日志出现 `发起跨屏 → TakeControl(...)`，光标回绕到左边缘
3. B 日志出现 `对端接管 → 注入入口 (...)`
4. 移动 A 鼠标 → A 日志转发（MouseMove）、B 日志注入；A 打字/点击 → B 响应
5. A 光标移回右边缘停住 → `ReleaseControl`，控制归还
6. B 光标移到左边缘停住 → B 夺回（`TakeControl` 反向）

> 单机双实例时两台"共用"一块屏幕一个鼠标，光标位置镜像、看不到另一台的效果，
> 属正常；链路日志全对即通过。

---

## 七、双机联调（最终验证）

1. 两台电脑连同一局域网（同路由器 / 手机热点 / 网线直连）
2. 各跑一个实例，互相填对方 IP；布局互为镜像（A 右 → B 左）
3. 防火墙放行 TCP 5200 / UDP 5300（Windows 首次保存会弹授权，点允许；
   或 控制面板 → Windows Defender 防火墙 → 高级设置 → 入站规则 手动放行）
4. 状态栏两边都"已连接"后，按第六节步骤 1~6 操作

已知限制（M2）：

- 分辨率不一致时坐标**等比映射**（小屏滑大屏可覆盖全屏，但会整体拉伸；
  缩放比例/多显示器适配是 M4）
- 双方同时抢控可能僵住 → 任一方在出口边停留即可夺回
- 被控期间对端持续注入会与本机真实鼠标"打架"

---

## 八、环境变量速查

| 变量 | 作用 |
|---|---|
| `RUST_LOG=ruiss_lib=debug` | 详细日志（捕获/注入/连接/令牌全打印），排障用 |
| `RUISS_SELF_TEST=1` | 启动即开启 M1 自测回环 |
| `RUISS_NO_SUPPRESS=1` | 关闭注入事件过滤（仅自动化验证用，正常**不要**开） |
| `RUISS_TCP_PORT` / `RUISS_UDP_PORT` | 覆盖端口（单机双实例必须错开） |
| `RUISS_CONFIG_FILE` | 覆盖设置文件路径（单机双实例必须分开） |

---

## 九、常见问题排查

| 现象 | 排查 |
|---|---|
| 显示"未连接（重连中）" | 先 `ping 对方IP`；检查防火墙 5200/5300；确认两端布局镜像 |
| 连上了但跨屏没反应 | 用 `RUST_LOG=ruiss_lib=debug` 跑，看有没有 `TakeControl`/`注入入口` 日志 |
| Mac 上完全没反应 | 辅助功能权限没授权（见 3.3）；日志确认 `CGEventTap 已启动` |
| 设置被互相覆盖 | 双实例必须用 `RUISS_CONFIG_FILE` 分开 |
| 光标"被拖拽" | M1 自测开着？关掉自测开关 |
| `cargo test` 崩溃 0xc0000139 | 工具链已知问题，不影响功能，用自测模式/双实例验证 |
| 编译报 `拒绝访问` | 先关掉正在运行的 ruiss.exe |

---

## 十、开发调试工具

```bash
# 输入探针：独立验证注入/捕获链路（Windows）
cargo run --manifest-path src-tauri/Cargo.toml --example input_probe -- full   # 注入按键/点击/滚轮并校验
cargo run --manifest-path src-tauri/Cargo.toml --example input_probe -- move   # 注入一次绝对移动（配合 RUISS_NO_SUPPRESS 验证捕获）
cargo run --manifest-path src-tauri/Cargo.toml --example input_probe -- hook   # 装 LL 钩子看注入事件是否可见
```

> 探针会往当前聚焦窗口注入按键/在光标处点击，跑 `full` 前先切到安全窗口（如记事本）。
