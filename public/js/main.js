/* 入口：身份 → Socket.IO 连接 → 渲染历史/在线状态 → 收发消息与文件 → 各类交互 */

import { h, antSVG, toast, hueOf } from './util.js';
import { loadIdentity, saveName } from './identity.js';
import { uploadFile, getLobby } from './api.js';
import { ChatView } from './chat.js';

const $ = (id) => document.getElementById(id);
const me = loadIdentity();
const myHue = hueOf(me.deviceId);
const chat = new ChatView();
const seen = new Set();          // 已渲染消息 id（去重：自己发的会被广播回来）

/* ---------- 静态部位注入蚂蚁 ---------- */
document.querySelector('.brand-ant').innerHTML = antSVG();
document.querySelector('.brand-ant').style.setProperty('--h', '26');
document.querySelector('.empty-ant').innerHTML = antSVG();
document.querySelector('.empty-ant').style.setProperty('--h', '26');
document.querySelector('.drop-ant').innerHTML = antSVG();
document.querySelector('.drop-ant').style.setProperty('--h', '26');

/* ══════════ Socket.IO ══════════ */
const socket = io({ transports: ['websocket', 'polling'] });

socket.on('hello-ack', ({ serverName, history, presence }) => {
  $('net-name').textContent = serverName || '局域网';
  document.title = `${serverName || '蚁巢'} · AntifyBot`;
  chat.clear();
  seen.clear();
  for (const msg of history) { seen.add(msg.id); chat.message(msg, msg.from?.deviceId === me.deviceId); }
  renderPresence(presence);
});

function sayHello() {
  socket.emit('hello', { deviceId: me.deviceId, name: me.name, platform: me.platform }, (ack) => {
    if (ack?.hue != null) { /* 服务端色相与本地算法一致，仅确认连通 */ }
  });
}
socket.on('connect', () => { $('reconnect').hidden = true; sayHello(); });
socket.on('disconnect', () => { $('reconnect').hidden = false; });

socket.on('presence', renderPresence);

socket.on('message', (msg) => {
  if (seen.has(msg.id)) return;
  seen.add(msg.id);
  const mine = msg.from?.deviceId === me.deviceId;
  chat.message(msg, mine);
  if (!mine && msg.type === 'file') toast(`收到文件：${msg.file.name}`);
});

/* ══════════ 在线状态 ══════════ */
let presenceList = [];
function renderPresence(list) {
  presenceList = list || [];
  $('online-count').textContent = presenceList.length || 1;

  const rail = $('presence-rail');
  rail.replaceChildren();
  const shown = presenceList.slice(0, 4);
  for (const p of shown) {
    const a = h('span', { class: 'avatar', style: { '--h': String(p.hue) }, html: antSVG(), title: p.name });
    rail.append(a);
  }
  if (presenceList.length > 4) {
    rail.append(h('span', { class: 'avatar more', textContent: `+${presenceList.length - 4}` }));
  }
  renderDeviceSheet();
}

function renderDeviceSheet() {
  const ul = $('device-list');
  ul.replaceChildren();
  for (const p of presenceList) {
    const li = h('li', {},
      h('span', { class: 'avatar', style: { '--h': String(p.hue) }, html: antSVG() }),
      h('div', {},
        h('div', { class: 'd-name', textContent: p.name }),
        h('div', { class: 'd-plat', textContent: p.platform })),
    );
    if (p.deviceId === me.deviceId) li.append(h('span', { class: 'd-you', textContent: '你' }));
    ul.append(li);
  }
  $('rename-input').value = me.name;
}

/* ══════════ 正在输入 ══════════ */
const typingMap = new Map(); // deviceId -> {name, until}
socket.on('peer-typing', ({ deviceId, name, on }) => {
  if (on) typingMap.set(deviceId, { name, until: Date.now() + 3000 });
  else typingMap.delete(deviceId);
});
setInterval(() => {
  const box = $('typing');
  const now = Date.now();
  for (const [id, t] of typingMap) if (t.until < now) typingMap.delete(id);
  const names = [...typingMap.values()].map((t) => t.name);
  if (!names.length) { box.hidden = true; box.replaceChildren(); return; }
  const label = names.length <= 2 ? names.join(' 和 ') : `${names[0]} 等 ${names.length} 只蚂蚁`;
  box.hidden = false;
  box.replaceChildren(h('span', {}, `${label} 正在输入`, h('span', { class: 'dots' })));
}, 500);

/* ══════════ 发送文字 ══════════ */
const input = $('input');
const sendBtn = $('btn-send');

function autoSize() {
  input.style.height = 'auto';
  input.style.height = Math.min(input.scrollHeight, window.innerHeight * 0.38) + 'px';
}
input.addEventListener('input', () => {
  autoSize();
  sendBtn.disabled = !input.value.trim();
  maybeEmitTyping();
});
sendBtn.disabled = true;

const coarse = matchMedia('(pointer: coarse)').matches;
input.addEventListener('keydown', (e) => {
  // 中文输入法组词回车不应直接发送
  if (e.key === 'Enter' && !e.shiftKey && !coarse && !e.isComposing && e.keyCode !== 229) {
    e.preventDefault();
    sendText();
  }
});
sendBtn.addEventListener('click', sendText);

function sendText() {
  const text = input.value.trim();
  if (!text || !socket.connected) { if (!socket.connected) toast('尚未连上蚁道，稍等…', true); return; }
  input.value = ''; autoSize(); sendBtn.disabled = true;
  socket.emit('typing', { on: false });

  const temp = {
    id: 'local-' + Math.random().toString(36).slice(2),
    ts: Date.now(), type: 'text',
    from: { deviceId: me.deviceId, name: me.name, hue: myHue }, text,
  };
  chat.message(temp, true);
  const row = chat.messagesEl.querySelector(`[data-mid="${temp.id}"]`);
  const timeEl = row?.querySelector('.msg-time');

  socket.timeout(5000).emit('message', { text }, (err, resp) => {
    if (err || !resp?.ok) {
      if (timeEl) timeEl.textContent = '发送失败';
      toast(resp?.error || '发送失败，请重试', true);
      return;
    }
    seen.add(resp.id); // 即将广播回来，先标记去重
    if (timeEl) { timeEl.textContent = ''; timeEl.append('已送达 ', h('span', { class: 'tick', textContent: '✓' })); }
  });
}

/* 打字指示（节流） */
let typingLastSent = 0, typingStopTimer = 0;
function maybeEmitTyping() {
  if (!socket.connected || !input.value) return;
  const now = Date.now();
  if (now - typingLastSent > 1200) { typingLastSent = now; socket.emit('typing', { on: true }); }
  clearTimeout(typingStopTimer);
  typingStopTimer = setTimeout(() => socket.emit('typing', { on: false }), 1800);
}

/* ══════════ 文件发送队列 ══════════ */
const fileQueue = [];
let pumping = false;

function enqueueFiles(files) {
  for (const f of files) fileQueue.push(f);
  pump();
  if (files.length) toast(`排队发送 ${files.length} 个文件 🐜`);
}

async function pump() {
  if (pumping) return;
  pumping = true;
  try {
    while (fileQueue.length) {
      const file = fileQueue.shift();
      await sendFile(file);
    }
  } finally { pumping = false; }
}

async function sendFile(file) {
  const ctl = chat.uploading(file, myHue);
  try {
    const r = await uploadFile(file, { deviceId: me.deviceId, name: me.name }, {
      onProgress: (sent, total) => ctl.progress(sent, total),
    });
    seen.add(r.messageId);
    ctl.done();
  } catch (err) {
    ctl.fail(err.message);
    toast(`「${file.name}」发送失败：${err.message}`, true);
  }
}

window.addEventListener('antify:retry-file', (e) => { fileQueue.push(e.detail.file); pump(); });

$('btn-attach').addEventListener('click', () => $('file-input').click());
$('file-input').addEventListener('change', (e) => { enqueueFiles([...e.target.files]); e.target.value = ''; });

/* 拖拽 */
let dragDepth = 0;
window.addEventListener('dragenter', (e) => {
  if (![...e.dataTransfer?.types || []].includes('Files')) return;
  e.preventDefault(); dragDepth++; $('drop-overlay').hidden = false;
});
window.addEventListener('dragover', (e) => e.preventDefault());
window.addEventListener('dragleave', () => { if (--dragDepth <= 0) { dragDepth = 0; $('drop-overlay').hidden = true; } });
window.addEventListener('drop', (e) => {
  e.preventDefault(); dragDepth = 0; $('drop-overlay').hidden = true;
  if (e.dataTransfer?.files?.length) enqueueFiles([...e.dataTransfer.files]);
});

/* 粘贴（截图直发） */
document.addEventListener('paste', (e) => {
  const files = [...(e.clipboardData?.files || [])];
  if (files.length) { e.preventDefault(); enqueueFiles(files); }
});

/* ══════════ 弹层通用 ══════════ */
const backdrop = $('sheet-backdrop');
function openSheet(el) { closeSheets(); backdrop.hidden = false; el.hidden = false; }
function closeSheets() {
  backdrop.hidden = true;
  $('sheet-qr').hidden = true;
  $('sheet-devices').hidden = true;
  $('lightbox').hidden = true;
}
backdrop.addEventListener('click', closeSheets);
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeSheets(); });

/* 二维码分享 */
async function openQr() {
  openSheet($('sheet-qr'));
  try {
    const lobby = await getLobby();
    $('qr-img').src = lobby.qr;
    $('qr-url').textContent = lobby.primary;
    $('qr-meta').textContent = `AntifyBot v${lobby.version} · 节点「${lobby.serverName}」 · 消息与文件仅在局域网内流转`;
  } catch (err) {
    toast('获取二维码失败：' + err.message, true);
  }
}
$('btn-qr').addEventListener('click', openQr);
$('empty-qr').addEventListener('click', openQr);

$('btn-copy-url').addEventListener('click', async () => {
  const text = $('qr-url').textContent;
  try {
    if (navigator.clipboard?.writeText) await navigator.clipboard.writeText(text);
    else {
      // http://内网IP 非安全上下文，无 clipboard API：退化为 execCommand
      const ta = h('textarea', { style: { position: 'fixed', opacity: '0' } });
      ta.value = text; document.body.append(ta); ta.select();
      document.execCommand('copy'); ta.remove();
    }
    toast('地址已复制，发给同一 Wi-Fi 的 TA 吧');
  } catch (_) { toast('复制失败，请手动长按复制', true); }
});

/* 设备列表 / 改名 */
$('presence-rail').addEventListener('click', () => openSheet($('sheet-devices')));
$('btn-rename').addEventListener('click', doRename);
$('rename-input').addEventListener('keydown', (e) => { if (e.key === 'Enter') doRename(); });
function doRename() {
  const name = $('rename-input').value.trim();
  if (!name) return toast('名字不能为空', true);
  saveName(name);
  me.name = name;
  sayHello(); // 服务端会广播改名系统消息
  toast(`你现在是「${name}」`);
  closeSheets();
}

/* 灯箱 */
window.addEventListener('antify:lightbox', (e) => {
  $('lightbox-img').src = e.detail.url;
  $('lightbox-download').href = e.detail.url;
  $('lightbox-download').setAttribute('download', e.detail.name || '');
  backdrop.hidden = false;
  $('lightbox').hidden = false;
});
$('lightbox-close').addEventListener('click', closeSheets);
$('lightbox').addEventListener('click', (e) => { if (e.target === $('lightbox')) closeSheets(); });

/* 通知权限：收文件时能提醒（可选，失败静默） */
if ('Notification' in window && Notification.permission === 'default') {
  const ask = () => { Notification.requestPermission().catch(() => {}); window.removeEventListener('pointerdown', ask); };
  window.addEventListener('pointerdown', ask, { once: true });
}
document.addEventListener('visibilitychange', () => {
  if (document.hidden) return;
  // 回前台时若断线则提示（Socket.IO 会自动重连）
  if (!socket.connected) $('reconnect').hidden = false;
});
