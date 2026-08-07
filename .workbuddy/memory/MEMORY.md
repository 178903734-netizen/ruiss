# Ruiss 项目长期记忆

双机键鼠共享（Rust + Tauri 2）。Windows + Mac 跨屏，鼠标滑到边缘切到对端，键鼠共用，剪贴板同步。

## 关键架构约定

- 平台代码 cfg gate 分发：platform/mod.rs 用 `#[cfg(target_os=...)]`，mac.rs 在 Windows 不编译、win.rs 在 Mac 不编译。改一端不会影响另一端编译，但跨端验证必须各自在真机做。
- 角色动态仲裁：Source（控制方，发事件）/ Sink（被控方，注入）。无固定主控，谁动谁主控。
- 跨屏时：Source 侧隐藏本机光标 + block 本机键鼠（移动放行转发）；Sink 侧显示光标（注入移动它）+ set_sink_active 吞本机 MouseMoved（防双鼠标）。

## 光标隐藏机制（双鼠标问题核心）

两端必须系统级对称，app 级 API 在跨窗口场景必失效：
- Windows：SetSystemCursor 内核级替换 4 种系统光标为透明图（任何窗口 LoadCursor 都拿透明图）。show_cursor 用 SPI_SETCURSORS 重载，无条件执行（防崩溃后卡死）。
- Mac：CGDisplayHideCursor/CGDisplayShowCursor（CoreGraphics 系统级，CoreGraphics framework，CGMainDisplayID 取主屏）。幂等：CURSOR_HIDDEN 标志，hide 仅 false→true 调一次、show 仅 true→false 调一次，严格配对。
- 关键坑：CGDisplayHideCursor/ShowCursor 只对【前台应用】生效（Apple 官方文档），跨屏时 ruiss 必在后台 → 静默无效。必须先用私有 CGS API 开后台权限：CGSSetConnectionProperty(_CGSDefaultConnection(), _CGSDefaultConnection(), "SetsCursorInBackground", true)（Synergy/InputLeap 同款），hide/show 前各调一次；CG 调用后补 CGAssociateMouseAndMouseCursorPosition(1)（修 randomly not hiding）。CG 返回值必须打日志，不能吞。
- 踩坑：NSCursor hide 是 app 级，只在光标位于本应用窗口时生效，跨屏时失效 → mac→win 双鼠标根因。已弃用。
- CGDisplayHideCursor 鼠标移动不自动重显，无需"移动补藏"；enforce_cursor_hidden 保留空实现供 lib.rs tick 统一调用（与 win.rs 同名对称）。

## 编译运行

- Windows：GNU 工具链，RUSTUP_HOME/CARGO_HOME 在 D 盘，CARGO_TARGET_DIR=D:\ruiss-target。产物 D:\ruiss-target\debug\ruiss.exe + WebView2Loader.dll 必须同目录。
- Mac：cargo build/run，需辅助功能权限（系统设置→隐私与安全性→辅助功能）。core-graphics 0.24 API。
- cargo test 在 GNU 工具链下崩 0xc0000139，用自测模式/双实例/探针验证。
- 环境变量：RUST_LOG=ruiss_lib=debug（日志）、RUISS_SELF_TEST=1（自测）、RUISS_NO_SUPPRESS=1（关注入过滤，仅调试）、RUISS_TCP_PORT/UDP_PORT/CONFIG_FILE（单机双实例）。

## 胡先生的协作习惯（项目相关）

- 改代码前先 git 备份（确认干净基线，改坏能回滚）。
- 先分析排查、查代码逻辑再动手改；给直接结论，别寒暄。
- 并行用多个大模型改代码再交叉审查。
