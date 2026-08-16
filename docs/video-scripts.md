# Ruiss 视频文案集（B站 / 抖音 / YouTube）

> 用途：开源项目 Ruiss 的宣传视频文案。三个平台各一份完整脚本，可直接照着念、照着拍。
> 核心事实核对：名称 Ruiss；MIT 开源；GitHub https://github.com/178903734-netizen/ruiss；Rust + Tauri 2；支持 Win↔Mac / Win↔Win / Mac↔Mac 混用。

---

## 〇、通用素材清单（三个平台共用，先录这些镜头）

| # | 镜头 | 内容 | 用途 |
|---|------|------|------|
| 1 | 全景 | 两台电脑并排实拍（显示器 + 笔记本最出效果） | 所有平台开场 |
| 2 | 鼠标跨屏→ | 光标滑到右屏边缘，停留一下，滑进左屏 | 核心镜头，B站/抖音/YTB 都用 |
| 3 | 鼠标跨屏← | 从另一侧滑回，原路返回 | 展示反向跨屏 |
| 4 | 键盘跟随 | 在 A 屏打字 → 滑到 B 屏继续打字 | 键盘共享 |
| 5 | 键位映射 | Windows 上 Ctrl+C，跨屏后在 Mac 上粘贴成功（Command 自动映射） | Win+Mac 混用 |
| 6 | 剪贴板文字 | A 机复制一句话，B 机直接粘贴 | 剪贴板同步 |
| 7 | 剪贴板图片 | A 机复制一张截图，B 机粘贴出图 | 剪贴板同步（图片） |
| 8 | 跨屏拖文件 | 鼠标拖住文件滑过屏幕边缘，对端出现拖影，松手落盘 | 高光镜头 |
| 9 | 传大文件夹 | 拖一个文件夹过去，展示分块传输/重名自动改名 | 细节展示 |
| 10 | 托盘/设置 | 托盘图标右键菜单 + 设置窗口（端口/开机自启） | 细节展示 |
| 11 | GitHub 页 | 仓库页滚屏 + LICENSE + Star 数 | 开源收尾 |

录制注意：
- 跨屏动作做两遍：正常速度一遍、慢动作一遍（后期卡点用）。
- 演示前先双机自测一遍，确认剪贴板/文件/拖拽在你当前版本上真实可用再录。
- 两台电脑摆得近一点，镜头里"鼠标滑过去"的物理感更强。

---

## 一、B站版（中长视频，约 3 分 40 秒）

### 标题（备选）
1. 鼠标滑一下，就从这台电脑滑到另一台电脑｜我开源了个键鼠共享工具 Ruiss
2. 用 Rust 写了 Synergy 的免费开源替代品，两台电脑共用一套键鼠
3. 让两台电脑变成"一块屏幕"：开源工具 Ruiss 上手实测
4. 【开源】Win 和 Mac 混用一套键鼠，我把这个工具做出来了

### 封面
两屏拼接图 + 一只鼠标从边缘滑出，大字：「开源 · 免费 · Rust」

### 口播稿 + 分镜

**【0:00–0:12】开场钩子**
画面：镜头 1 全景，鼠标滑过屏幕边缘（镜头 2），配 whoosh 音效
> 你敢信，一台鼠标，能从这台电脑的屏幕，直接滑到旁边那台电脑的屏幕里？
> 这不是特效，这是我的桌面实拍。为了这个效果，我用 Rust 写了一个开源的键鼠共享工具，叫 Ruiss。

**【0:12–0:32】痛点引入**
画面：两台电脑挤一张桌子，手来回换鼠标 / 开微信给自己发文件
> 同时用两台电脑的朋友应该懂——鼠标拔来拔去，键盘换来换去，想传个文件还得开个微信群，把文件发给"自己"。
> 这类工具其实早就有，像 Synergy、Barrier。但要么配置繁琐、要么界面停在十年前，Windows 和 Mac 混用的时候还经常踩坑。

**【0:32–0:50】是什么**
画面：GitHub 仓库页滚屏 + 代码片段
> 所以我自己写了一个：Ruiss。免费、开源、MIT 协议。它的目标只有一个——让两台电脑，用起来像同一台电脑的两块屏幕。

**【0:50–1:15】功能一：鼠标跨屏**
画面：镜头 2 / 3，右上角打字幕「鼠标跨屏」
> 第一，鼠标跨屏。光标滑到屏幕边缘，停留一瞬间，就滑进了另一台电脑；再滑回来，原路返回。
> Windows 和 Mac 可以混着用。它没有固定的主从关系——谁动鼠标，谁就是主控，不用设置"哪台是主机"。

**【1:15–1:35】功能二：键盘共享**
画面：镜头 4 / 5，字幕「键盘共享 · 键位自动映射」
> 第二，键盘共享。键盘跟着光标走。在 Windows 上是 Ctrl+C，滑到 Mac 上，它自动帮你按成 Command+C——键位映射好了，不用改肌肉记忆。

**【1:35–1:55】功能三：剪贴板同步**
画面：镜头 6 / 7，字幕「剪贴板同步 · 文字/图片/文件」
> 第三，剪贴板同步。在这台电脑复制，另一台直接粘贴。文字可以，图片可以，文件路径也可以。
> 而且有防回环设计——收到自己发出去的内容不会再次转发，不会无限循环。

**【1:55–2:20】功能四：跨屏拖拽**
画面：镜头 8 / 9，字幕「跨屏拖拽」
> 第四，也是我最喜欢的——跨屏拖拽。直接把文件从这台电脑拖到另一台的桌面上，松手，它就沿着局域网飞过去，落在对端光标的位置。
> 大文件按 256KB 分块传，传完自动进下载目录，重名了还会自动改名加 (1)(2)。

**【2:20–2:40】细节补齐**
画面：镜头 10
> 另外，托盘常驻、设置窗口、开机自启、端口配置……该有的都有。哦对了，跨屏的时候另一台的光标会隐藏，不会出现"两个鼠标打架"。

**【2:40–3:20】原理浅讲**
画面：架构图（Source/Sink + TCP/UDP 双通道）+ 关键代码
> 简单讲讲它背后怎么工作的，写代码的朋友应该会感兴趣。
> 它没有固定主从：谁的鼠标在动，谁就是 Source，另一台变成 Sink，把事件注入到本地系统。屏幕边缘要停留 100 到 200 毫秒才触发跨屏——防止你只是想贴到边缘时误切。
> 网络层走了两个通道：TCP 传按键、剪贴板和文件，可靠不丢包；UDP 传高频的鼠标移动，快。
> Mac 上的坑是最多的：跨屏后出现双鼠标、触控板惯性滚动导致两边一起滚、光标滑到 Dock 上误触发动画……这些我都修了，光标隐藏用的还是跟 Synergy 同款的系统级方案。代码里注释写得很详细，欢迎去看。

**【3:20–3:40】开源与贡献**
画面：GitHub + LICENSE + Star 数
> 目前 Ruiss 以 MIT 协议完全开源，GitHub 链接放在简介和评论区。你可以直接下载安装包用，也可以自己编译、魔改。
> 项目还在早期，如果它帮到了你，去 GitHub 点个 Star；遇到问题欢迎提 issue，会写代码的欢迎提 PR。开源的乐趣，就是大家一起把它变得更好。

**【3:40–3:50】结尾**
画面：回到开场全景，镜头缓缓拉远
> 我是 XX。如果你也想让两台电脑共用一套键鼠，不妨试试。我们下期再见。

### 简介（B站发布用）

> 开源键鼠共享工具 Ruiss：两台电脑共用一套鼠标键盘，鼠标滑过屏幕边缘即可跨屏；剪贴板（文字/图片/文件）自动同步；支持跨屏拖拽文件；Windows + macOS 混用、键位自动映射。
>
> 🆓 免费开源 · MIT 协议
> 🔗 GitHub：https://github.com/178903734-netizen/ruiss
> ⚙️ 技术栈：Rust + Tauri 2
>
> ⏱ 时间轴
> 0:00 开场效果
> 0:12 为什么写它
> 0:32 Ruiss 是什么
> 0:50 鼠标跨屏
> 1:15 键盘共享
> 1:35 剪贴板同步
> 1:55 跨屏拖拽
> 2:40 原理浅讲
> 3:20 开源与贡献
> 3:40 结尾

### 标签
#开源软件 #Rust #键鼠共享 #效率工具 #程序员 #Windows #macOS #桌面工具

---

## 二、抖音版（竖屏 9:16，约 20 秒）

节奏：快、字幕大、BGM 卡点。每句一行大字幕居中，关键词标色。

### 口播稿 + 分镜

| 时间 | 画面 | 大字幕 | 口播 |
|------|------|--------|------|
| 0–3s | 两台电脑，鼠标滑过屏幕边缘（whoosh 音效） | 你的鼠标，能滑到另一台电脑吗？ | 你的鼠标，能直接滑到另一台电脑上吗？ |
| 3–8s | 跨屏实拍 + 键盘跟随 | 我做的开源工具 Ruiss | 我做了个开源工具——Ruiss，鼠标滑到屏幕边缘，直接进入旁边那台电脑，键盘跟着走。 |
| 8–13s | 复制 → 跨屏 → 粘贴 | 这台复制 那台粘贴 | 在这台电脑复制，那台直接粘贴，文字、图片、文件都能跨机同步。 |
| 13–17s | 拖文件跨屏落盘 | 文件直接拖过去 | 文件还能直接拖过去，松手就传完。 |
| 17–20s | GitHub 页面 | 完全免费 MIT 开源 | 完全免费、MIT 开源。评论区扣"工具"，我发链接。 |

BGM：节奏感强的电子乐；跨屏瞬间加 whoosh 音效卡点。
字幕：每句一行，字号拉满，关键词标红/标黄。

### 标题
- 鼠标居然能滑到另一台电脑？我写了个免费开源工具
- 两台电脑共用一套键鼠，这个开源工具免费送

### 话题标签
#效率工具 #开源 #黑科技 #程序员日常 #电脑小技巧 #免费软件

### 10 秒极简版（备选，纯口播）
> 两台电脑共用一套键鼠——鼠标滑到边缘就换屏，剪贴板自动同步，文件直接拖过去。我做的开源工具 Ruiss，免费、MIT 开源。评论区拿链接。

---

## 三、YouTube 版（英文，约 6 分钟）

### Title（备选）
1. Two Computers, ONE Mouse & Keyboard — I Built It in Rust (Open Source)
2. Control 2 Computers with One Mouse — My Open Source Tool Ruiss
3. I Made a Free Synergy Alternative in Rust — Ruiss

### Script

**0:00 Hook** — *Visual: two screens side by side, cursor slides across the edge.*
> Two computers, one mouse, one keyboard. Wait — the cursor just moved from this screen to the other one? Yes. This is my real desktop, not editing. I built Ruiss — a free, open source tool that lets you control two computers with a single mouse and keyboard. Let me show you what it does and what's under the hood.

**0:20 The problem** — *Visual: messy desk, swapping peripherals, emailing files to yourself.*
> If you work across a desktop and a laptop — or a Windows machine and a Mac — you know the struggle. Swapping mouse and keyboard between machines, sending files to yourself, and dealing with clunky tools like Synergy or Barrier that are painful to configure and break across platforms.

**0:45 What is Ruiss** — *Visual: GitHub repo, code.*
> Ruiss is a keyboard and mouse sharing tool written in Rust, with a Tauri 2 UI. It's completely free and open source under the MIT license. It supports Windows and macOS — and you can mix them freely: Win to Win, Mac to Mac, or Windows to Mac.

**1:10 Feature demo**
> Let's go through the features.
>
> **Mouse cross-screen** — *Visual: cursor crosses edge both ways.*
> Slide your cursor to the edge of the screen, hold for a moment, and it slides onto the other machine. Slide back to return. There's no fixed master or slave — whichever side moves the mouse becomes the controller.
>
> **Shared keyboard** — *Visual: typing, Ctrl+C → Cmd+V on Mac.*
> Your keyboard follows the cursor. Ctrl on Windows automatically becomes Command on macOS, so you don't have to relearn your shortcuts.
>
> **Clipboard sync** — *Visual: copy text/image, paste on the other machine.*
> Copy text, an image, or a file path on one machine — paste it on the other. All synced automatically, with anti-loop protection so it never re-broadcasts its own writes.
>
> **Drag & drop across screens** — *Visual: drag a file off one screen, drop on the other.*
> And my favorite: drag a file off one screen and drop it on the other. It transfers over the LAN in 256KB chunks, lands in the download folder, and auto-renames on conflicts.
>
> **Polish** — *Visual: tray icon, settings window, autostart.*
> Tray icon, settings UI, autostart, configurable ports — all the niceties you'd expect are there.

**3:10 How it works (tech deep dive)** — *Visual: architecture diagram, protocol code.*
> For the developers watching, here's how it works under the hood.
>
> It uses dynamic arbitration — there are no fixed roles. Whoever moves the mouse is the Source, and the other machine becomes the Sink, injecting events locally.
>
> Edge detection requires dwelling at the screen edge for 100 to 200 milliseconds before crossing — that prevents accidental switches when you just want to touch the edge.
>
> Networking uses two channels: TCP for reliable input, clipboard and file transfer, and UDP for high-frequency mouse movement.
>
> The clipboard has anti-loop tags, so it never re-shares its own content.
>
> And the platform hacks were the hardest part. On Windows, the cursor is hidden by swapping system cursors with a transparent one via SetSystemCursor. On macOS, it's CGDisplayHideCursor plus a private CGS API for background control — the same trick Synergy uses. I also fixed trackpad momentum scrolling causing double-scrolls on the other machine, and Dock hot-zone triggers. All documented in the code.

**5:20 Open source & contribution** — *Visual: GitHub, LICENSE, issue/PR.*
> Everything is on GitHub under the MIT license — link in the description. You can grab the installer, or build it yourself with cargo. It's still early-stage: clipboard sync, file transfer and drag-and-drop are actively being polished, especially on macOS. So issues, pull requests and stars are all very welcome — this is a real community project.

**6:00 Wrap up** — *Visual: back to the desk.*
> If this freed up some desk space, or saved you from another round of "emailing yourself the file", give it a star. Thanks for watching, and I'll see you in the next one.

### Description（含 chapters）

```
Two computers, one mouse, one keyboard — I built Ruiss, a free open source
keyboard/mouse sharing tool in Rust + Tauri 2. Supports Windows & macOS.

⭐ GitHub: https://github.com/178903734-netizen/ruiss
⚖️ License: MIT

⏱ Chapters
0:00 Demo
0:20 The problem
0:45 What is Ruiss
1:10 Features (cross-screen mouse, keyboard, clipboard, drag & drop)
3:10 How it works (arbitration, protocol, platform hacks)
5:20 Open source & contribution
6:00 Outro

#opensource #rust #keyboardsharing #productivity #windows #macos #tauri
```

### Tags
opensource, rust, keyboard sharing, mouse sharing, KVM software, productivity, windows, macos, tauri, synergy alternative, free software

---

## 四、发布建议与注意事项

1. **评论区置顶**：B站 / 抖音 / YouTube 都把 GitHub 链接放评论区置顶（B站简介不能点外链，置顶评论是主入口）。
2. **发布顺序**：YouTube 英文版内容最全，先剪英文版，再剪中文版复用素材（B站 3 分半、抖音 20 秒）。
3. **B站差异**：技术区观众吃"原理"，保留 2:40–3:20 的技术段；弹幕友好的梗点放在"发送给'自己'"和"两个鼠标打架"这两句。
4. **抖音差异**：前 3 秒必须出现跨屏实拍画面，纯口播会划走；字幕字号 ≥ 屏幕 1/6。
5. **诚实边界**（别踩）：
   - 只宣传当前版本真实可用的功能——M3（剪贴板/文件/拖拽）Windows 侧已验证，**Mac 端需实测通过后再出 Mac 镜头**；
   - 不要说"加密传输""支持公网"——第一版明确不做；
   - 版本号建议提"0.2.0"，不要夸"正式版"。
6. **更新时机**：等 M3 双机实机联测通过、Mac 端验证后再录"剪贴板 + 拖拽"完整演示最稳；现在就发的话，以"鼠标跨屏 + 键盘共享"为主体，剪贴板/拖拽标注"演示中"。
