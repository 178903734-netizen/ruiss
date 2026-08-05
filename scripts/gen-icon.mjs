// 生成 Ruiss 托盘图标 PNG（64x64，RGBA）。
// 画法：深色圆角背景 + 两个浅色"屏幕"方块（象征两台电脑共享一套键鼠）。
// 用法：node scripts/gen-icon.mjs
// 输出：src-tauri/icons/icon.png
import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SIZE = 64;
const OUT = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'icons', 'icon.png');

// ---- 画像素 ----
const px = new Uint8Array(SIZE * SIZE * 4);
const set = (x, y, r, g, b, a) => {
  if (x < 0 || y < 0 || x >= SIZE || y >= SIZE) return;
  const i = (y * SIZE + x) * 4;
  px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = a;
};

// 背景：深蓝渐变
for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    const t = y / SIZE;
    set(x, y, Math.round(24 + t * 18), Math.round(32 + t * 22), Math.round(58 + t * 34), 255);
  }
}

// 圆角矩形
const inRoundRect = (x, y, rx, ry, rw, rh, rad) => {
  const cx = Math.max(rx + rad, Math.min(x, rx + rw - rad));
  const cy = Math.max(ry + rad, Math.min(y, ry + rh - rad));
  return (x - cx) ** 2 + (y - cy) ** 2 <= rad * rad;
};

// 左侧屏幕（浅蓝）
for (let y = 0; y < SIZE; y++)
  for (let x = 0; x < SIZE; x++)
    if (inRoundRect(x, y, 8, 14, 20, 30, 4))
      if (px[(y * SIZE + x) * 4 + 3] === 255)
        set(x, y, 96, 165, 250, 255);
// 右侧屏幕（浅青，比左侧偏上一点，"滑过去"的感觉）
for (let y = 0; y < SIZE; y++)
  for (let x = 0; x < SIZE; x++)
    if (inRoundRect(x, y, 34, 18, 22, 30, 4))
      if (px[(y * SIZE + x) * 4 + 3] === 255)
        set(x, y, 78, 205, 196, 255);
// 屏幕之间的"滑轨"圆点
for (let y = 0; y < SIZE; y++)
  for (let x = 0; x < SIZE; x++)
    if ((x - 30) ** 2 + (y - 32) ** 2 <= 4 ** 2)
      set(x, y, 255, 255, 255, 255);

// ---- 编码 PNG（RGBA8, 非隔行）----
const rowLen = SIZE * 4 + 1;
const raw = Buffer.alloc(rowLen * SIZE);
for (let y = 0; y < SIZE; y++) {
  raw[y * rowLen] = 0; // filter: None
  Buffer.from(px.buffer, y * SIZE * 4, SIZE * 4).copy(raw, y * rowLen + 1);
}
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8;  // bit depth
ihdr[9] = 6;  // color type RGBA
ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;

const chunk = (type, data) => {
  const t = Buffer.from(type, 'ascii');
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([t, data])), 0);
  return Buffer.concat([len, t, data, crc]);
};

const crcTable = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();
function crc32(buf) {
  let c = -1;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', ihdr),
  chunk('IDAT', deflateSync(raw, { level: 9 })),
  chunk('IEND', Buffer.alloc(0)),
]);

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, png);
console.log('written:', OUT, png.length, 'bytes');
