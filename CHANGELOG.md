# CHANGELOG

## 2026-08-06 — Windows 光标隐藏升级：SetSystemCursor 系统级透明替换（协作改动）

- 背景：ShowCursor 计数方案实测仍双鼠标（tao/其他窗口可把计数拉回），
  协作智能体改用 SetSystemCursor 系统级替换，修复其编译错误 + 崩溃恢复 bug。
- win.rs（Windows）：
  - hide_cursor()：创建 1x1 全透明光标（AND 全 1 / XOR 全 0），SetSystemCursor
    替换 4 种系统光标（箭头/文本/手型/等待）——内核级替换，任何窗口
    LoadCursor/SetCursor 都拿到透明图，不再受 per-thread 计数影响。
  - show_cursor()：SPI_SETCURSORS 从注册表重载所有系统光标。
    改为无条件执行（不检查 CURSOR_SUPPRESS）——修复崩溃/强杀后重启时
    show_cursor 因标志为 false 直接 return、系统光标永久透明的 bug。
  - enforce_cursor_hidden() 保留空实现（SetSystemCursor 无需补藏）。
  - 修复编译错误：CopyCursor 未解析 → 改为每次新建光标传给 SetSystemCursor
    （它会销毁传入句柄）；TRANSPARENT_CURSOR 静态 OnceLock<HCURSOR> 不满足
    Send/Sync → 删除静态缓存；CreateCursor 掩码指针类型 → *const c_void；
    SetSystemCursor 第二参数 → SYSTEM_CURSOR_ID(id)。
- mac.rs：enforce_cursor_hidden 的 NSCursor hide 改用
  performSelectorOnMainThread 调度主线程（NSCursor 必须在主线程调用）。
- 验证：Windows cargo check/build 通过（10:26 exe）；mac.rs 需 Mac 端编译确认。

## 2026-08-06 — 双鼠标根治：两端对称的"持续补藏"机制

- 根因：光标隐藏是"一次性"的，会被外部/系统拉回显示：
  - Windows：tao/WebView2 会不定期 ShowCursor(TRUE) 把计数拉回 0，原补藏只
    在鼠标移动事件时触发（mouse_proc），用户停手不动时补藏停止 → 光标重新出现。
  - macOS：NSCursor.hide 后系统在鼠标/触控板移动时自动重显光标，且 mac.rs
    完全没有补藏逻辑 → 一滑动双鼠标必现。
- 修复（两端对称三件事）：
  - lib.rs spawn_tick_task：Source 跨屏期间（linked）每 100ms 主动调
    platform::enforce_cursor_hidden()——停手不动也持续压制，不依赖移动事件。
  - mac.rs：新增 enforce_cursor_hidden()（CURSOR_SUPPRESS 时再 hide 一次），
    tap 回调里真实 MouseMoved 事件后调用（对抗 macOS 移动自动重显）；
    hide/show 改为计数配对（CURSOR_HIDE_COUNT 记录 hide 次数，show 循环 unhide
    同样次数，NSCursor 计数制防失衡）；CURSOR_HIDDEN 幂等标志废除（它会让
    补藏失效：外部拉回显示后标志仍是 true，hide_cursor 直接跳过）。
  - win.rs：enforce_cursor_hidden_on_move 改名为 pub enforce_cursor_hidden，
    供 lib.rs tick 调用（原有 mouse_proc 移动补藏保留）。
- 验证：cargo check 通过；mac.rs 改动需 Mac 端编译确认。

## 2026-08-06 — 修复返回光标位置 + 被控端光标可见性

- 现象 1：原路返回后鼠标从屏幕另一头出现（跨屏时 Source 侧 Warp 到对侧，返回时没 warp 回来）。
  - arbiter.rs：新增 exit_pos 字段记录跨屏出口位置 + exit_edge()/exit_pos() 访问器。
  - lib.rs：收到 ReleaseControl 时，根据 exit_edge/exit_pos 把光标 warp 回出口边内侧。
- 现象 2：Windows→Mac 跨屏后 Mac"显示被控但找不到鼠标"，必须大面积滑动才出现。
  - 根因：lib.rs 收到 TakeControl 时调 hide_cursor()，把被控端唯一的系统光标藏了
    （Mac 上注入事件移动的就是系统光标本身，没有第二个光标可显示）。
  - 修复：Sink 侧收到 TakeControl 改为 show_cursor()（保持可见）；Source 侧照旧隐藏。
- mac.rs：hide_cursor/show_cursor 幂等化（CURSOR_HIDDEN 标志位，NSCursor 计数制
  防 hide 重复调用后 unhide 拉不回来）。
- 验证：cargo check 通过；mac.rs 改动需 Mac 端编译确认。

## 2026-08-06 — Sink 侧原路返回判定放宽（触控板友好）

- 根因：Mac→Windows 跨屏后无法原路返回。on_sink_tick 要求注入光标精确停在
  出口边 2px 内且位置完全不变 150ms——触控板是相对移动 + 停手后惯性微移，
  位置差 1px 就重置计时，150ms 永远凑不满 → 返回判定永不触发。
- 修复（src-tauri/src/core/arbiter.rs）：
  - 新增 RETURN_MARGIN = 30：Sink 侧返回判定用 30px 宽边缘带（原 EDGE_MARGIN=2 仅用于
    Source 侧跨屏触发，保持精确防误触）。
  - 新增 JITTER_TOLERANCE = 3：注入位置变化 ≤3px 视为"停住"，不重置停留计时，
    抵消触控板惯性/微移。
- 事故记录：Mac 端推送 git-push.sh（36af9a6）时带上了 Mac 本地旧版 arbiter.rs，
  覆盖删除了本修复 → 已 reset 对齐远程后重新应用，确认两处修改在位。
- 验证：cargo check 通过；打包 D:/ruiss-target/debug/ruiss.exe。
- 同步：推送 GitHub，Mac 端 pull 后重新编译同版本测试。

## 2026-08-05 — M1 自测模式 v2：延迟回声 + 统计 + 注入测试

- 修复回环看不到效果的问题：
  - 根因一：即时回声被 Windows 按键状态合并吞掉（物理键未松开时注入同键按下无效）→ 改为
    "延迟回声"：按下先入表，松开时注入完整"按下+松开"对（物理键已抬起，必可见）。
  - 根因二：回环注入鼠标移动造成"被拖拽"手感 → 移动只计数不注入。
  - 修饰键（Ctrl/Alt/Win）按住期间不出回声，避免 Ctrl+C 等组合键被双重触发。
- 新增可观测验证：
  - 设置窗口实时统计（捕获鼠标/键盘数、注入成功/失败数，300ms 轮询）。
  - "注入测试"按钮：向当前聚焦窗口注入 `ruiss 123`，独立验证注入链路。
- 新增开发工具 examples/input_probe.rs：注入探针（full/move/hook 三模式），
  配合 RUISS_NO_SUPPRESS=1 可自动化验证捕获链路。
- 自动化验证结果：注入生效（GetAsyncKeyState 校验通过）、跨进程事件被钩子捕获
  （探针注入 3 次移动 → Ruiss 日志 3 条捕获）、正常模式防回环生效（注入事件被过滤）。
- win.rs：inject() 返回 SendInput 注入条数；钩子捕获事件加 debug 日志；
  支持 RUISS_NO_SUPPRESS=1 关闭注入过滤（仅开发验证用）。

## 2026-08-05 — M1 自测模式：本机捕获→注入链路

- 启用 windows crate 0.58 依赖（Win32 钩子 / 输入 / 消息 / 线程）。
- platform/win.rs 完整实现：WH_MOUSE_LL + WH_KEYBOARD_LL 低层钩子捕获（专用钩子线程 + 消息循环 +
  消费线程回调），SendInput 注入（VK + 扫描码 + 扩展位回放，鼠标坐标按主屏归一化）。
- 防回环：利用钩子自带的 LLMHF_INJECTED / LLKHF_INJECTED 标记丢弃注入事件（M1 防死循环，M2 防再转发）。
- core/protocol.rs：Payload::Key 增加 scan / extended 字段（精确回放左右 Ctrl、方向键等同码键）。
- lib.rs：新增 set_self_test 命令 + SelfTest 状态；支持 RUISS_SELF_TEST=1 启动即开自测。
- GUI：设置窗口加"M1 自测"开关区（窗口加高到 520）。
- 修复：Cargo.toml 去掉 cdylib（GNU 工具链 ld 导出序号溢出，"export ordinal too large"）。

## 2026-08-05 — 初始化项目骨架（M0）

- 按规划搭建 Ruiss 双机键鼠共享项目：git init、目录结构、Cargo.toml（Tauri 2 + tokio + arboard，平台依赖注释待 M1 启用）。
- Tauri 配置：托盘图标 + 设置窗口（默认隐藏，托盘菜单唤起）。
- 模块占位：core（protocol/geometry/keys）、platform（win/mac）、net（tcp/udp）、clipboard。
- 前端：gui/index.html 设置窗口一屏表单（本机名、对端 IP、屏幕布局、剪贴板开关、开机自启）。
- 文档：PROJECT.md（规划存档 + 项目地图）、CHANGELOG.md。
- 图标生成脚本 scripts/gen-icon.mjs（node 生成 PNG）。
- 原因：把规划落地为可运行的骨架，先跑通"托盘图标弹出来"，再按 M1→M4 迭代。

## 2026-08-06 — 跨屏桌面零 hover 改用"全屏透明罩子窗口"方案（回退 Raw Input 后）

- 背景：B 路线（Raw Input 相对位移）在 Windows 实测失败——吞掉本机
  MouseMove 后系统钳制光标，Raw Input 增量也丢失（154a1c1 提交注释里
  作者警告过的坑），对端光标钉死在边缘 → 回退到 16b6a0c（移动放行 +
  绝对坐标转发）
- 新方案：跨屏期间创建全屏透明罩子窗口（WS_EX_LAYERED + alpha=1 置顶），
  鼠标消息打在罩子上、桌面收不到 → 桌面文件零 hover；低层钩子仍在罩子
  之前拿到事件，绝对坐标转发数据链完全不变
- 代价：跨屏期间本机 UI 本就不操作（点击转发对端），罩子挡住符合预期
- 文件：win.rs（create_shield_window + SHIELD_HWND + set_local_input_blocked
  联动开合）、Cargo.toml（加 Win32_Graphics_Gdi / Win32_System_LibraryLoader）
