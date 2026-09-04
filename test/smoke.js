'use strict';

/**
 * 冒烟测试：真实拉起 server.js，模拟两台设备完成
 * 连接 / 在线状态 / 文字消息 / 分块上传 / 下载 / Range / 重启恢复 / UDP 发现。
 * 运行：npm test
 */

const assert = require('assert');
const { spawn } = require('child_process');
const crypto = require('crypto');
const path = require('path');
const { io } = require('socket.io-client');
const { Discovery } = require('../lib/discovery');

const PORT = 3900 + Math.floor(Math.random() * 200); // 随机端口，避免与残留进程冲突
const BASE = `http://127.0.0.1:${PORT}`;
const ROOT = path.join(__dirname, '..');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sha = (buf) => crypto.createHash('sha256').update(buf).digest('hex');

let passed = 0;
function ok(name) { passed++; console.log(`  ✅ ${name}`); }

function startServer() {
  const child = spawn(process.execPath, ['server.js'], {
    cwd: ROOT,
    env: { ...process.env, PORT: String(PORT) },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', () => {});
  child.stderr.on('data', (d) => process.stderr.write(`[server] ${d}`));
  return child;
}

async function waitLobby(base = BASE, tries = 60) {
  for (let i = 0; i < tries; i++) {
    try { return await (await fetch(`${base}/api/lobby`)).json(); } catch (_) { await sleep(250); }
  }
  throw new Error('server did not start');
}

/** 等到某个 socket 收到满足条件的 message */
function nextMessage(sock, pred) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('等待消息超时')), 8000);
    sock.on('message', function onMsg(msg) {
      if (!pred || pred(msg)) { clearTimeout(timer); sock.off('message', onMsg); resolve(msg); }
    });
  });
}

function connectDevice(name) {
  const sock = io(BASE, { transports: ['websocket'] });
  const ready = new Promise((resolve) => sock.on('hello-ack', resolve));
  sock.on('connect', () => sock.emit('hello', {
    deviceId: 'dev-' + name, name, platform: 'test',
  }));
  return { sock, ready };
}

async function uploadRaw(name, buf, mime, sender) {
  const init = await (await fetch(`${BASE}/api/upload/init`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name, size: buf.length, mime, sender }),
  })).json();
  const chunk = 1024 * 1024;
  for (let off = 0; off < buf.length; off += chunk) {
    const part = buf.subarray(off, Math.min(off + chunk, buf.length));
    const res = await fetch(`${BASE}/api/upload/chunk/${init.fileId}`, {
      method: 'PUT',
      headers: { 'content-type': 'application/octet-stream', 'x-offset': String(off) },
      body: part,
    });
    assert.equal(res.status, 200, `chunk ${off} 上传失败`);
  }
  return (await (await fetch(`${BASE}/api/upload/complete/${init.fileId}`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ sender, senderId: 'dev-A' }),
  })).json());
}

(async () => {
  console.log('\n🐜 AntifyBot 冒烟测试\n');
  let server = startServer();
  // 任何退出路径都带走子进程
  const cleanup = () => { try { server.kill('SIGKILL'); } catch (_) {} };
  process.on('exit', cleanup);
  process.on('SIGINT', () => { cleanup(); process.exit(1); });

  const lobby = await waitLobby();
  assert.ok(lobby.qr.startsWith('data:image/png;base64,'), 'QR 应为 dataURL');
  assert.ok(lobby.primary.includes(String(PORT)));
  ok('服务启动 + /api/lobby（含二维码）');

  // ---- 静态页
  const page = await (await fetch(BASE + '/')).text();
  assert.ok(page.includes('AntifyBot'), '首页应包含品牌名');
  ok('静态首页可访问');

  // ---- 两台设备上线
  const A = connectDevice('设备A');
  const B = connectDevice('设备B');
  const bSeesJoin = nextMessage(B.sock, (m) => m.type === 'sys' && m.text.includes('设备B'));
  await Promise.all([A.ready, B.ready]);
  await bSeesJoin;
  ok('两台设备上线 + 系统消息广播');

  // ---- 文字消息 + XSS 载荷原样往返（渲染端用 textContent）
  const evil = '<img src=x onerror=alert(1)> 你好 & 蚂蚁';
  const bGotText = nextMessage(B.sock, (m) => m.type === 'text' && m.text === evil);
  const ack = await new Promise((res) => A.sock.timeout(3000).emit('message', { text: evil }, (err, resp) => res(resp || { ok: false, error: err?.message })));
  assert.equal(ack.ok, true);
  assert.equal((await bGotText).text, evil);
  ok('文字消息广播（含 HTML 特殊字符原样往返）');

  // ---- 限流
  let lastAck = { ok: true };
  for (let i = 0; i < 30; i++) {
    lastAck = await new Promise((res) => A.sock.timeout(2000).emit('message', { text: 'spam ' + i }, (err, resp) => res(resp || { ok: false, error: err?.message })));
  }
  assert.equal(lastAck.ok, false, '第 30 条应被限流');
  ok('消息限流生效');

  // ---- 大文件分块上传（9.3MB → 10 块）
  const payload = crypto.randomBytes(9.3 * 1024 * 1024 | 0);
  const bGotFile = nextMessage(B.sock, (m) => m.type === 'file' && m.file.name === '蚁群大礼包.bin');
  const up = await uploadRaw('蚁群大礼包.bin', payload, 'application/octet-stream', '设备A');
  const fileMsg = await bGotFile;
  assert.equal(fileMsg.file.size, payload.length);
  assert.equal(fileMsg.from.name, '设备A', '文件消息应带发送者');
  ok(`分块上传 ${payload.length} 字节（${Math.ceil(payload.length / 1048576)} 块）+ 文件消息广播`);

  // ---- 下载完整性与中文文件名
  const dl = await fetch(`${BASE}${up.url}`);
  assert.equal(dl.status, 200);
  assert.equal(decodeURIComponent(dl.headers.get('content-disposition').match(/filename\*=UTF-8''(.+?)(;|$)/)[1]), '蚁群大礼包.bin');
  const dlBuf = Buffer.from(await dl.arrayBuffer());
  assert.equal(sha(dlBuf), sha(payload), '下载内容哈希应一致');
  ok('下载内容一致（SHA-256）+ UTF-8 文件名');

  // ---- Range（移动端视频拖进度依赖）
  const part = await fetch(`${BASE}${up.url}`, { headers: { Range: 'bytes=100-199' } });
  assert.equal(part.status, 206);
  assert.equal(part.headers.get('content-range'), `bytes 100-199/${payload.length}`);
  assert.equal(part.headers.get('content-length'), '100');
  assert.equal(sha(Buffer.from(await part.arrayBuffer())), sha(payload.subarray(100, 200)));
  ok('Range 请求（206 / Content-Range / 分段内容一致）');

  // ---- 乱序分块应被拒绝
  const bad = await (await fetch(`${BASE}/api/upload/init`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'bad.bin', size: 1024, mime: 'application/octet-stream' }),
  })).json();
  const badRes = await fetch(`${BASE}/api/upload/chunk/${bad.fileId}`, {
    method: 'PUT', headers: { 'content-type': 'application/octet-stream', 'x-offset': '512' }, body: Buffer.alloc(10),
  });
  assert.equal(badRes.status, 409, 'offset 不匹配应 409');
  ok('乱序分块拒绝（409）');

  // ---- 空文件
  const empty = await uploadRaw('空.txt', Buffer.alloc(0), 'text/plain', '设备A');
  const emptyDl = await fetch(`${BASE}${empty.url}`);
  assert.equal(emptyDl.status, 200);
  assert.equal(Number(emptyDl.headers.get('content-length')), 0);
  ok('空文件可传可下');

  // ---- 打字指示
  const bTyping = new Promise((res) => B.sock.once('peer-typing', res));
  A.sock.emit('typing', { on: true });
  assert.equal((await bTyping).name, '设备A');
  ok('打字指示广播');

  // ---- UDP 多播发现（同机自测：独立 Discovery 实例应看到 server 的 beacon）
  // 注意：同机共享 ~/.antifybot-node-id，须给 scanner 独立 id，否则 beacon 会被当作"自己"忽略
  const scanner = new Discovery({ id: 'scanner-' + crypto.randomUUID(), name: 'scanner', announce: false });
  const found = new Promise((res) => { scanner.onPeer = (p, ev) => ev === 'up' && res(p); });
  scanner.start();
  const peer = await Promise.race([found, sleep(6000).then(() => null)]);
  scanner.stop();
  assert.ok(peer, 'scanner 应在 6 秒内发现 server 节点');
  assert.equal(peer.port, PORT);
  ok(`UDP 多播发现（beacon → ${peer.addr}:${peer.port}）`);

  // ---- 重启恢复：杀掉 server 重启，文件仍可下载
  A.sock.close(); B.sock.close();
  server.kill('SIGTERM');
  await sleep(800);
  server = startServer();
  await waitLobby();
  const dl2 = await fetch(`${BASE}${up.url}`);
  assert.equal(dl2.status, 200, '重启后文件应仍可下载');
  assert.equal(sha(Buffer.from(await dl2.arrayBuffer())), sha(payload));
  ok('服务重启后文件元数据恢复，下载仍可用');

  server.kill('SIGTERM');
  console.log(`\n🎉 全部 ${passed} 项通过\n`);
  process.exit(0);
})().catch((err) => {
  console.error('\n❌ 测试失败：', err.message, '\n');
  process.exit(1);
});
