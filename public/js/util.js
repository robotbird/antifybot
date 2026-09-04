/* 工具函数：DOM 构建（全程 textContent，杜绝 XSS）、格式化、蚂蚁 SVG、Toast */

export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'dataset') Object.assign(el.dataset, v);
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
    else if (k === 'html') el.innerHTML = v;          // 仅用于内部生成的 SVG（无用户输入）
    else if (k === 'textContent' || k === 'value' || k === 'disabled') el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of children.flat()) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(kid));
  }
  return el;
}

/**
 * 几何蚁 Logo。身体 fill/stroke 由 CSS 按 --h（设备色相）染色：
 * .avatar svg .ant-body / .ant-limb 见 style.css
 */
export function antSVG() {
  return `<svg viewBox="0 0 64 64" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
  <g stroke-linecap="round" stroke-linejoin="round">
    <g class="ant-limb" fill="none" stroke-width="2.6">
      <path d="M31 39 L25 49"/><path d="M34 41 L34 52"/><path d="M38 39 L44 49"/>
      <path d="M46 18 C48 12 54 10 58 9"/>
      <path d="M42 17 C42 11 37 8 33 6"/>
    </g>
    <ellipse class="ant-body" cx="15" cy="36" rx="12" ry="9.5" transform="rotate(-12 15 36)"/>
    <circle  class="ant-body" cx="34" cy="34" r="7"/>
    <circle  class="ant-body" cx="47" cy="26" r="8.5"/>
    <circle cx="49.6" cy="23.6" r="1.7" fill="rgba(255,255,255,.92)"/>
  </g>
</svg>`;
}

export function formatBytes(n) {
  if (!Number.isFinite(n)) return '?';
  if (n < 1024) return `${Math.round(n)} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n, i = -1;
  do { v /= 1024; i++; } while (v >= 1024 && i < units.length - 1);
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

export function fmtTime(ts) {
  const d = new Date(ts);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

export function fmtDivider(ts) {
  const d = new Date(ts);
  const now = new Date();
  const sameDay = (a, b) => a.toDateString() === b.toDateString();
  const yest = new Date(now); yest.setDate(now.getDate() - 1);
  const hm = fmtTime(ts);
  if (sameDay(d, now)) return hm;
  if (sameDay(d, yest)) return `昨天 ${hm}`;
  return `${d.getMonth() + 1}月${d.getDate()}日 ${hm}`;
}

const KIND_MAP = [
  [/^image\//, 'image'], [/^video\//, 'video'], [/^audio\//, 'audio'],
  [/zip|tar|rar|7z|gz|bz2|xz/i, 'zip'],
  [/word|excel|powerpoint|msword|officedocument|pdf|csv|iwork|pages|numbers|key/i, 'doc'],
  [/javascript|json|xml|yaml|html|css|shell|python|java|source|typescript/i, 'code'],
  [/android\/package-archive|\.apk$/i, 'apk'],
];

export function kindOf(name, mime = '') {
  for (const [re, kind] of KIND_MAP) if (re.test(mime) || re.test(name)) return kind;
  return 'file';
}

export function extOf(name) {
  const m = /\.([a-z0-9]{1,5})$/i.exec(name || '');
  return m ? m[1] : 'FILE';
}

export function hueOf(id) {
  let x = 0;
  for (const ch of String(id)) x = (x * 31 + ch.codePointAt(0)) % 360;
  return x;
}

let toastBox = null;
export function toast(text, isErr = false) {
  if (!toastBox) toastBox = document.getElementById('toasts');
  const t = h('div', { class: 'toast' + (isErr ? ' err' : ''), textContent: text });
  toastBox.append(t);
  setTimeout(() => { t.style.transition = 'opacity .3s'; t.style.opacity = '0'; setTimeout(() => t.remove(), 320); }, 2600);
}
