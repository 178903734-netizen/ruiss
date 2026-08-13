(function () {
  const $ = (id) => document.getElementById(id);
  const defaults = {
    mouseBackShortcut: nativeShortcut('Digit1', { ctrl: true }),
    mouseForwardShortcut: nativeShortcut('Digit2', { ctrl: true }),
    mouseMiddleShortcut: nativeShortcut('Digit3', { ctrl: true }),
  };
  const bindings = { ...defaults };
  const transfers = new Map();
  let recording = null;

  function nativeShortcut(key, modifiers = {}) {
    return {
      key,
      modifiers: {
        ctrl: !!modifiers.ctrl,
        alt: !!modifiers.alt,
        shift: !!modifiers.shift,
        superKey: !!modifiers.superKey,
      },
    };
  }

  function tauri() {
    return window.__TAURI__;
  }

  async function invoke(cmd, args) {
    const t = tauri();
    if (t && t.core && typeof t.core.invoke === 'function') {
      return t.core.invoke(cmd, args);
    }
    if (cmd === 'get_settings') {
      return {
        name: '', peerIp: '', layout: 'right', clipboardEnabled: true,
        autostart: false, crossScreenEnabled: true, receiveDir: '', ...defaults,
      };
    }
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
    $('receiveDir').value = s.receiveDir || '';
    for (const name of Object.keys(defaults)) {
      bindings[name] = s[name] === undefined ? defaults[name] : s[name];
    }
    renderBindings();
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
          mouseBackShortcut: bindings.mouseBackShortcut,
          mouseForwardShortcut: bindings.mouseForwardShortcut,
          mouseMiddleShortcut: bindings.mouseMiddleShortcut,
          receiveDir: $('receiveDir').value.trim(),
        },
      });
      status.textContent = '已保存 ✓';
    } catch (e) {
      status.textContent = '保存失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 2200);
  }

  function keyFromCode(code) {
    if (/^Key[A-Z]$/.test(code)) return code.slice(3);
    if (/^Digit[0-9]$/.test(code)) return code;
    if (/^F(?:[1-9]|1[0-2])$/.test(code)) return code;
    const map = {
      Enter: 'Enter', NumpadEnter: 'Enter', Space: 'Space', Backspace: 'Backspace',
      Tab: 'Tab', Escape: 'Esc', ArrowUp: 'ArrowUp', ArrowDown: 'ArrowDown',
      ArrowLeft: 'ArrowLeft', ArrowRight: 'ArrowRight', Delete: 'Delete',
      Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
      Insert: 'Insert', CapsLock: 'CapsLock', Comma: 'Comma', Period: 'Period',
      Slash: 'Slash', Semicolon: 'Semicolon', Quote: 'Quote', BracketLeft: 'LBracket',
      BracketRight: 'RBracket', Backslash: 'Backslash', Minus: 'Minus',
      Equal: 'Equals', Backquote: 'Backtick',
    };
    return map[code] || null;
  }

  const modifierCodes = new Set([
    'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight',
    'MetaLeft', 'MetaRight',
  ]);

  function startRecording(name) {
    recording = name;
    renderBindings();
  }

  function stopRecording() {
    recording = null;
    renderBindings();
  }

  function captureShortcut(event) {
    if (!recording) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat || modifierCodes.has(event.code)) return;
    if (event.code === 'Escape' && !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) {
      stopRecording();
      return;
    }
    const key = keyFromCode(event.code);
    if (!key) {
      $('status').textContent = `暂不支持这个键：${event.code}`;
      return;
    }
    bindings[recording] = nativeShortcut(key, {
      ctrl: event.ctrlKey,
      alt: event.altKey,
      shift: event.shiftKey,
      superKey: event.metaKey,
    });
    recording = null;
    renderBindings();
  }

  function displayKey(key) {
    if (key && key.startsWith('Digit')) return key.slice(5);
    const map = {
      Esc: 'Esc', Space: 'Space', ArrowLeft: '←', ArrowRight: '→',
      ArrowUp: '↑', ArrowDown: '↓', Backspace: 'Backspace', PageUp: 'Page Up',
      PageDown: 'Page Down', LBracket: '[', RBracket: ']', Backslash: '\\',
      Comma: ',', Period: '.', Slash: '/', Semicolon: ';', Quote: "'",
      Minus: '-', Equals: '=', Backtick: '`',
    };
    return map[key] || key;
  }

  function displayShortcut(shortcut) {
    if (!shortcut) return '已清除';
    const parts = [];
    if (shortcut.modifiers.ctrl) parts.push('Ctrl');
    if (shortcut.modifiers.alt) parts.push('Alt');
    if (shortcut.modifiers.shift) parts.push('Shift');
    if (shortcut.modifiers.superKey) parts.push('Win');
    parts.push(displayKey(shortcut.key));
    return parts.join(' + ');
  }

  function renderBindings() {
    document.querySelectorAll('.shortcut-record').forEach((button) => {
      const name = button.dataset.binding;
      button.classList.toggle('recording', recording === name);
      button.textContent = recording === name ? '请按快捷键…（Esc 取消）' : displayShortcut(bindings[name]);
    });
  }

  async function refreshNet() {
    const s = await invoke('get_net_status');
    if (!s) return;
    const el = $('netStatus');
    const version = $('versionStatus');
    const peerVersion = s.peerVersion || '未知';
    const peerProtocol = s.peerProtocolVersion ?? '未知';
    version.classList.remove('synced', 'mismatch');
    if (!s.connected) {
      version.textContent = `版本：本机 v${s.localVersion} / 协议 ${s.localProtocolVersion} · 对端未连接`;
    } else if (s.versionsMatch) {
      version.textContent = `版本：本机 v${s.localVersion} / 对端 v${peerVersion} · 协议 ${s.localProtocolVersion} · 已同步`;
      version.classList.add('synced');
    } else {
      version.textContent = `版本不一致：本机 v${s.localVersion} / 协议 ${s.localProtocolVersion} · 对端 v${peerVersion} / 协议 ${peerProtocol}`;
      version.classList.add('mismatch');
    }
    if (!s.captureOk) {
      el.textContent = '输入捕获不可用：Mac 请在“隐私与安全性 → 辅助功能”授权后重启';
      return;
    }
    if (!s.configured) {
      el.textContent = '网络：未配置（填写对方 IP 并保存）';
      return;
    }
    const role = s.mode === 'sink' ? '被控' : '主控';
    const link = s.linked ? '，跨屏中' : '';
    const conn = s.connected ? '已连接' : '未连接（重连中）';
    const cross = s.crossScreen ? '' : '，跨屏已关闭';
    el.textContent = `网络：${conn} · ${role}${link}${cross} · 收 ${s.received} / 发 ${s.sent}`;
  }

  async function pickAndSendFile() {
    const status = $('fileStatus');
    status.textContent = '选择文件中…';
    try {
      const id = await invoke('pick_and_send_file');
      status.textContent = id ? '已加入传输任务' : '';
    } catch (e) {
      status.textContent = '发送失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 2500);
  }

  async function pickAndSendFolder() {
    const status = $('fileStatus');
    status.textContent = '选择文件夹中…';
    try {
      const id = await invoke('pick_and_send_folder');
      status.textContent = id ? '已加入传输任务' : '';
    } catch (e) {
      status.textContent = '发送失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 2500);
  }

  async function pickReceiveDirectory() {
    try {
      const path = await invoke('pick_receive_directory');
      if (path) $('receiveDir').value = path;
    } catch (e) {
      $('fileStatus').textContent = '选择目录失败：' + e;
    }
  }

  function formatBytes(bytes) {
    const value = Number(bytes || 0);
    if (value < 1024) return value + ' B';
    if (value < 1024 * 1024) return (value / 1024).toFixed(1) + ' KB';
    if (value < 1024 * 1024 * 1024) return (value / 1024 / 1024).toFixed(1) + ' MB';
    return (value / 1024 / 1024 / 1024).toFixed(2) + ' GB';
  }

  function formatDuration(seconds) {
    if (!Number.isFinite(seconds) || seconds < 0) return '';
    if (seconds < 60) return `${Math.max(1, Math.round(seconds))} 秒`;
    if (seconds < 3600) return `${Math.ceil(seconds / 60)} 分钟`;
    return `${Math.floor(seconds / 3600)} 小时 ${Math.ceil((seconds % 3600) / 60)} 分`;
  }

  function statusText(status) {
    return {
      preparing: '正在整理', transferring: '传输中', completed: '已完成',
      failed: '失败', cancelled: '已取消',
    }[status] || status;
  }

  function renderTransfers() {
    const list = $('transferList');
    list.replaceChildren();
    Array.from(transfers.values()).reverse().forEach((task) => {
      const item = document.createElement('div');
      item.className = `transfer-item ${task.status || ''}`;
      const head = document.createElement('div');
      head.className = 'transfer-head';
      const name = document.createElement('b');
      name.className = 'transfer-name';
      name.title = task.name || '';
      name.textContent = task.name || '文件传输';
      const direction = document.createElement('span');
      direction.className = 'transfer-direction';
      direction.textContent = task.direction === 'receive' ? '接收' : '发送';
      head.append(name, direction);

      const bar = document.createElement('div');
      bar.className = 'transfer-bar';
      const fill = document.createElement('span');
      const percent = task.total > 0 ? Math.min(100, task.transferred / task.total * 100) : (task.status === 'completed' ? 100 : 0);
      fill.style.width = percent + '%';
      bar.append(fill);

      const meta = document.createElement('div');
      meta.className = 'transfer-meta';
      const progress = document.createElement('span');
      const speed = task.speed > 0 ? ` · ${formatBytes(task.speed)}/s` : '';
      const eta = task.eta > 0 && task.status === 'transferring' ? ` · 剩余 ${formatDuration(task.eta)}` : '';
      progress.textContent = `${formatBytes(task.transferred)} / ${formatBytes(task.total)}${speed}${eta}`;
      const state = document.createElement('span');
      const files = task.filesTotal ? ` · ${task.filesDone || 0}/${task.filesTotal} 个` : '';
      state.textContent = statusText(task.status) + files;
      meta.append(progress, state);
      item.append(head, bar, meta);

      if (task.error) {
        const error = document.createElement('div');
        error.className = 'transfer-error';
        error.textContent = task.error;
        item.append(error);
      }
      if (task.direction === 'send' && ['preparing', 'transferring'].includes(task.status)) {
        const actions = document.createElement('div');
        actions.className = 'transfer-actions';
        const cancel = document.createElement('button');
        cancel.className = 'transfer-action';
        cancel.textContent = '取消';
        cancel.addEventListener('click', async () => {
          try { await invoke('cancel_file_transfer', { id: task.id }); } catch (_) {}
        });
        actions.append(cancel);
        item.append(actions);
      } else if (task.direction === 'send' && ['failed', 'cancelled'].includes(task.status)) {
        const actions = document.createElement('div');
        actions.className = 'transfer-actions';
        const retry = document.createElement('button');
        retry.className = 'transfer-action';
        retry.textContent = '重试';
        retry.addEventListener('click', async () => {
          try { await invoke('retry_file_transfer', { id: task.id }); } catch (e) { $('fileStatus').textContent = '重试失败：' + e; }
        });
        actions.append(retry);
        item.append(actions);
      }
      list.append(item);
    });
  }

  function onTransferUpdate(payload) {
    if (!payload || !payload.id) return;
    const previous = transfers.get(payload.id) || {};
    const now = performance.now();
    let speed = Number(previous.speed || 0);
    const elapsed = (now - Number(previous.sampleAt || now)) / 1000;
    const delta = Number(payload.transferred || 0) - Number(previous.sampleBytes || 0);
    if (payload.status === 'transferring' && elapsed >= 0.08 && delta >= 0) {
      const instant = delta / elapsed;
      speed = speed > 0 ? speed * 0.72 + instant * 0.28 : instant;
    }
    if (payload.status !== 'transferring') speed = 0;
    const remaining = Math.max(0, Number(payload.total || 0) - Number(payload.transferred || 0));
    const eta = speed > 1 ? remaining / speed : 0;
    transfers.set(payload.id, {
      ...previous, ...payload, speed, eta,
      sampleAt: now, sampleBytes: Number(payload.transferred || 0),
    });
    while (transfers.size > 30) transfers.delete(transfers.keys().next().value);
    renderTransfers();
  }

  function listenFileReceived() {
    const t = tauri();
    if (t && t.event && typeof t.event.listen === 'function') {
      t.event.listen('file-received', (event) => {
        const p = event.payload || {};
        $('fileStatus').textContent = `已收到 ${p.name || '文件'}`;
        setTimeout(() => ($('fileStatus').textContent = ''), 2500);
      });
      t.event.listen('file-transfer-update', (event) => onTransferUpdate(event.payload || {}));
    }
  }

  document.addEventListener('DOMContentLoaded', () => {
    $('saveBtn').addEventListener('click', save);
    const pickerMenu = $('pickTransferMenu');
    $('pickTransferBtn').addEventListener('click', (event) => {
      event.stopPropagation();
      pickerMenu.hidden = !pickerMenu.hidden;
    });
    $('pickFileBtn').addEventListener('click', () => { pickerMenu.hidden = true; pickAndSendFile(); });
    $('pickFolderBtn').addEventListener('click', () => { pickerMenu.hidden = true; pickAndSendFolder(); });
    document.addEventListener('click', () => { pickerMenu.hidden = true; });
    $('clearTransfersBtn').addEventListener('click', () => {
      for (const [id, task] of transfers) {
        if (!['preparing', 'transferring'].includes(task.status)) transfers.delete(id);
      }
      renderTransfers();
      $('fileStatus').textContent = transfers.size ? '进行中的任务已保留' : '传输记录已清空';
      setTimeout(() => ($('fileStatus').textContent = ''), 1800);
    });
    $('pickReceiveDirBtn').addEventListener('click', pickReceiveDirectory);
    $('clearReceiveDirBtn').addEventListener('click', () => { $('receiveDir').value = ''; });
    document.querySelectorAll('.shortcut-record').forEach((button) => {
      button.addEventListener('click', () => startRecording(button.dataset.binding));
    });
    document.querySelectorAll('.shortcut-clear').forEach((button) => {
      button.addEventListener('click', () => {
        bindings[button.dataset.binding] = null;
        stopRecording();
      });
    });
    window.addEventListener('keydown', captureShortcut, true);
    listenFileReceived();
    setInterval(refreshNet, 500);
    load();
  });
})();
