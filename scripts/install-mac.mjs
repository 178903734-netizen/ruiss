// 把打好的 Ruiss.app 安装（替换）到 /Applications：只在 macOS 上运行。
//
// 只换应用本体：~/Library 里的配置、配对信息都不动；因为签名证书和 bundle id 固定，
// 系统的辅助功能 / 输入监控授权也不用重新设置（见 mac-signing.mjs）。
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { taskEnv } from './toolchain.mjs';
import { verifyMacApp } from './mac-signing.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const APP_NAME = 'Ruiss';
const BINARY = 'ruiss';
const SOURCE = path.join(root, 'src-tauri', 'target', 'release', 'bundle', 'macos', `${APP_NAME}.app`);
const DESTINATION = path.join('/Applications', `${APP_NAME}.app`);

if (process.platform !== 'darwin') {
  console.error('[install:mac] 该任务只支持 macOS');
  process.exit(1);
}

const env = taskEnv();
const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

const source = verifyMacApp(SOURCE, env);
if (!source.ok) {
  console.error(`[install:mac] 待安装的应用签名不合要求（先跑 npm run package:mac）：`);
  for (const problem of source.problems) console.error(`  - ${problem}`);
  process.exit(1);
}

// 替换前先让旧应用退出，否则覆盖出来的包会缺文件或处于运行中状态
const isRunning = () => spawnSync('pgrep', ['-x', BINARY], { encoding: 'utf8' }).status === 0;

if (isRunning()) {
  console.log(`[install:mac] 退出正在运行的 ${APP_NAME}…`);
  spawnSync('osascript', ['-e', `tell application "${APP_NAME}" to quit`], { env });
  for (let i = 0; i < 15 && isRunning(); i += 1) sleep(1000);
  if (isRunning()) {
    spawnSync('pkill', ['-TERM', '-x', BINARY], { env });
    for (let i = 0; i < 5 && isRunning(); i += 1) sleep(1000);
  }
  if (isRunning()) {
    console.error(`[install:mac] ${APP_NAME} 仍在运行，请手动退出后重试`);
    process.exit(1);
  }
}

// 旧应用先挪走而不是直接删：复制失败时还能恢复
const backupDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ruiss-install-'));
const backup = path.join(backupDir, `${APP_NAME}.app`);
const hadOldApp = fs.existsSync(DESTINATION);

if (hadOldApp) {
  try {
    fs.renameSync(DESTINATION, backup);
  } catch (error) {
    console.error(`[install:mac] 无法移动旧应用（可能需要在终端手动执行）：${error.message}`);
    process.exit(1);
  }
}

try {
  fs.cpSync(SOURCE, DESTINATION, { recursive: true });
} catch (error) {
  if (hadOldApp) {
    try {
      fs.rmSync(DESTINATION, { recursive: true, force: true });
      fs.renameSync(backup, DESTINATION);
      console.error('[install:mac] 复制失败，已恢复原应用');
    } catch (restoreError) {
      console.error(`[install:mac] 复制失败且恢复失败，旧应用在：${backup}`);
      console.error(restoreError.message);
    }
  }
  console.error(`[install:mac] 复制失败：${error.message}`);
  console.error('  对 /Applications 没有写权限时可以手动执行：');
  console.error(`    sudo rm -rf "${DESTINATION}" && sudo cp -R "${SOURCE}" "${DESTINATION}"`);
  process.exit(1);
}

// 本地构建一般不带隔离属性；从 dmg 拷贝过来时可能有，清掉避免首次打开被拦
spawnSync('xattr', ['-dr', 'com.apple.quarantine', DESTINATION], { env });

const installed = verifyMacApp(DESTINATION, env);
if (!installed.ok) {
  console.error('[install:mac] 安装后的签名与源不一致：');
  for (const problem of installed.problems) console.error(`  - ${problem}`);
  process.exit(1);
}

fs.rmSync(backupDir, { recursive: true, force: true });

console.log(`[install:mac] 已替换 ${DESTINATION}`);
if (hadOldApp) console.log(`  旧版本已备份并在成功后清理；${APP_NAME}.app 的配置和系统授权保持不变`);
console.log(`  ${installed.requirement}`);
spawnSync('open', [DESTINATION], { env });
console.log(`[install:mac] 已启动 ${APP_NAME}`);
