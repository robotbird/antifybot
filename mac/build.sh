#!/bin/bash
# 构建 AntifyBot P2P macOS 应用（无需 Xcode 工程，仅 swiftc + iconutil）
# 用法：bash mac/build.sh   → 产出 mac/AntifyBot.app
set -euo pipefail
cd "$(dirname "$0")"

APP=AntifyBot.app
HTML=../antify-p2p.html

[ -f "$HTML" ] || { echo "缺少 $HTML"; exit 1; }

echo "▸ 编译 Swift 壳…"
rm -rf "$APP" build
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" build
swiftc -O AntifyBot-P2P.swift -o "$APP/Contents/MacOS/AntifyBot"

echo "▸ 打包页面资源…"
cp "$HTML" "$APP/Contents/Resources/antify-p2p.html"

echo "▸ 生成图标…"
swift genicon.swift
mkdir -p AppIcon.iconset
for s in 16 32 128 256 512; do
  sips -z $s $s build/icon-1024.png --out AppIcon.iconset/icon_${s}x${s}.png >/dev/null
  sips -z $((s*2)) $((s*2)) build/icon-1024.png --out AppIcon.iconset/icon_${s}x${s}@2x.png >/dev/null
done
iconutil -c icns AppIcon.iconset -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf AppIcon.iconset

echo "▸ 写入 Info.plist + ad-hoc 签名…"
cp Info.plist "$APP/Contents/Info.plist"
codesign --force --sign - "$APP" >/dev/null 2>&1 || echo "  （ad-hoc 签名跳过，本机仍可运行）"

echo "✓ 完成：$PWD/$APP"
du -sh "$APP"
