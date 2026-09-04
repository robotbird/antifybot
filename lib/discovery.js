'use strict';

/**
 * AntifyBot Discovery Protocol v1（参考 LocalSend 的多播发现思路）
 * ---------------------------------------------------------------
 * 传输层 : UDP 多播  组播组 239.71.66.1 : 45666
 * 包体   : UTF-8 JSON，一行，< 512 字节
 * 字段   : {
 *            v:    1,                  // 协议版本
 *            app:  "antifybot",        // 应用标识
 *            id:   "<uuid>",           // 节点唯一 ID（本机持久化）
 *            name: "节点昵称",
 *            port: 3777,               // HTTP 服务端口
 *            ts:   1690000000000       // 发包时间戳(ms)
 *          }
 * 行为   : 每个节点每 ANNOUNCE_MS 广播一次自身信息，同时监听组播组；
 *          超过 PEER_TTL_MS 未再发声的邻居视为下线。同一节点 id 只保留最新包。
 * 用途   : 桌面节点互相自动发现（无需输入 IP）；手机端直接扫码访问任一节点。
 */

const dgram = require('dgram');
const os = require('os');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const PROTO_VERSION = 1;
const APP_NAME = 'antifybot';
const MULTICAST_GROUP = '239.71.66.1';
const MULTICAST_PORT = 45666;
const ANNOUNCE_MS = 3000;
const PEER_TTL_MS = 10000;

/** 本节点持久化 id（重启不变），存于用户目录，CLI 与服务端共用 */
function localNodeId() {
  const file = path.join(os.homedir(), '.antifybot-node-id');
  try {
    const existing = fs.readFileSync(file, 'utf8').trim();
    if (existing) return existing;
  } catch (_) { /* 首次运行 */ }
  const id = crypto.randomUUID();
  try { fs.writeFileSync(file, id, { mode: 0o600 }); } catch (_) { /* 只读环境则退化为内存 id */ }
  return id;
}

/** 所有可用于多播的 IPv4 地址（排除回环/内部接口） */
function multicastInterfaces() {
  const out = [];
  for (const list of Object.values(os.networkInterfaces())) {
    for (const ni of list || []) {
      if (ni.family === 'IPv4' && !ni.internal) out.push(ni.address);
    }
  }
  return out;
}

class Discovery {
  /**
   * @param {object} opts
   * @param {string} opts.name  节点昵称
   * @param {number} opts.port  HTTP 服务端口（告知同伴如何访问我）
   * @param {boolean} [opts.announce=true]  是否广播自身（纯监听方传 false）
   */
  constructor(opts = {}) {
    this.name = String(opts.name || 'antify-node').slice(0, 60);
    this.port = Number(opts.port) || 0;
    this.announce = opts.announce !== false;
    this.id = opts.id || localNodeId();
    this.peers = new Map(); // id -> { id, name, addr, port, firstSeen, lastSeen }
    this.socket = null;
    this.timer = null;
    this.sweepTimer = null;
    this.onPeer = opts.onPeer || null;   // (peer, event: 'up'|'update'|'down')
    this._emit = (peer, event) => { if (this.onPeer) { try { this.onPeer(peer, event); } catch (_) {} } };
  }

  packet() {
    return Buffer.from(JSON.stringify({
      v: PROTO_VERSION, app: APP_NAME, id: this.id,
      name: this.name, port: this.port, ts: Date.now(),
    }));
  }

  start() {
    if (this.socket) return;
    const sock = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    this.socket = sock;

    sock.on('error', (err) => {
      // 多播在部分受限网络/虚拟网卡下可能失败：发现功能降级，不影响主服务
      console.warn(`[discovery] 多播不可用（${err.message}），已降级为仅单机模式`);
      this.stop();
    });

    sock.on('message', (buf, rinfo) => {
      if (buf.length > 1024) return;
      let msg;
      try { msg = JSON.parse(buf.toString('utf8')); } catch (_) { return; }
      if (!msg || msg.v !== PROTO_VERSION || msg.app !== APP_NAME) return;
      if (!msg.id || msg.id === this.id) return; // 忽略自己

      const peer = {
        id: String(msg.id),
        name: String(msg.name || 'unknown').slice(0, 60),
        addr: rinfo.address,
        port: Number(msg.port) || 80,
        firstSeen: this.peers.has(msg.id) ? this.peers.get(msg.id).firstSeen : Date.now(),
        lastSeen: Date.now(),
      };
      const existed = this.peers.has(peer.id);
      this.peers.set(peer.id, peer);
      this._emit(peer, existed ? 'update' : 'up');
    });

    sock.bind(MULTICAST_PORT, () => {
      for (const addr of multicastInterfaces()) {
        try {
          sock.addMembership(MULTICAST_GROUP, addr);
          if (this.announce) sock.setMulticastInterface(addr); // 以最后一个可用网卡为出口
        } catch (_) { /* 单个网卡失败忽略 */ }
      }
      try { sock.setMulticastTTL(1); } catch (_) {}
      if (this.announce) {
        this._sendOnce();
        this.timer = setInterval(() => this._sendOnce(), ANNOUNCE_MS);
        this.timer.unref();
      }
      // 周期清理过期邻居
      this.sweepTimer = setInterval(() => {
        const now = Date.now();
        for (const [id, p] of this.peers) {
          if (now - p.lastSeen > PEER_TTL_MS) {
            this.peers.delete(id);
            this._emit(p, 'down');
          }
        }
      }, ANNOUNCE_MS);
      this.sweepTimer.unref();
    });
  }

  _sendOnce() {
    if (!this.socket) return;
    const buf = this.packet();
    try { this.socket.send(buf, 0, buf.length, MULTICAST_PORT, MULTICAST_GROUP); } catch (_) {}
  }

  stop() {
    if (this.timer) { clearInterval(this.timer); this.timer = null; }
    if (this.sweepTimer) { clearInterval(this.sweepTimer); this.sweepTimer = null; }
    if (this.socket) { try { this.socket.close(); } catch (_) {} this.socket = null; }
  }

  peerList() {
    return [...this.peers.values()].sort((a, b) => a.name.localeCompare(b.name));
  }
}

module.exports = {
  Discovery,
  PROTO_VERSION,
  APP_NAME,
  MULTICAST_GROUP,
  MULTICAST_PORT,
  localNodeId,
  multicastInterfaces,
};
