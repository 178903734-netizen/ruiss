# CHANGELOG

## 2026-08-17 — README 增加交流群二维码与微信号

- 新增 docs/IMG_6373.jpeg（桌面 IMG_6373.jpeg 复制入库）。
- README.md 末尾新增「交流群」段落：欢迎扫码添加微信（微信号：hu1562）进群交流。
- 原因：开源项目（2026-08-14 推送 GitHub）需要给访客一个入群交流入口。

## 2026-08-16 — 跨屏握手看门狗：TakeControl 3 秒未确认自动回滚

修复：跨屏发起后对端若崩溃/卡死/双方抢控而未回 ControlReady，本机光标永久隐藏、
输入永久屏蔽（只能重启）的问题。现在 3 秒内未收到对端确认会自动恢复本机光标与
输入，并通知对端释放控制（含清理本机拖拽会话）。改动集中在 lib.rs：RouterState
新增 tx_take_at 计时字段；tick 循环（25ms）检查超时并回滚；所有 tx 状态复位点
（断线/Release/重配/接管）同步清空计时。验证：cargo check 通过。

## 2026-08-16 — 新增三平台视频文案（B站 / 抖音 / YouTube）

- 新增 docs/video-scripts.md：为开源发布准备的宣传视频脚本——
  通用演示镜头清单（11 个镜头）、B站中长视频口播稿+分镜（约 3'40"，
  含标题/封面/简介/时间轴/标签）、抖音竖屏口播稿+大字幕分镜（20s +
  10s 极简版）、YouTube 英文脚本（约 6 分钟，含标题/Description 章节/
  Tags）、发布建议与诚实边界（只宣传当前版本真实可用功能，Mac 端
  未实测前不出现 Mac 镜头）。
- 原因：项目已开源（2026-08-14 推送 GitHub），需要发 B站/抖音/YouTube
  宣传文案。

## 2026-08-15 — 修复开机自启不生效（mac/win 都只存了设置、从未真正注册系统）
根因：Settings 有 autostart 字段并持久化，但全项目没有任何代码调用操作系统注册自启
（无 tauri-plugin-autostart 依赖、无注册表 Run 键写入、无 LaunchAgent）。
修复：platform 层新增 set_autostart ——
- Windows：写/删 HKCU\Software\Microsoft\Windows\CurrentVersion\Run 键（值名 Ruiss，
  指向当前 exe，值带引号防空格路径被拆开；用系统自带 reg.exe，不新增依赖）
- macOS：写/删 ~/Library/LaunchAgents/com.ruiss.app.plist（RunAtLoad）
- save_settings 检测 autostart 变化时应用（失败不落盘）；启动时对齐一次（防升级/移机后失效）
- 已重新打包：D:/ruiss-target/release/bundle/nsis/Ruiss_0.2.0_x64-setup.exe（2026-08-15 22:46，含本修复）

## 2026-08-14 — 开源准备：README + MIT LICENSE + 仓库清理（已推送 GitHub）

- 新增 README.md：项目介绍、功能特性、技术栈、构建方式、使用说明、目录结构
- 新增 LICENSE：MIT 协议
- package.json 移除 `private: true`，正式开源
- .gitignore 排除本地 AI 工具数据（.workbuddy/.zcode/.claude/.codex-login/.cargo），并从 git 跟踪移除
- 远程仓库清理：删除 19 个 backup 备份分支，仅保留 master
- master 已推送至 https://github.com/178903734-netizen/ruiss（经 127.0.0.1:7890 代理）

## 2026-08-14 — 修复安装包缺 WebView2Loader.dll（安装后报"找不到 DLL"）

- **现象**：用户实测安装后运行报错「由于找不到 WebView2Loader.dll，无法继续执行代码」。
- **根因**：w64devkit GNU 工具链编译的 exe 动态依赖 WebView2Loader.dll（静态链接不可用），
  而打包器未把它自动打进 NSIS 安装包（build3.log 中 makensis 无任何 .dll 的 File 行）。
- **修复**：`src-tauri/tauri.conf.json` bundle 段新增 `resources`，把
  `target/release/WebView2Loader.dll` 显式映射进安装包根目录；同时 `targets` 从 `all` 改为
  `nsis`（跳过 MSI/wix 下载，避免再次卡网络）。
- 验证状态：待新打包完成（build4.log）后确认 makensis 日志含 WebView2Loader.dll 的 File 行。

## 2026-08-14 — 接入 Tauri 打包工具链，支持打 Windows 安装 exe

- 新增根目录 package.json（private），本地安装 @tauri-apps/cli 2.11.4（未用全局，避免改系统环境）。
- 打包命令：`npx tauri build` → 产出 `src-tauri/target/release/bundle/nsis/Ruiss_<ver>_x64-setup.exe`
  （NSIS 安装程序）+ MSI + 免安装 exe；安装程序会把 GNU 运行时 DLL（libgcc_s_seh-1.dll /
  libstdc++-6.dll / libwinpthread-1.dll）一并装进 Program Files，用户装完即用，无需手动带 DLL。
- 注意：图标暂沿用 `src-tauri/icons/icon.ico`（仅 641B 占位图），正式上线前应替换多尺寸正式图标。

## 2026-08-13 — 修复打包产物输出位置：固定到 D:/ruiss-target

- **现象**：每次 `cargo build` 产物都跑到 `src-tauri/target`，而不是预期的 `D:/ruiss-target/debug`。
- **根因**：`scripts/set-ruiss-env.ps1` 虽写了 `CARGO_TARGET_DIR=D:\ruiss-target`，但该脚本未成功执行过
  （用户级环境变量为空），cargo 找不到该变量便退回默认目录 `src-tauri/target`。
- **修复**：打包命令显式指定 `env CARGO_TARGET_DIR=D:/ruiss-target cargo build`，不再依赖环境变量。
  已在 PROJECT.md「常用命令 → 打包」段落固化该用法。
- 验证：`env CARGO_TARGET_DIR=D:/ruiss-target cargo build` 成功，产物 `D:/ruiss-target/debug/ruiss.exe`
  （21:37 生成）；`src-tauri/target/debug/ruiss.exe` 不存在（确认未跑错位置）。

## 2026-08-13 — 远程拖拽带系统图标拖影 + 拖拽启动时序优化（Windows）；同步 mac 编译修复

- **拖拽图标拖影（win.rs，新增 attach_drag_image）**：用
  IDragSourceHelper::InitializeFromBitmap 给合成拖拽挂文件类型图标拖影——目标机
  没有实体文件，用 SHGetFileInfoW + SHGFI_USEFILEATTRIBUTES 按扩展名从注册表
  解析类型图标，DrawIconEx 画 32x32 位图，COLOR_WINDOW(5) 作透明 key 色；
  失败只降级为系统默认光标，不影响拖拽本体。Cargo.toml 启用
  Win32_Storage_FileSystem 特性。
- **拖拽捕获窗口（win.rs，新增 create_drag_capture_window + SetCapture）**：
  DoDragDrop 的模态循环从"本线程"消息队列取鼠标事件，而 SendInput 注入的
  移动/松开默认投递到前台窗口线程 → 拖拽不跟手、松手不落。创建属于拖拽线程
  的隐形捕获窗口并 SetCapture，把注入事件路由进本线程队列（等价真实拖拽时源
  窗口 capture 鼠标），合成 LEFT_DOWN 也不会误点光标下的前台应用。
- **修复编译错误（win.rs）**：`SetCapture(Some(capture_hwnd))` →
  `SetCapture(capture_hwnd)`（Option<HWND> 不满足 Param<HWND, CopyType>）。
- **拖拽启动时序（file_transfer/mod.rs）**：DragStart 不再等完整目录树遍历——
  改为同步 build_roots 只 stat 顶层条目，立即把 DragStart 通告给对端，让对端
  在光标进入瞬间就启动合成拖拽；完整发送计划（build_plan）推迟到对端真正
  Drop（DragCommit → commit_drag）再构建，避免大文件夹遍历让对端拖拽起步过晚、
  用户松手时落点错乱。build_roots 与 build_plan 用相同顺序的 unique_logical_name，
  保证 DragStart 里的 root 名与后续 FileBatchStart 完全一致。
- **mac.rs**：PENDING_DRAG_PATHS 有效期 2s→15s，drag_probe_updated 持续刷新
  记录时间戳——用户在边缘犹豫/来回微移时路径不因有效期过期（跨屏触发那一刻
  才采样）。
- **同步远程 87686a2（修复 macOS 编译失败）**：移除 .cargo/config.toml 里的
  Windows 专用 target-dir（改由环境变量 CARGO_TARGET_DIR 控制，见
  scripts/set-ruiss-env.ps1）；修复 mac.rs RcBlock 非 Send、CGPoint Encode
  缺失、NSPoint FFI 桥接。
- 验证：Windows cargo check + build 通过（D:/ruiss-target/debug/ruiss.exe
  09:42 生成）；mac.rs 改动需 Mac 端编译确认。

## 2026-08-12 — 修复跨屏拖拽消息被分流丢弃（拖文件过去对端无文件）

现象：跨屏拖拽文件，对端完全没有文件。链路在"对端启动原生拖拽会话"处断开。

- **根因**：net/mod.rs 的读循环按 payload 类型分流——FileBatch*/File* 系列进
  file_incoming（由 run_file_router 处理），其余进普通 incoming（run_incoming_router）。
  但 DragStart / DragCommit / DragCancel 三类跨屏拖拽会话消息漏加了 is_file 名单，
  落入普通队列后被 run_incoming_router 的 _ 分支直接丢弃：
  对端永远收不到 DragStart → 不启动 DoDragDrop/NSDraggingSession → 松开鼠标无 Drop；
  源端也收不到 DragCommit → 永远不 commit_drag → 文件永不发送。
- **修复（net/mod.rs，两端通用，无需分平台）**：把 DragStart / DragCommit / DragCancel
  加入 is_file 匹配列表，让它们进 file_incoming 由 run_file_router 处理。

## 2026-08-11（追加）— 修复 mac→win 跨屏"发起即归还"（Sink 自动返回误触发）

现象：mac→win 刚发起跨屏 ~1s 就被归还（win→mac 正常）。日志：发起 TakeControl 后
紧接着"对端归还控制"。

- **根因**：Win 收到 TakeControl 变成 Sink 后，注入光标落在入口边——而入口边恰好就是
  Win 自己的出口边（Mac 在 Win 右侧，入口 x=w-1 = 出口右边缘）。Win 端 Sink 自动返回
  （on_sink_tick）的判定是"注入光标停在出口边 150ms 没动就归还"→ 跨屏刚建立，Mac 侧
  手还停在边缘（触控板跨屏本就要求边缘停 150ms 触发），如果接下来 150ms 内没重新动起来，
  Win 立刻归还。之前没暴露是因为测双滚时手一直在滑（光标在动，计时持续重置）。
- **修复（arbiter.rs，两端都要重编）**：on_sink_tick 增加 sink_was_away 判定——只有
  【离开过出口边之后】再停回出口边 150ms 才算想返回；刚被接管时注入光标本来就在出口边，
  不算。on_peer_take 重置该标志。
- 附带：mac.rs 跨屏吞移动转发时加 debug 日志 [MAC-MOVE]（RUST_LOG=ruiss_lib=debug 可见，
  用于确认跨屏后移动事件是否持续转发到对端）。
- 验证：Windows cargo check 通过；测试代码编译通过（本机 0xc0000139 无法运行测试，
  断言已人工核对）；需双机重编后实机验证。

## 2026-08-11 — 根治 Mac 跨屏双滚 + Dock 隔空触发（Session tap + 吞移动 + 热区回避）

背景：此前 4e4ffc5（8/8）与 55e86cc（8/10）两次修复都被回退，两个老问题仍在
（Mac 跨屏到 Win 后滑触控板两边都滚 / Win 光标到任务栏时 Mac 隐藏光标出现在
Dock 位置并触发应用悬停动画）。本次基于 6618b71 实测结论重做。

- **为什么之前修不好（关键）**：
  - 4e4ffc5：warp 到屏幕中心切断坐标镜像（对端光标首动跳屏中央，"好像改了东西"）；
    吞移动无 delta 累积 → 对端光标卡住；释放时不恢复光标。
  - 55e86cc：补了虚拟位置/热区回避/释放恢复，方向正确，但 tap 仍在 HID 层——
    8/11 实验 6618b71 实测：触控板惯性滚动（momentum）由 WindowServer 内部合成
    派发，**HID 层 tap return None 拦不住**（Mac 应用照样滚），双滚根因没动到，
    所以 23 分钟后被回退。
- **问题 1 根因（双滚）**：momentum 由 WindowServer 合成、pid 非 0 → 旧的
  `pid != 0` 防回环早退把它当"自有注入"直通 → 跨屏期间 Mac 本地照滚。
  - tap 捕获层 HID → **Session**（WindowServer 对外派发层，momentum 派发给应用前
    必经此层，实测可拦）；
  - 早退 `pid != 0` → `pid == 本进程 pid`（OWN_PID OnceLock；Synergy/InputLeap
    同款：Session tap + 按 source pid 识别自身注入）→ momentum 落回正常"吞+转发"。
- **问题 2 根因（Dock 隔空触发）**：Source 跨屏期间本机 MouseMoved 只转发不吞 →
  隐藏光标与物理鼠标 1:1 滑向底部 Dock/顶部菜单栏热区 → macOS 在系统 UI 区域不
  约束 CGDisplayHideCursor（强制重显）+ 悬停动画只看位置必触发。
  - Source 跨屏（BLOCK_LOCAL_INPUT）期间吞 MouseMoved/Dragged，转发改用
    SOURCE_VIRTUAL_POS（delta 累积，与 Sink 侧 LOCAL_VIRTUAL_POS 同款机制）——
    光标冻结在安全位，本地零 hover，对端镜像不断；
  - 新增 warp_cursor_cross：落点 y 避开 Dock(20)/菜单栏(25) 热区 + 播种虚拟位置
    （x 仍在对侧边缘，不改屏幕中心——规避 4e4ffc5 的镜像断裂）；
  - 释放（blocked true→false）时把冻结光标 warp 回最后虚拟位置（补缺失的恢复）；
  - 仲裁器 Warp 排 TakeControl 前（warp 先于 blocked=true，种子确定），两个测试
    断言同步更新；win.rs 加同名桩（Windows 无热区问题，行为等同 warp_cursor）。
- 验证：Windows cargo check 通过（mac.rs 不参与 Windows 编译）；cargo test 本机
  已知 0xc0000139 不可用，仲裁器测试已人工核对断言。
- **需 Mac 实机验证**：①跨屏甩触控板，应只有对端滚（看日志 [MAC-SCROLL-DIAG]
  pid / blocked / sink，预期惯性段 pid≠本进程、被吞）；②Win 光标到任务栏，Mac
  无光标出现、无 hover 动画；③返回后光标落在手所在处；④win→mac（Mac 为 Sink）
  注入仍正常（验证 Session tap 下自有注入 pid 识别正确，无回环/无吞注入）。


## 2026-08-10 — 修复 M3 剪贴板/文件功能 Windows 编译错误（15 处）+ 清理 warning

- 背景：M3 跨屏剪贴板（文字/图片/文件）+ 文件传输 + 跨屏拖拽功能代码未提交且 Windows 侧编译失败（15 个 error，全部在 win.rs 新增剪贴板实现段）。
- 根因（windows crate 0.58 API 签名误用）：
  - GlobalLock 返回裸指针 `*mut c_void`（不是 Result）→ `.ok()?` 和 `if let Ok(ptr)` 全部编译失败，
    改为判空：`let ptr = GlobalLock(h); if ptr.is_null() { return None; }`（read_global_bytes /
    read_global_string / dib_to_png / 三处 clipboard_write_* 共 7 处）。
  - DragQueryFileW 只收 3 参数（windows crate 用 `Option<&mut [u16]>` 自带长度，不再传 cch）
    → 删掉第 4 参（read_hdrop 三处）。
  - RegisterClipboardFormatW 返回 u32（不是 Result）→ `if let Ok(fmt)` 改为 `if fmt != 0`
    （read_inner / clipboard_write_image 两处）。
  - HGLOBAL 在 Win32::Foundation 模块而非 System::Memory → 修正路径 + 顶部 import 补 HGLOBAL。
- 清理新增 warning：lib.rs 未使用的 Emitter import、win.rs 未使用的 wstr_to_string 函数。
- 验证：`CARGO_TARGET_DIR=D:/ruiss-target cargo check` 通过（EXIT_CODE=0，5s 增量）。
- 备注：期间曾遇 os error 32（文件被占用）——是 ruiss.exe 应用运行中锁住了 build 产物，
  关掉应用后编译正常；与代码无关。

## 2026-08-10 — M3：跨屏剪贴板（文字/图片/文件）+ 文件传输 + 跨屏拖拽

- 目标：在已完成的 M0/M1/M2（键鼠跨屏）之上，解锁"跨屏复制粘贴文字/图片/文件"
  和"跨屏拖拽图片/文件"。原 PROJECT.md「明确不做：文件拖拽」本次解锁。
- 协议（core/protocol.rs）：保留并启用 ClipboardText / ClipboardImage；新增
  ClipboardFiles（路径列表）、文件分块流 FileStart/FileChunk/FileEnd/FileCancel（256KB/块）、
  DragOffer（拖拽跨屏通告）。网络单帧上限 1MB→16MB（net/tcp.rs MAX_FRAME）。
- 平台剪贴板层（platform/win.rs + mac.rs）：
  - 读：clipboard_read() 按 files>image>text 优先级返回（Win: CF_HDROP / CF_PNG+CF_DIB→PNG /
    CF_UNICODETEXT；Mac: NSPasteboard NSFilenames/PNG/TIFF→PNG/NSString）。
  - 写：clipboard_write_text / write_image / write_files（Win: CF_UNICODETEXT /
    CF_DIB+CF_PNG / CF_HDROP；Mac: NSPasteboard 对应类型）。
  - 监听：start_clipboard_watcher（Win 用 AddClipboardFormatListener + 隐藏窗口；
    Mac 用 NSPasteboard changeCount 轮询）。LOCAL_WRITE 标志防回环（本机写入触发的变化跳过）。
- 剪贴板同步（clipboard/mod.rs）：本机变化→发对端；对端 Clipboard* 消息→写本机剪贴板。
- 文件传输（file_transfer/mod.rs）：FileSender 分块发送（uuid 标识，下载目录 dirs::download_dir，
  重名自动加 (1)(2)）；FileReceiver 状态机写盘，完成后路径写入本机剪贴板 + emit
  "file-received" 通知前端。
- 跨屏拖拽（lib.rs execute_action）：TakeControl 时若左键按下，读本机剪贴板（文件/图片/文字）
  带过去；对端收到 DragOffer 后文字/图片在光标处注入粘贴（Mac: Cmd+V / Win: Ctrl+V），
  文件传完路径已入剪贴板供手动粘贴。
- 集成（lib.rs）：apply_link_config 按 clipboard_enabled 启动 ClipboardSync；注册
  send_file / pick_and_send_file 命令（tauri-plugin-dialog）；run_incoming_router 分发
  剪贴板/文件/拖拽消息。前端 gui：加"跨屏文件传输"区（选文件发送 + 传输记录）。
- 依赖：arboard(已有) + png / dirs / uuid / tauri-plugin-dialog。
- 验证：Windows cargo check 通过（需在 C: 盘 target 编，D: 盘 .cargo-build-lock 有
  safe-delete 拦截导致的拒绝访问，无法复用）。Mac 端 mac.rs 新增剪贴板/拖拽代码为首次
  编写，需 Mac 端编译实测（NSPasteboard API 签名若有误，按报错迭代）。
- 已知待 Mac 验证：clipboard_read/write 的 NSPasteboard 调用、拖拽检测用 NSEvent
  修饰键判定；Win 端可编译但双机行为需实机联测。

## 2026-08-08(5) — 回退 4e4ffc5（mac 跨屏双滚 + Dock 隔空触发），对齐 2b01364

- 原因：4e4ffc5 在 Mac 端实测有问题（用户反馈"好像改了东西"），回退到
  已验证的 2b01364（双击 click state + MouseDragged 拖动修复）。
- 方式：git revert --no-edit 4e4ffc5 生成反向提交 f20075d，保留完整历史、
  可正常推送；代码状态与 2b01364 完全一致（git diff 2b01364 HEAD 仅剩
  .workbuddy 记忆文档差异，mac.rs 已一致）。
- 验证：git diff 2b01364 HEAD --stat 无 mac.rs；cargo 不重新编译
  （mac.rs 不参与 Windows 编译）。

## 2026-08-08(4) — 修复 Mac 端双击失效：注入事件补 click state

- 现象：跨屏到 Mac 后双击（进文件夹/打开应用）失效，变成两次单击只选中不打开。
- 根因：mac.rs InputInjector 注入鼠标按下事件时未设置 kCGMouseEventClickState 字段，
  macOS 窗口服务把两次注入点击识别为两次独立单击而非一次双击（Finder 双击打开依赖该字段）。
  7775a7c 曾修过但连带 CGEventSource 跨线程问题被回退，本次只动注入 click state。
- 修复（src-tauri/src/platform/mac.rs，仅 InputInjector，不碰 CGEventSource）：
  - 新增 ClickState（last_time / last_pos / count）+ 常量 DOUBLE_CLICK_WINDOW=500ms、
    DOUBLE_CLICK_DISTANCE=4px。
  - InputInjector 增加 Mutex<ClickState> 字段（new() 初始化）。
  - inject() 的 MouseButton down 分支：距上次点击 ≤500ms 且落点偏移 ≤4px → count+1，
    否则重置为 1；通过 ev.set_integer_value_field(MOUSE_EVENT_CLICK_STATE, count)
    写入事件字段；up 分支不写（保持默认）。
- 验证：Windows 侧不编译 mac.rs（cfg gate），cargo check 仅确认项目整体未破坏；
  mac.rs 需 Mac 端编译实测（双击打开文件夹、滑动选中、单击拖拽）。

## 2026-08-08(3) — 键盘映射补齐：导航键 + 标点符号（只改映射表，不动注入器）

- 背景：7775a7c 曾补齐键盘映射但连带 CGEventSource 复用导致 Mac 编译报
  NonNull<CGEventSource> cannot be sent between threads（21 个错误），
  回退后又丢了键盘映射修复。本次只做映射表补齐，不碰 InputInjector，
  Mac 端无编译风险。
- keys.rs：Key 枚举新增导航/编辑键（Delete/Home/End/PageUp/PageDown/Insert/CapsLock）
  和标点符号（Comma/Period/Slash/Semicolon/Quote/LBracket/RBracket/Backslash/Minus/Equals/Backtick）。
- win.rs：VK_MAP 补齐对应 Windows VK 码（0x2E/0x24/0x23/0x21/0x22/0x2D/0x14、
  0xBC-0xC0 等），解决 Win 按键在捕获侧变成 Other、目标端乱码/丢键。
- mac.rs：CG_MAP 补齐对应 CGKeyCode（Delete→117、Home→115、End→119、PageUp→116、
  PageDown→121、Insert→114、CapsLock→57、标点→43/47/44/41/39/33/30/42/27/24/50）。
- 验证：Windows cargo check 通过（1.97s 零错误）；mac.rs 改动为纯数据表，
  Windows 不编译，需 Mac 端编译确认。

## 2026-08-08(2) — 回退到 8907359（Mac 端问题多，对齐已验证版本）

- 原因：7775a7c（双击/滑动选中 + 键盘映射修复）在 Mac 端实测问题太多，
  Mac 端暂时不可用 → 整体回退到 8907359（Sink 侧注入滚轮不吞）。
- 方式：git revert 生成反向提交（61a8512 回退 docs、92f0429 回退 fix），
  保留完整历史、可快进推送；代码状态与 8907359 完全一致（git diff 8907359 为空）。
- 原版本保留在本地备份分支 backup-5657fe5（含 7775a7c + 5657fe5），
  后续 Mac 端排查完可再 cherry-pick 回来。
- 验证：git diff 8907359 HEAD --stat 为空；已推送远程（5657fe5..92f0429）。

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
