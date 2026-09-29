// 终端面板固定任务的统一入口：node scripts/run.mjs <命令> [参数...]
// 只为把 Rust 工具链补进 PATH（见 toolchain.mjs），命令本身不动。
import { spawnSync } from 'node:child_process';
import { taskEnv } from './toolchain.mjs';

const parts = process.argv.slice(2);
if (parts.length === 0) {
  console.error('用法：node scripts/run.mjs <命令> [参数...]');
  process.exit(2);
}

const result = spawnSync(parts.join(' '), { stdio: 'inherit', shell: true, env: taskEnv() });
process.exit(result.status ?? 1);
