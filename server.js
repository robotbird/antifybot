'use strict';

/**
 * AntifyBot 服务端
 * ----------------
 * 架构（参考同仓库 earthchat：Node + Express + Socket.IO + 原生前端，免注册匿名身份）：
 *   · 在线状态 / 文字消息 / 文件通知 —— Socket.IO（实时、自带心跳）
 *   · 文件传输 —— HTTP 分块流式上传（大文件不吃内存、断点可续），下载支持 Range
 *   · 节点发现 —— UDP 多播 beacon（lib/discovery.js），桌面节点互相自动可见
 *   · 身份 —— 客户端 localStorage 生成 UUID + 昵称，零注册
 */

const http = require('http');
const os = require('os');
const fs = require('fs');
const fsp = fs.promises;
const path = require('path');
const crypto = require('crypto');
const express = require('express');
const { Server } = require('socket.io');
const QRCode = require('qrcode');
const { Discovery } = require('./lib/discovery');

// ---------------------------------------------------------------- 配置
const PORT = parseInt(process.env.PORT || '3777', 10);
const HOSTNAME = (process.env.NODE_NAME || os.hostname() || 'antify-node').slice(0, 60);
const MAX_FILE_BYTES = (parseInt(process.env.MAX_FILE_MB || '2048', 10)) * 1024 * 1024; // 默认单文件 2GB
const MAX_CHUNK_BYTES = 8 * 1024 * 1024;              // 每块 8MB
const MAX_TEXT_LEN = 4000;                            // 单条消息上限
const HISTORY_CAP = 300;                              // 内存中保留的消息条数
const FILES_DIR = path.join(__dirname, 'files');
const MSG_RATE = { max: 25, windowMs: 10_000 };       // 每连接 10s 内最多 25 条

// ---------------------------------------------------------------- 小工具
const now = () => Date.now();
const uuid = () => crypto.randomUUID();

/** 由 id 稳定推导一个色相，作为设备主题色（服务端与客户端算法一致） */
function hueOf(id) {
  let h = 0;
  for (const ch of String(id)) h = (h * 31 + ch.codePointAt(0)) % 360;
  return h;
}

/** 清洗展示用文件名：去路径、去控制字符、限长 */
function sanitizeFileName(name) {
  const base = path.basename(String(name || 'file'))
    .replace(/[\u0000-\u001f\u007f]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  const s = base.slice(0, 120);
  return s || 'file';
}

/** 展示名可含任意字符，但要限长去控制符 */
function cleanText(s, max) {
  return String(s ?? '')
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, '')
    .trim()
    .slice(0, max);
}

function lanUrls(port) {
  const urls = [];
  for (const list of Object.values(os.networkInterfaces())) {
    for (const ni of list || []) {
      if (ni.family === 'IPv4' && !ni.internal) urls.push(`http://${ni.address}:${port}`);
    }
  }
  return urls;
}

// ---------------------------------------------------------------- 文件仓库
/** files/<id>.bin 数据 + files/<id>.json 元数据；重启后从 json 恢复（聊天记录不恢复，下载链接仍有效） */
const files = new Map(); // id -> { id, name, mime, size, sender, ts, path }

async function loadFileMetas() {
  await fsp.mkdir(FILES_DIR, { recursive: true });
  let restored = 0;
  for (const f of await fsp.readdir(FILES_DIR)) {
    if (!f.endsWith('.json')) continue;
    try {
      const meta = JSON.parse(await fsp.readFile(path.join(FILES_DIR, f), 'utf8'));
      const bin = path.join(FILES_DIR, `${meta.id}.bin`);
      const st = await fsp.stat(bin);
      if (!st.isFile() || st.size !== meta.size) continue; // 不完整的丢弃
      meta.path = bin;
      files.set(meta.id, meta);
      restored++;
    } catch (_) { /* 跳过坏元数据 */ }
  }
  if (restored) console.log(`[files] 已恢复 ${restored} 个历史文件元数据`);
}

// ---------------------------------------------------------------- 消息总线
const history = []; // { id, ts, type:'text'|'file'|'sys', from?, text?, file? }

function pushHistory(msg) {
  history.push(msg);
  if (history.length > HISTORY_CAP) history.splice(0, history.length - HISTORY_CAP);
}

// ---------------------------------------------------------------- 在线设备
const online = new Map(); // socketId -> { deviceId, name, platform, hue, since }
const deviceSockets = new Map(); // deviceId -> Set<socketId>（同设备多标签页去重用）

function presenceList() {
  const byDevice = new Map();
  for (const p of online.values()) {
    byDevice.set(p.deviceId, p); // 同设备多连接只展示一次
  }
  return [...byDevice.values()].map(({ deviceId, name, platform, hue, since }) => ({ deviceId, name, platform, hue, since }));
}

function sysMessage(text) {
  const msg = { id: uuid(), ts: now(), type: 'sys', text };
  pushHistory(msg);
  io.emit('message', msg);
}

// ---------------------------------------------------------------- HTTP / Express
const app = express();
app.disable('x-powered-by');
app.use(express.json({ limit: '200kb' }));

// 上传：初始化
const pendingUploads = new Map(); // id -> { id, name, mime, size, received, sender, tmpPath }

app.post('/api/upload/init', (req, res) => {
  const { name, size, mime } = req.body || {};
  const nBytes = Number(size);
  if (!Number.isFinite(nBytes) || nBytes < 0 || nBytes > MAX_FILE_BYTES) {
    return res.status(400).json({ error: `文件大小需在 0 ~ ${Math.floor(MAX_FILE_BYTES / 1024 / 1024)}MB 之间` });
  }
  const id = crypto.randomUUID() + '.part';
  const meta = {
    id,
    name: sanitizeFileName(name),
    mime: cleanText(mime, 100) || 'application/octet-stream',
    size: nBytes,
    received: 0,
    sender: cleanText(req.body.sender, 60) || 'unknown',
    tmpPath: path.join(FILES_DIR, `${id}.bin`),
  };
  pendingUploads.set(id, meta);
  fsp.mkdir(FILES_DIR, { recursive: true }).then(() =>
    fsp.open(meta.tmpPath, 'w').then((fh) => fh.close()));
  res.json({ fileId: id, chunkSize: MAX_CHUNK_BYTES, maxFileBytes: MAX_FILE_BYTES });
});

// 上传：数据块（顺序追加，offset 必须等于已接收字节数）
app.put(
  '/api/upload/chunk/:fileId',
  express.raw({ type: () => true, limit: MAX_CHUNK_BYTES + 64 * 1024 }),
  async (req, res) => {
    const meta = pendingUploads.get(req.params.fileId);
    if (!meta) return res.status(404).json({ error: '上传会话不存在或已完成' });
    const offset = Number(req.headers['x-offset'] || 0);
    if (!Number.isFinite(offset) || offset !== meta.received) {
      return res.status(409).json({ error: '分块乱序', expectedOffset: meta.received });
    }
    const buf = req.body;
    if (!Buffer.isBuffer(buf) || buf.length === 0) return res.status(400).json({ error: '空分块' });
    if (meta.received + buf.length > meta.size) return res.status(400).json({ error: '超出声明的文件大小' });
    try {
      await fsp.appendFile(meta.tmpPath, buf);
    } catch (err) {
      pendingUploads.delete(meta.id);
      return res.status(500).json({ error: '写入失败：' + err.message });
    }
    meta.received += buf.length;
    res.json({ received: meta.received, total: meta.size });
  }
);

// 上传：完成 → 落元数据 + 广播文件消息
app.post('/api/upload/complete/:fileId', async (req, res) => {
  const meta = pendingUploads.get(req.params.fileId);
  if (!meta) return res.status(404).json({ error: '上传会话不存在' });
  if (meta.received !== meta.size) {
    pendingUploads.delete(meta.id);
    fsp.unlink(meta.tmpPath).catch(() => {});
    return res.status(400).json({ error: `传输不完整（${meta.received}/${meta.size}），已丢弃` });
  }
  pendingUploads.delete(meta.id);

  const finalId = uuid();
  const finalPath = path.join(FILES_DIR, `${finalId}.bin`);
  try {
    await fsp.rename(meta.tmpPath, finalPath);
  } catch (err) {
    return res.status(500).json({ error: '落盘失败：' + err.message });
  }

  const senderId = cleanText(req.body?.senderId, 64);
  const onlineDev = senderId ? [...online.values()].find((p) => p.deviceId === senderId) : null;
  const sender = onlineDev
    || findDeviceByLabel(String(req.body?.sender || ''))
    || { deviceId: senderId || 'server', name: cleanText(req.body?.sender, 24) || meta.sender || 'unknown', hue: hueOf(senderId || meta.sender || 'x') };
  const fileRec = {
    id: finalId, name: meta.name, mime: meta.mime, size: meta.size,
    sender: sender.name, ts: now(), path: finalPath,
  };
  files.set(finalId, fileRec);
  await fsp.writeFile(path.join(FILES_DIR, `${finalId}.json`), JSON.stringify({
    id: finalId, name: fileRec.name, mime: fileRec.mime, size: fileRec.size,
    sender: fileRec.sender, ts: fileRec.ts,
  }, null, 2));

  const msg = {
    id: uuid(), ts: now(), type: 'file',
    from: { deviceId: sender.deviceId, name: sender.name, hue: sender.hue },
    file: { id: finalId, name: fileRec.name, mime: fileRec.mime, size: fileRec.size, url: `/api/files/${finalId}` },
  };
  pushHistory(msg);
  io.emit('message', msg);
  res.json({ fileId: finalId, url: msg.file.url, messageId: msg.id });
});

// 允许内联预览的 MIME 白名单（其余一律 attachment，避免浏览器执行 HTML 伪装文件）
const INLINE_MIMES = new Set([
  'image/jpeg', 'image/png', 'image/gif', 'image/webp', 'image/bmp', 'image/avif',
  'video/mp4', 'video/webm', 'audio/mpeg', 'audio/wav', 'audio/ogg', 'audio/mp4', 'application/pdf',
]);

// 下载 / 预览（支持 Range，移动端视频拖动进度条依赖它）
app.get('/api/files/:fileId', (req, res) => {
  const rec = files.get(req.params.fileId);
  if (!rec) return res.status(404).json({ error: '文件不存在（服务可能已重启且元数据丢失）' });
  res.setHeader('X-Content-Type-Options', 'nosniff');
  res.setHeader('Accept-Ranges', 'bytes');
  const asciiName = rec.name.replace(/[^\x20-\x7e]/g, '_').replace(/["\\]/g, '_') || 'file';
  const dispo = INLINE_MIMES.has(rec.mime) ? 'inline' : 'attachment';
  res.setHeader('Content-Disposition', `${dispo}; filename="${asciiName}"; filename*=UTF-8''${encodeURIComponent(rec.name)}`);

  fs.stat(rec.path, (err, st) => {
    if (err || !st.isFile()) return res.status(404).json({ error: '文件已丢失' });
    const total = st.size;
    const range = req.headers.range;
    if (range) {
      const m = /^bytes=(\d*)-(\d*)$/.exec(String(range).trim());
      if (!m || (m[1] === '' && m[2] === '')) return res.status(416).end();
      let start = m[1] === '' ? total - Number(m[2]) : Number(m[1]);
      let end = m[2] === '' ? total - 1 : Number(m[2]);
      if (Number.isNaN(start) || Number.isNaN(end) || start > end || start < 0 || end >= total) {
        return res.status(416).setHeader('Content-Range', `bytes */${total}`), res.end();
      }
      res.status(206);
      res.setHeader('Content-Range', `bytes ${start}-${end}/${total}`);
      res.setHeader('Content-Length', String(end - start + 1));
      if (rec.mime) res.setHeader('Content-Type', rec.mime);
      return fs.createReadStream(rec.path, { start, end }).pipe(res);
    }
    if (rec.mime) res.setHeader('Content-Type', rec.mime);
    res.setHeader('Content-Length', String(total));
    fs.createReadStream(rec.path).pipe(res);
  });
});

// 连接信息：局域网地址 + 二维码
let lobbyCache = null;
app.get('/api/lobby', async (req, res) => {
  const host = req.headers.host || `localhost:${PORT}`;
  const proto = req.socket.encrypted ? 'https' : 'http';
  const primary = `${proto}://${host}`;
  if (!lobbyCache || lobbyCache.primary !== primary) {
    lobbyCache = {
      version: require('./package.json').version,
      serverName: HOSTNAME,
      primary,
      urls: lanUrls(PORT),
      qr: await QRCode.toDataURL(primary, { width: 280, margin: 1 }),
    };
  }
  res.json(lobbyCache);
});

// 局域网内发现的其他 AntifyBot 节点
app.get('/api/nodes', (req, res) => {
  res.json({ self: { name: HOSTNAME, port: PORT }, peers: discovery.peerList() });
});

app.use(express.static(path.join(__dirname, 'public'), { index: 'index.html', maxAge: '1h' }));
app.use('/api', (req, res) => res.status(404).json({ error: 'not found' }));

// ---------------------------------------------------------------- Socket.IO
const server = http.createServer(app);
const io = new Server(server, { maxHttpBufferSize: 128 * 1024, pingInterval: 10_000, pingTimeout: 20_000 });

function findDeviceByLabel(label) {
  for (const p of online.values()) {
    if (p.deviceId === label || p.name === label) return p;
  }
  return null;
}

io.on('connection', (socket) => {
  let me = null;
  let msgTimes = [];

  socket.emit('hello-ack', { serverName: HOSTNAME, history, presence: presenceList() });

  socket.on('hello', (info, ack) => {
    const prev = me;
    me = {
      deviceId: cleanText(info?.deviceId, 64) || uuid(),
      name: cleanText(info?.name, 24) || '匿名蚂蚁',
      platform: cleanText(info?.platform, 30) || 'unknown',
      hue: hueOf(cleanText(info?.deviceId, 64) || socket.id),
      since: prev?.since || now(),
    };
    online.set(socket.id, me);
    if (!deviceSockets.has(me.deviceId)) deviceSockets.set(me.deviceId, new Set());
    deviceSockets.get(me.deviceId).add(socket.id);

    if (!prev) {
      sysMessage(`${me.name}（${me.platform}）加入了`);
    } else if (prev.name !== me.name) {
      sysMessage(`${prev.name} 改名为 ${me.name}`);
    }
    io.emit('presence', presenceList());
    if (typeof ack === 'function') ack({ ok: true, hue: me.hue });
  });

  socket.on('message', (payload, ack) => {
    if (!me) return;
    const text = cleanText(payload?.text, MAX_TEXT_LEN);
    if (!text) return;

    // 简单滑动窗口限流
    const t = now();
    msgTimes = msgTimes.filter((x) => t - x < MSG_RATE.windowMs);
    if (msgTimes.length >= MSG_RATE.max) {
      if (typeof ack === 'function') ack({ ok: false, error: '发送太频繁，歇一会儿' });
      return;
    }
    msgTimes.push(t);

    const msg = { id: uuid(), ts: t, type: 'text', from: { deviceId: me.deviceId, name: me.name, hue: me.hue }, text };
    pushHistory(msg);
    io.emit('message', msg);
    if (typeof ack === 'function') ack({ ok: true, id: msg.id });
  });

  // 打字指示：仅在"有内容变化"时转发，客户端自带节流
  socket.on('typing', (payload) => {
    if (!me) return;
    socket.broadcast.emit('peer-typing', { deviceId: me.deviceId, name: me.name, on: !!payload?.on });
  });

  socket.on('disconnect', () => {
    if (!me) return;
    online.delete(socket.id);
    const set = deviceSockets.get(me.deviceId);
    if (set) {
      set.delete(socket.id);
      if (set.size === 0) {
        deviceSockets.delete(me.deviceId);
        sysMessage(`${me.name} 离开了`);
      }
    }
    io.emit('presence', presenceList());
  });
});

// ---------------------------------------------------------------- 节点发现
const discovery = new Discovery({ name: HOSTNAME, port: PORT });
discovery.onPeer = (peer, event) => {
  if (event === 'up') console.log(`[discovery] 🐜 发现节点 ${peer.name} (${peer.addr}:${peer.port})`);
  if (event === 'down') console.log(`[discovery] ⚠️  节点下线 ${peer.name}`);
};

// ---------------------------------------------------------------- 启动
(async () => {
  await loadFileMetas();
  discovery.start();

  server.listen(PORT, '0.0.0.0', () => {
    const urls = lanUrls(PORT);
    console.log('');
    console.log('  🐜 AntifyBot 局域网快传 已启动');
    console.log(`  · 本机   http://localhost:${PORT}`);
    for (const u of urls) console.log(`  · 局域网 ${u}   ← 手机/其他电脑用这个`);
    console.log('');
    console.log('  提示：手机浏览器打开局域网地址即可加入对话；');
    console.log('        其他电脑安装 Node 后运行 `npm run discover` 可自动发现本节点。');
    console.log('');
  });
})();

process.on('SIGINT', () => { discovery.stop(); server.close(() => process.exit(0)); setTimeout(() => process.exit(0), 1500).unref(); });
process.on('SIGTERM', () => { discovery.stop(); server.close(() => process.exit(0)); setTimeout(() => process.exit(0), 1500).unref(); });
