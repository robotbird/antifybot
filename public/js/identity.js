/* 匿名身份：localStorage 存 deviceId + 昵称，免注册（参考 earthchat 的匿名方案） */

const KEY = 'antifybot:identity';

function detectPlatform() {
  const ua = navigator.userAgent;
  if (/iPhone/i.test(ua)) return 'iPhone';
  if (/iPad/i.test(ua) || (ua.includes('Macintosh') && navigator.maxTouchPoints > 1)) return 'iPad';
  if (/Android/i.test(ua)) return 'Android';
  if (/Windows/i.test(ua)) return 'Windows';
  if (/Macintosh/i.test(ua)) return 'macOS';
  if (/Linux/i.test(ua)) return 'Linux';
  return '浏览器';
}

function uuid() {
  return (crypto.randomUUID && crypto.randomUUID()) ||
    'xxxxxxxxyxxx'.replace(/[xy]/g, (c) => {
      const r = Math.random() * 16 | 0;
      return (c === 'x' ? r : (r & 0x3 | 0x8)).toString(16);
    }) + Date.now().toString(36);
}

function defaultName(id) {
  return `${detectPlatform()}·${id.slice(0, 4).toUpperCase()}`;
}

export function loadIdentity() {
  let it = null;
  try { it = JSON.parse(localStorage.getItem(KEY)); } catch (_) {}
  if (!it || !it.deviceId) {
    it = { deviceId: uuid() };
    it.name = defaultName(it.deviceId);
    try { localStorage.setItem(KEY, JSON.stringify(it)); } catch (_) {}
  }
  it.platform = detectPlatform();
  return it;
}

export function saveName(name) {
  const it = loadIdentity();
  it.name = name || it.name;
  try { localStorage.setItem(KEY, JSON.stringify({ deviceId: it.deviceId, name: it.name })); } catch (_) {}
  return it;
}
