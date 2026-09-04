#!/usr/bin/env node
'use strict';

/**
 * 扫描局域网内的 AntifyBot 节点（监听多播 beacon 6 秒后退出）。
 * 用法：npm run discover
 */

const { Discovery } = require('../lib/discovery');

console.log('🔍 正在扫描局域网内的 AntifyBot 节点（约 6 秒）…\n');

const d = new Discovery({ name: 'scanner', announce: false });
const seen = new Map();

d.onPeer = (peer, event) => {
  if (event === 'down') {
    seen.delete(peer.id);
    console.log(`  ⚠️  下线  ${peer.name}  (${peer.addr})`);
    return;
  }
  if (seen.has(peer.id)) return;
  seen.set(peer.id, peer);
  console.log(`  🐜 发现  ${peer.name.padEnd(24)} http://${peer.addr}:${peer.port}`);
};

d.start();

setTimeout(() => {
  d.stop();
  if (seen.size === 0) {
    console.log('\n未发现节点。请确认：');
    console.log('  · 至少有一台设备已运行 `npm start`');
    console.log('  · 各设备处于同一网段 / 同一 Wi-Fi');
    console.log('  · 路由器未开启 AP 隔离（isolated client）');
    process.exit(0);
  }
  console.log(`\n共 ${seen.size} 个节点。手机直接扫码或浏览器打开上述地址即可使用。`);
  process.exit(0);
}, 6000);
