(function () {
  const $ = (id) => document.getElementById(id);
  const defaults = {
    mouseBackShortcut: nativeShortcut('Digit1', { ctrl: true }),
    mouseForwardShortcut: nativeShortcut('Digit2', { ctrl: true }),
    mouseMiddleShortcut: nativeShortcut('Digit3', { ctrl: true }),
  };
  const bindings = { ...defaults };
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
        autostart: false, crossScreenEnabled: true, ...defaults,
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
      await invoke('pick_and_send_file');
      status.textContent = '已发送 ✓';
    } catch (e) {
      status.textContent = '发送失败：' + e;
    }
    setTimeout(() => (status.textContent = ''), 2500);
  }

  function logReceived(name, path) {
    const ul = $('fileLog');
    const li = document.createElement('li');
    const title = document.createElement('b');
    const pathEl = document.createElement('span');
    title.textContent = `收到 ${name}`;
    pathEl.className = 'path';
    pathEl.textContent = path;
    li.append(title, document.createElement('br'), pathEl);
    ul.prepend(li);
    if (ul.children.length > 20) ul.lastChild.remove();
  }

  function listenFileReceived() {
    const t = tauri();
    if (t && t.event && typeof t.event.listen === 'function') {
      t.event.listen('file-received', (event) => {
        const p = event.payload || {};
        logReceived(p.name || '文件', p.path || '');
      });
    }
  }

  document.addEventListener('DOMContentLoaded', () => {
    $('saveBtn').addEventListener('click', save);
    $('pickFileBtn').addEventListener('click', pickAndSendFile);
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
