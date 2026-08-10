// Ruiss 设置窗口前端逻辑。
// 骨架阶段：先读/写表单（后端命令在 lib.rs 已注册，M1 接配置持久化）。
// Tauri 2 中前端通过 @tauri-apps/api/core 的 invoke 调用 Rust 命令；
// 这里用全局注入（WebView 内置 __TAURI__）做兼容，避免骨架阶段引入 npm 依赖。

(function () {
  const $ = (id) => document.getElementById(id);

  function tauri() {
    return window.__TAURI__;
  }

  async function invoke(cmd, args) {
    const t = tauri();
    if (t && t.core && typeof t.core.invoke === 'function') {
      return t.core.invoke(cmd, args);
    }
    // 浏览器直接打开时无 Tauri 环境：返回默认值（方便纯前端调试）
    if (cmd === 'get_settings') return { name: '', peerIp: '', layout: 'right', clipboardEnabled: true, autostart: false };
    return undefined;
  }

  async function load() {
    const s = await invoke('get_settings');
    if (!s) return;
    $('name').value = s.name || '';
    $('peerIp').value = s.peerIp || '';
    $('layout').value = s.layout === 'left' ? 'left' : 'right';
    $('clipboardEnabled').checked = !!s.clipboardEnabled;
    $('autostart').checked = !!s.autostart;
    $('crossScreenEnabled').checked = s.crossScreenEnabled !== false;
  }

  async function save() {
    const status = $('status');
    status.textContent = '保存中…';
    try {
      await invoke('save_settings', {
        settings: {
          name: $('name').value.trim(),
          peerIp: $('peerIp').value.trim(),
          layout: $('layout').value,
          clipboardEnabled: $('clipboardEnabled').checked,
          autostart: $('autostart').checked,
          crossScreenEnabled: $('crossScreenEnabled').checked,
        },
      });
      status.textContent = '已保存 ✓';
    } catch (e) {
      status.textContent = '保存失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 2000);
  }

  // M1 自测模式：开关直接驱动后端钩子/注入器的启停
  async function toggleSelfTest() {
    const checkbox = $('selfTest');
    const status = $('status');
    try {
      const now = await invoke('set_self_test', { enabled: checkbox.checked });
      checkbox.checked = !!now;
      status.textContent = now ? '自测模式已开启（看统计/试试打字）' : '自测模式已关闭';
    } catch (e) {
      checkbox.checked = !checkbox.checked;
      status.textContent = '自测模式切换失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 2500);
  }

  // 每 300ms 刷新统计：验证捕获/注入链路是否在跑
  async function refreshStats() {
    const s = await invoke('get_self_test_stats');
    if (!s) return;
    const el = $('stats');
    if (!s.enabled) {
      el.textContent = '自测关闭';
      return;
    }
    el.textContent =
      `捕获 鼠标 ${s.capturedMouse} / 键盘 ${s.capturedKeys}` +
      ` ｜ 注入成功 ${s.injectedOk} / 失败 ${s.injectedFail}`;
  }

  // 每 500ms 刷新网络状态（连接 / 主控被控 / 跨屏中）
  async function refreshNet() {
    const s = await invoke('get_net_status');
    if (!s) return;
    const el = $('netStatus');
    if (!s.captureOk) {
      el.textContent = '输入捕获不可用：Mac 请到 系统设置→隐私与安全性→辅助功能 授权后重启';
      return;
    }
    if (!s.configured) {
      el.textContent = '网络：未配置（填对方 IP 并保存）';
      return;
    }
    if (!s.crossScreen) {
      el.textContent = '网络：已连接 ｜ 跨屏已关闭（设置里开启）';
      return;
    }
    const role = s.mode === 'sink' ? '被控' : '主控';
    const link = s.linked ? '，跨屏中' : '';
    const conn = s.connected ? '已连接' : '未连接（重连中）';
    el.textContent = `网络：${conn} ｜ ${role}${link} ｜ 收 ${s.received} / 发 ${s.sent}`;
  }

  // 注入测试：向当前聚焦窗口输入一串字符（独立验证注入链路）
  async function runTestInject() {
    const status = $('status');
    try {
      const n = await invoke('test_inject', { text: 'ruiss 123' });
      status.textContent = `已注入 ${n} 个事件（看聚焦的窗口有没有出现）`;
    } catch (e) {
      status.textContent = '注入测试失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 3000);
  }

  // 跨屏文件传输：选文件发对端
  async function pickAndSendFile() {
    const status = $('fileStatus');
    status.textContent = '选择文件中…';
    try {
      await invoke('pick_and_send_file');
      status.textContent = '已发送 ✓';
    } catch (e) {
      status.textContent = '发送失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 2500);
  }

  // 收到对端传来的文件：追加一条记录
  function logReceived(name, path) {
    const ul = $('fileLog');
    if (!ul) return;
    const li = document.createElement('li');
    li.innerHTML = `<b>收到</b> ${name}<br><span class="path">${path}</span>`;
    ul.prepend(li);
    if (ul.children.length > 20) ul.lastChild.remove();
  }

  // 监听后端 file-received 事件（对端发来文件完成时触发）
  function listenFileReceived() {
    const t = tauri();
    if (t && t.event && typeof t.event.listen === 'function') {
      t.event.listen('file-received', (e) => {
        const p = e.payload || {};
        logReceived(p.name || '文件', p.path || '');
      });
    }
  }

  document.addEventListener('DOMContentLoaded', () => {
    $('saveBtn').addEventListener('click', save);
    $('selfTest').addEventListener('change', toggleSelfTest);
    $('testInjectBtn').addEventListener('click', runTestInject);
    $('pickFileBtn').addEventListener('click', pickAndSendFile);
    listenFileReceived();
    setInterval(refreshStats, 300);
    setInterval(refreshNet, 500);
    load();
  });
})();
