# CHANGELOG

## 2026-08-08 — 修复 Mac 端双击/滑动选中失效 + 补齐跨平台键盘映射

- 现象：Win 电脑跨屏到 Mac 后，Mac 端双击、滑动选中（按住左键拖动）不生效；
  Win 键盘部分按键（Delete/Home/End/PageUp/PageDown、逗号句号分号等标点）在 Mac 端不能用。
- 根因 1（双击/滑动选中）：mac.rs 的 inject() 每次调用都新建 CGEventSource。
  macOS 按 event source 追踪点击计数（clickCount）与拖拽状态——每次新建 source，
  每次点击都是"第 1 次点击"→ 双击永远不被识别；拖拽关联同样断裂。
- 根因 2（按键失效）：VK_MAP / CG_MAP 只覆盖字母数字、修饰键、方向键、F1-F12，
  大量常用键未映射。未映射键变成 Key::Other(Windows VK 码)，到 Mac 端 key_to_cg 把
  Windows VK 码直接当 CGKeyCode 用——两个平台键码完全不同，注入的是乱键或空键。
- 修复：
  - mac.rs：InputInjector 新增 source: Mutex<Option<CGEventSource>> 字段，首次注入时
    创建一次，后续所有注入复用同一个 source（双击/拖拽恢复识别）。
  - keys.rs：Key 枚举新增导航键（Delete/Home/End/PageUp/PageDown/Insert/CapsLock）和
    标点符号（Comma/Period/Slash/Semicolon/Quote/LBracket/RBracket/Backslash/Minus/Equals/Backtick）。
  - win.rs：VK_MAP 补齐对应 Windows VK 码（0x2E/0x24/0x23/0x21/0x22/0x2D/0x14、
    0xBC-0xC0 等），解决捕获侧变成 Other。
  - mac.rs：CG_MAP 补齐对应 CGKeyCode（Delete→117 ForwardDelete、Home→115、End→119、
    PageUp→116、PageDown→121、Insert→114、CapsLock→57、标点→43/47/44/41/39/33/30/42/27/24/50），
    解决注入侧映射正确。
- 验证：Windows 侧不编译 mac.rs（cfg gate）；mac.rs 需 Mac 端编译实测（双击、滑动选中、标点输入）。
- 提交：7775a7c。

# 2026-08-07(3) — fix: Sink 侧注入滚轮不吞——对端滚动在本机生效
- 根因：win.rs mouse_proc 的 swallow 在 3d3e906（滚动双滚）加了 SINK_ACTIVE 后，
  mac→win 时 win 是 Sink，从对端转发注入的滚轮（LLMHF_INJECTED）被吞掉
  → win 应用收不到滚动，对端滚动在本机失效（mac 动、win 不动）。
- 修复：swallow 排除 injected_wheel（LLMHF_INJECTED 的滚轮消息）——放行对端注入的滚动；
  罗技平滑滚动同样带 INJECTED 一并放行；Sink 时本机无人操作，无双滚副作用。
- 验证：cargo build 通过；双机实测 mac→win 滚动恢复、win 本机滚轮仍被吞（防双滚）。

## 2026-08-07(2) — Mac 双鼠标再根治：CGDisplayHideCursor 只对前台应用生效

- 现象：换上 CGDisplayHideCursor 后 mac→win 仍双鼠标（mac/win 光标都在动）。
- 排查：代码链路无问题（win 光标能动证明 TakeControl 已触发，hide_cursor
  必被调到）；幂等标志无失衡路径。真正根因是 Apple 官方文档明确：
  CGDisplayHideCursor/ShowCursor **只对前台应用生效**（"To use these
  functions, your application must be in the foreground"）。跨屏瞬间 mac 上
  前台是桌面/其他窗口，ruiss 在后台 → hide 静默无效；返回值被 `let _ =`
  吞掉，失败无任何痕迹。win→mac 完美是因为 SetSystemCursor 无前台限制，
  且该方向 mac 作 Sink 本就不藏光标。
- 修复（src-tauri/src/platform/mac.rs，照抄 Synergy/InputLeap 的
  OSXScreen::hideCursor）：
  - 新增 allow_background_cursor_control()：私有 CGS API
    CGSSetConnectionProperty(_CGSDefaultConnection(), _CGSDefaultConnection(),
    "SetsCursorInBackground", true)，允许后台进程控制光标；
    hide_cursor/show_cursor 调 CG 前各执行一次（幂等设置，无副作用）。
  - CGDisplayHideCursor/ShowCursor 后补 CGAssociateMouseAndMouseCursorPosition(1)
    （InputLeap 注释：修 "mouse randomly not hiding/showing" 玄学 bug）。
  - CG 调用返回值不再丢弃，err 打 info 日志（[MAC-CURSOR] 前缀），
    下次再失效能直接从日志看到错误码。
  - 私有符号由 CoreGraphics framework 导出；自用工具不上 App Store，无审核风险。
- 验证：Windows 侧不编译 mac.rs（cfg gate），语法已审；需 Mac 端编译实测。

## 2026-08-07 — Mac 双鼠标根治：光标隐藏改 CGDisplayHideCursor 系统级

- 现象：mac→win 跨屏后 mac 侧光标没消失，mac/win 两边都有鼠标且都在滑动；
  win 侧已完美（SetSystemCursor 内核级替换）。
- 根因：mac.rs 用 NSCursor hide 隐藏光标——这是 app 级 API，苹果文档明确
  "只在光标位于本应用窗口时生效"。跨屏时光标在桌面/别的 app 窗口上，hide
  直接失效；之前的"移动补藏 + 100ms tick 补藏"调的也是 NSCursor hide，app 级
  补藏治不了 app 级失效。Source 侧移动放行转发，光标没藏住 → 看见 mac 本机
  光标动 + win 注入光标动 = 两边都有鼠标都在滑。
- 修复（src-tauri/src/platform/mac.rs）：hide_cursor/show_cursor 从 NSCursor
  hide/unhide 换成 CGDisplayHideCursor/CGDisplayShowCursor（CoreGraphics 系统级，
  与 win 的 SetSystemCursor 对称）。系统级隐藏不挑窗口，鼠标移动不自动重显。
  - 幂等：新增 CURSOR_HIDDEN 标志，hide 仅 false→true 调一次、show 仅 true→false
    调一次，严格配对，避免 CG 内部计数叠加导致 show 不回来。
  - 删除 CURSOR_HIDE_COUNT 计数（幂等后不再需要）。
  - enforce_cursor_hidden 改空实现（CGDisplayHideCursor 移动不重显，无需补藏）；
    tap 回调里两处补藏调用（pid!=0 注入分支、真实移动分支）删除。
  - 删除未使用的 use objc::{sel, sel_impl}。
- 验证：Windows 侧不编译 mac.rs（cfg gate），语法已审；mac.rs 需 Mac 端编译确认。

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

## 2026-08-06 — Mac 双鼠标修复：被控端（Sink）吞掉本机 MouseMove

- 现象：Mac 被 Windows 控制时偶现双鼠标/乱跳——本机触控板一动，
  本机光标被"抢走"，与对端注入的移动打架（Windows 端对称存在）
- 修复：新增 SINK_ACTIVE 标志，Sink 期间捕获侧吞掉本机 MouseMove
  （win.rs mouse_proc / mac.rs tap 回调），光标只跟对端注入走
- 状态同步：TakeControl 收到→true、ReleaseControl→false、断线复位→false
- 文件：mac.rs / win.rs / lib.rs
