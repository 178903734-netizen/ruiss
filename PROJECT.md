# Ruiss — 双机键鼠共享

一句话：两台电脑共用一套鼠标键盘，鼠标滑到屏幕边缘就"滑"到另一台；剪贴板（文本+图片+文件）自动同步，还能跨屏拖拽图片/文件。
Windows 托盘图标 + Mac 菜单栏图标，右键菜单：显示设置窗口 / 退出。

## 技术栈

- 核心：Rust（事件捕获/注入、网络、剪贴板读写）
- 界面/托盘：Tauri 2（前端 HTML/JS，后续可切 Vue）
- Windows 平台：SendInput / SetWindowsHookEx（windows crate）
- Mac 平台：CGEventPost（core-graphics crate）
- 剪贴板：arboard crate
- 网络：tokio + 自定义二进制协议（TCP 走按键/剪贴板，UDP 走高频鼠标移动）

## 目录结构

```
ruiss/
├── src-tauri/               # Rust 侧全部代码（Tauri 2 标准布局）
│   ├── Cargo.toml
│   ├── tauri.conf.json      # 窗口、托盘、打包配置
│   ├── build.rs
│   ├── capabilities/default.json
│   ├── icons/               # 图标资源（icon.png 等）
│   └── src/
│       ├── main.rs          # 入口：调 lib::run()
│       ├── lib.rs           # Tauri 启动、托盘菜单、设置窗口、Tauri 命令
│       ├── core/            # 共享逻辑：协议定义、坐标换算、键位映射
│       │   ├── mod.rs
│       │   ├── protocol.rs  # 跨机事件消息协议（枚举 + serde）
│       │   ├── geometry.rs  # 屏幕坐标换算（逻辑像素，DPI 友好）
│       │   └── keys.rs      # 键位映射表（Ctrl↔Command、Win↔Command）
│       ├── platform/        # 平台相关：事件捕获与注入
│       │   ├── mod.rs       # cfg 分发（win / mac）
│       │   ├── win.rs       # Windows 实现（SendInput / SetWindowsHookEx）
│       │   └── mac.rs       # macOS 实现（CGEventPost）
│       ├── net/             # TCP + UDP 通道
│       │   ├── mod.rs
│       │   ├── tcp.rs       # 按键、剪贴板（可靠传输）
│       │   └── udp.rs       # 鼠标移动（高频、可丢）
│       ├── clipboard/       # 剪贴板同步（监听变化 + 防回环）
│       │   └── mod.rs
│       └── file_transfer/   # M3：文件分块传输（发送/接收状态机）
│           └── mod.rs
├── gui/                     # 前端：设置窗口（index.html + js + css）
├── docs/
│   └── video-scripts.md     # 三平台视频文案（B站/抖音/YouTube）
├── README.md                # 项目介绍（GitHub 仓库主页）
├── LICENSE                  # 开源协议（MIT）
├── package.json             # 打包工具链（本地 devDep：@tauri-apps/cli，npx tauri build）
├── scripts/
│   └── gen-icon.mjs         # 生成托盘图标 PNG（node 脚本）
├── PROJECT.md               # 本文件：项目地图 + 规划存档
└── CHANGELOG.md
```

## 模块职责

- core/protocol.rs — 定义对端事件消息：MouseMove/MouseButton/KeyDown/KeyUp/Clipboard(Text|Image) 等，serde 序列化。
- core/arbiter.rs — 跨屏仲裁器：Source 侧边缘停留 150ms 触发跨屏（EDGE_MARGIN=2 精确判定）；
  Sink 侧注入光标停在出口边附近 150ms 触发返回（RETURN_MARGIN=30 宽带 + JITTER_TOLERANCE=3 抖动容差，
  触控板惯性微移不重置计时）；记录跨屏出口位置 exit_pos（返回时 warp 回出口边）；Take/Release 间 1s 防抖。
- core/geometry.rs — 坐标换算。统一按"逻辑像素"算（Windows 缩放 125%/150%、Mac Retina 都避开物理像素问题）。
- core/keys.rs — 键位映射：Ctrl ↔ Command（Mac 上 Command 才是 Ctrl 的位）；Win 键 → Mac Command；Shift/Alt 不动。第一版就这三条规则（2026-08-08 补齐导航键与标点符号枚举：Delete/Home/End/PageUp/PageDown/Insert/CapsLock + 逗号句号等）。
- platform/ — 事件捕获与注入，每端约 200 行。Windows: SetWindowsHookEx 捕获 + SendInput 注入；Mac: CGEventTap 捕获 + CGEventPost 注入。
  - 开机自启（2026-08-15）：set_autostart —— Windows 写/删 HKCU\...\CurrentVersion\Run
    键（值名 Ruiss，用 reg.exe，值带引号防空格路径被拆）；Mac 写/删
    ~/Library/LaunchAgents/com.ruiss.app.plist。lib.rs save_settings 变化时应用 +
    启动时对齐。
  - 双击识别（Mac 注入侧，2026-08-08）：注入按下事件必须写 kCGMouseEventClickState
    字段（双击=count 2），否则 macOS 把两次注入点击当两次单击 → Finder 双击打开失效。
    InputInjector 内 ClickState 维护 last_time/last_pos/count（500ms 窗口 + 4px 容差）。
  - 光标隐藏（双鼠标对抗）：Windows 用 SetSystemCursor 把系统光标替换为透明图标
    （内核级，任何窗口 SetCursor 都拿透明图），show_cursor 用 SPI_SETCURSORS 从
    注册表重载恢复（无条件执行，崩溃重启也能恢复）；Mac 用 NSCursor hide +
    tap 回调移动补藏（macOS 移动自动重显）对抗。lib.rs tick 每 100ms 调
    enforce_cursor_hidden()（Windows 已无需补藏，空实现）。
  - 远程拖拽（Win 合成侧，2026-08-13）：attach_drag_image 用
    IDragSourceHelper::InitializeFromBitmap 挂文件类型图标拖影（SHGetFileInfoW
    按扩展名解析，目标机无实体文件）；create_drag_capture_window 建拖拽线程的
    隐形捕获窗口 + SetCapture，把 SendInput 注入事件路由进 DoDragDrop 模态循环
    所在线程（否则不跟手、松手不落）。
- net/ — TCP（按键、剪贴板，可靠）+ UDP（鼠标高频移动）。二进制协议，粘包处理。
  读循环按 payload 分流：FileBatch*/File*/DragStart/DragCommit/DragCancel 进 file_incoming
  （run_file_router 处理），其余进普通 incoming（run_incoming_router）。新增协议消息时
  必须检查这里的分流名单，漏加会导致消息被普通路由丢弃。
- clipboard/ — 监听系统剪贴板变化（Win: AddClipboardFormatListener；Mac: NSPasteboard changeCount 轮询），变化后经 TCP 发给对端；带标记位防回环（收到自己发的不再转发）。对端消息写本机剪贴板（文字/图片/文件路径）。
- file_transfer/ — M3 文件分块传输：FileSender 把文件按 256KB 分块发 FileStart→Chunk*→FileEnd；FileReceiver 状态机写盘到下载目录（重名自动 (1)(2)），完成后路径写本机剪贴板 + emit file-received 通知前端。触发：本机剪贴板出现文件路径 / GUI 选文件 / 跨屏拖拽。
  - 拖拽启动时序（2026-08-13）：DragStart 只同步 build_roots（stat 顶层条目）
    立即发通告，对端光标进入瞬间即启动合成拖拽；完整目录树遍历推迟到 DragCommit
    后 build_plan 再构建（大文件夹不让对端拖拽起步过晚）。build_roots 与
    build_plan 同序 unique_logical_name，保证 root 名与 FileBatchStart 一致。

## 核心机制（想通这三点，代码就顺）

1. 角色动态仲裁：不设固定主控/被控。谁的鼠标在动谁就是主控，另一台收事件注入。"心跳 + 令牌"避免两边同时发。
2. 跨屏判定：本机鼠标 x 超出右边缘且停留 100~200ms（防误触，手感关键）→ 把鼠标从右边缘"滑过去"，通知对端注入从左边进入的事件；回不来同理。
3. 剪贴板防回环：标记位，收到自己发的消息不再转发。

## 开发路线（里程碑）

- M1 单机自测：本机捕获→本机注入，验证钩子和注入链路通。（滑自己屏幕"隔空打字"）
- M2 双机打通：网络层 + 跨屏判定 + 键位映射。两台电脑真能共用一套键鼠。
- M3 剪贴板 + 文件 + 跨屏拖拽 + 托盘 UI：剪贴板（文字/图片/文件）同步、文件传输、跨屏拖拽、托盘、设置窗口、开机自启、打包分发。
- M4 打磨：DPI 适配、鼠标平滑、延迟优化、断线重连、日志。

当前进度：骨架搭建完成（M0）。托盘图标能弹出 = M0 验收标准。

## 明确不做（第一版）

加密传输、跨公网、多显示器复杂布局、手机端。
（注：文件拖拽原列入「不做」，已解锁——M3 实现为"本机剪贴板文件/图片自动跨屏传 + 光标处粘贴"。）

## 提前踩过的坑（写代码时注意）

1. Mac 辅助功能权限：设置窗口里必须有"申请辅助功能权限"按钮引导用户授权，否则 Mac 端完全收不到键鼠事件。
2. DPI：坐标统一按逻辑像素算，别用物理像素。
3. 边缘停留 100~200ms 再切换，一碰就切会误触。
4. 键位映射表：Ctrl↔Command 硬映射，Win→Command，Shift/Alt 不动，第一版就三条。

## 常用命令

> 完整的环境安装、编译、自测/双实例/双机测试步骤见 **[BUILD.md](BUILD.md)**（含 Windows/Mac 双平台、环境变量速查、常见问题）。

```bash
# 安装 Rust（本机还没有，必须装）
winget install Rustlang.Rustup        # Windows 装 rustup
# 或手动：https://rustup.rs

# 生成/更新托盘图标
node scripts/gen-icon.mjs

# 开发运行（弹出托盘图标 + 设置窗口）
cargo run --manifest-path src-tauri/Cargo.toml

# 编译检查
cargo check --manifest-path src-tauri/Cargo.toml

# 打包（Windows 产物固定输出到 D:/ruiss-target）
# 注意：必须显式指定 CARGO_TARGET_DIR=D:/ruiss-target（不要依赖环境变量，
# 它当前未生效——scripts/set-ruiss-env.ps1 未成功执行过，否则会退回默认
# src-tauri/target，造成"每次都打包到别处"）
cd src-tauri
env CARGO_TARGET_DIR=D:/ruiss-target cargo build          # debug → D:/ruiss-target/debug/ruiss.exe
# env CARGO_TARGET_DIR=D:/ruiss-target cargo build --release # release → D:/ruiss-target/release/ruiss.exe

# 正式打包安装程序（Windows 出 NSIS 安装 exe）
# 根目录 package.json 已配好 @tauri-apps/cli，直接：
npx tauri build --bundles nsis   # 产物：src-tauri/target/release/bundle/nsis/Ruiss_<ver>_x64-setup.exe
# 注意：tauri.conf.json 已配 bundle.resources 把 target/release/WebView2Loader.dll 打进安装包
#（GNU 工具链 exe 动态依赖该 DLL，打包器不会自动带，必须显式配置）
```

## 当前进度

- [x] M0 骨架：目录结构、Cargo.toml、Tauri 配置、托盘 + 设置窗口、模块占位、文档
- [x] M1 单机自测（本机捕获→注入）：
  - 已实现：Windows 低层钩子捕获 + SendInput 注入 + INJECTED 标记防回环；
    延迟回声（按下先入表、松开时注入完整按下+松开对，避开按键状态合并）；
    鼠标移动只计数不注入；修饰键按住时不出回声。
  - 已自动化验证：注入生效（GetAsyncKeyState 校验）、跨进程事件被钩子捕获
    （探针 + RUISS_NO_SUPPRESS）、正常模式防回环生效。
- [x] M2 双机打通（网络层 + 跨屏判定 + 键位映射）：键鼠跨屏已完成并实机联测。
- [x] M2.5 跨屏稳定性（2026-08-16）：握手看门狗——TakeControl 发出后 3 秒未收到
  ControlReady（对端崩溃/卡死/双方抢控）自动回滚：恢复光标 + 解除输入屏蔽 +
  通知对端 ReleaseControl + 清理本机拖拽。防"对端不确认 → 本机光标永久隐藏、
  输入永久屏蔽、只能重启"。实现：lib.rs RouterState.tx_take_at + tick 循环检查。
- [~] M3 剪贴板 + 文件 + 跨屏拖拽 + 托盘 UI：
  - 已实现（代码层）：协议扩展（文字/图片/文件/分块流/拖拽通告）；
    Win+Mac 平台剪贴板读写与监听（防回环）；剪贴板同步协调器；
    文件分块传输（发送/接收/写盘/入剪贴板/通知前端）；跨屏拖拽（带内容跨屏 + 光标处粘贴）；
    前端文件传输区 + 传输记录；网络单帧上限 1MB→16MB；
    Windows 远程拖拽系统图标拖影 + 拖拽捕获窗口（2026-08-13）。
  - 验证状态：Windows 端 cargo check/build 通过（见 CHANGELOG M3 条目）；
    Mac 端 mac.rs 剪贴板/拖拽为首次编写，需 Mac 编译实测并迭代；
    双机实机联测（剪贴板同步、文件收发、拖拽粘贴）待做。
- [ ] 打包分发（cargo tauri build）
- [ ] M4 打磨

## 环境备注

- 本机（2026-08-05）：Windows + Git Bash；node v24.15.0 / npm 11.12.1 / git 2.54.0；Rust 尚未安装。
- 首次运行前：装 Rust（上面命令），然后 `cargo check --manifest-path src-tauri/Cargo.toml` 验证骨架可编译，再 `cargo run` 跑托盘。
- Windows 依赖 WebView2 Runtime（Win10/11 一般自带，没有的话装一下）。

## 跨屏桌面零 hover：全屏透明罩子窗口（win.rs）

- SHIELD_HWND：罩子窗口句柄（isize 存储，HWND 不 Send）
- create_shield_window：WS_EX_LAYERED + WS_EX_TOPMOST + WS_EX_NOACTIVATE
  全屏透明窗口，alpha=1（不可见但 hit-test 有效）
- set_local_input_blocked(true/false) 与跨屏状态联动 ShowWindow
- 原理：罩子接住鼠标消息 → 桌面收不到 → 零 hover；WH_MOUSE_LL 钩子
  先于窗口拿到事件 → 转发链不受影响

## 被控端防双鼠标（SINK_ACTIVE）

- win.rs / mac.rs：static SINK_ACTIVE + set_sink_active(active)
- 捕获侧：Sink 时吞本机 MouseMove（win.rs mouse_proc 返回 1；mac.rs tap 回调返回 None）
- lib.rs：TakeControl 收到 → set_sink_active(true)；ReleaseControl / 断线复位 → false
- 原理：被控时光标只跟对端注入走，本机输入不再移动光标 → 无双鼠标/乱跳
