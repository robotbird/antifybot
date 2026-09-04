/* 聊天渲染：气泡 / 文件卡片 / 系统签 / 上传中的"蚁径"进度 / 分隔时间 */

import { h, antSVG, formatBytes, fmtTime, fmtDivider, kindOf, extOf } from './util.js';

export class ChatView {
  constructor() {
    this.messagesEl = document.getElementById('messages');
    this.chatEl = document.getElementById('chat');
    this.emptyEl = document.getElementById('empty');
    this._lastTs = 0;
    this._lastSender = null;
  }

  _nearBottom() {
    const el = this.chatEl;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 140;
  }

  _scrollToBottom(force) {
    if (force || this._nearBottom()) this.chatEl.scrollTop = this.chatEl.scrollHeight;
  }

  _dividerIfNeeded(ts) {
    if (ts - this._lastTs > 10 * 60 * 1000 && this._lastTs) {
      this.messagesEl.append(h('div', { class: 'sys', textContent: fmtDivider(ts) }));
    }
    this._lastTs = ts;
  }

  _avatar(hue) {
    return h('span', { class: 'avatar', style: { '--h': String(hue) }, html: antSVG() });
  }

  /** 系统消息 */
  sys(text) {
    this.messagesEl.append(h('div', { class: 'sys', textContent: text }));
    this._lastTs = Date.now();
    this._scrollToBottom();
    this._checkEmpty();
  }

  /** 渲染一条服务端历史/广播消息 */
  message(msg, mine) {
    this._dividerIfNeeded(msg.ts);
    if (msg.type === 'sys') return this.sys(msg.text);

    const row = h('div', { class: 'msg' + (mine ? ' mine' : ''), dataset: { mid: msg.id } });
    if (!mine) row.append(this._avatar(msg.from.hue));

    const main = h('div', { class: 'msg-main' });
    if (!mine) main.append(h('div', { class: 'msg-name', textContent: msg.from.name }));

    if (msg.type === 'text') {
      main.append(h('div', { class: 'bubble', textContent: msg.text }));
    } else if (msg.type === 'file') {
      main.append(this._fileBubble(msg.file));
    }

    main.append(h('div', { class: 'msg-time', textContent: fmtTime(msg.ts) }));
    row.append(main);
    this.messagesEl.append(row);
    this._lastSender = mine ? 'me' : msg.from?.deviceId;
    this._scrollToBottom(mine);
    this._checkEmpty();
  }

  _fileBubble(file) {
    const kind = kindOf(file.name, file.mime);
    const bubble = h('div', { class: 'bubble' });

    // 图片：直接出缩略图（点击灯箱看大图）
    if (kind === 'image' && file.size < 40 * 1024 * 1024) {
      const img = h('img', {
        class: 'file-thumb', src: file.url, alt: file.name,
        loading: 'lazy', referrerpolicy: 'no-referrer',
      });
      img.addEventListener('click', () => window.dispatchEvent(new CustomEvent('antify:lightbox', { detail: file })));
      img.addEventListener('error', () => bubble.replaceChildren(this._fileCard(file, kind)), { once: true });
      bubble.append(img);
      return bubble;
    }
    bubble.append(this._fileCard(file, kind));
    return bubble;
  }

  _fileCard(file, kind) {
    return h('div', { class: 'file-card' },
      h('div', { class: 'file-tile', 'data-kind': kind, textContent: extOf(file.name) }),
      h('div', { class: 'file-info' },
        h('div', { class: 'file-name', textContent: file.name, title: file.name }),
        h('div', { class: 'file-meta', textContent: formatBytes(file.size) }),
      ),
      h('a', {
        class: 'file-dl', href: file.url, download: file.name,
        'aria-label': `下载 ${file.name}`, title: '下载',
        html: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/></svg>',
      }),
    );
  }

  /**
   * 本地"发送中"占位气泡（含蚁径进度条）。
   * 返回控制器：progress / done / fail
   */
  uploading(file, myHue) {
    this._dividerIfNeeded(Date.now());
    const kind = kindOf(file.name, file.type);
    const fill = h('div', { class: 'trail-fill' });
    const ant = h('span', { class: 'trail-ant', html: antSVG() });
    const trail = h('div', { class: 'trail' }, fill, ant);
    const meta = h('div', { class: 'file-meta', textContent: formatBytes(0) + ' / ' + formatBytes(file.size) });
    const card = h('div', { class: 'file-card' },
      h('div', { class: 'file-tile', 'data-kind': kind, textContent: extOf(file.name) }),
      h('div', { class: 'file-info' },
        h('div', { class: 'file-name', textContent: file.name, title: file.name }),
        meta,
        trail,
      ),
    );
    const time = h('div', { class: 'msg-time', textContent: fmtTime(Date.now()) + ' · 上传中' });
    const main = h('div', { class: 'msg-main' }, h('div', { class: 'bubble' }, card), time);
    const row = h('div', { class: 'msg mine' }, main);
    this.messagesEl.append(row);
    this._scrollToBottom(true);
    this._checkEmpty();

    let startedAt = 0;
    return {
      el: row,
      progress(sent, total) {
        const pct = total ? Math.min(100, (sent / total) * 100) : 100;
        fill.style.width = pct + '%';
        ant.style.left = pct + '%';
        if (sent > 0) {
          if (!startedAt) startedAt = performance.now();
          const speed = sent / Math.max(0.35, (performance.now() - startedAt) / 1000);
          meta.textContent = `${formatBytes(sent)} / ${formatBytes(total)} · ${formatBytes(speed)}/s`;
        }
      },
      done() {
        trail.remove();
        meta.textContent = formatBytes(file.size); // 速度行还原为最终大小，与广播卡片一致
        time.textContent = fmtTime(Date.now()) + ' · 已送达';
        time.append(Object.assign(document.createElement('span'), { className: 'tick', textContent: '✓' }));
      },
      fail(errText) {
        trail.remove();
        time.textContent = fmtTime(Date.now()) + ' · 发送失败';
        meta.textContent = errText || '上传失败，点击重试';
        card.style.opacity = '.72';
        const retry = h('button', {
          class: 'mini-btn', textContent: '重试',
          style: { flex: 'none' },
        });
        retry.addEventListener('click', () => {
          window.dispatchEvent(new CustomEvent('antify:retry-file', { detail: { file } }));
          row.remove();
        });
        card.append(retry);
      },
    };
  }

  _checkEmpty() {
    const has = this.messagesEl.children.length > 0;
    this.emptyEl.hidden = has;
    if (!has) this._lastTs = 0;
  }

  clear() {
    this.messagesEl.replaceChildren();
    this._lastTs = 0;
    this._checkEmpty();
  }
}
