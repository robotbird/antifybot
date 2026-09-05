#!/bin/bash
# 打包 AntifyBot.app（Tauri 壳 + LocalSend 节点，单二进制）
# 用法：bash rust/build-app.sh   （在仓库根或 rust/ 下均可）
set -euo pipefail

cd "$(dirname "$0")"
ROOT="$(pwd)"
APP_NAME="AntifyBot"
APP="AntifyBot.app"
OUT="$ROOT/$APP"

echo "── 1/4 release 构建（首次较慢）"
cargo build --release -p antify-gui
BIN="$ROOT/target/release/antify-gui"
[ -x "$BIN" ] || { echo "❌ 没找到 $BIN"; exit 1; }

echo "── 2/4 组装 .app"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp "$BIN" "$OUT/Contents/MacOS/$APP_NAME"
cp gui/icons/icon.icns "$OUT/Contents/Resources/AppIcon.icns"

cat > "$OUT/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>              <string>AntifyBot</string>
    <key>CFBundleDisplayName</key>       <string>AntifyBot</string>
    <key>CFBundleExecutable</key>        <string>AntifyBot</string>
    <key>CFBundleIdentifier</key>        <string>works.antifybot.rs</string>
    <key>CFBundleVersion</key>           <string>1.0.0</string>
    <key>CFBundleShortVersionString</key><string>1.0.0</string>
    <key>CFBundlePackageType</key>       <string>APPL</string>
    <key>CFBundleIconFile</key>          <string>AppIcon</string>
    <key>LSMinimumSystemVersion</key>    <string>11.3</string>
    <key>NSHighResolutionCapable</key>   <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
    <key>NSAppTransportSecurity</key>
    <dict>
        <!-- 面板与本机节点均为 127.0.0.1，LocalSend 对端为自签 HTTPS：按协议约定不校验 -->
        <key>NSAllowsLocalNetworking</key> <true/>
    </dict>
    <key>LSApplicationCategoryType</key> <string>public.app-category.utilities</string>
</dict>
</plist>
PLIST

echo "── 3/4 ad-hoc 签名"
codesign --force --deep --sign - "$OUT" >/dev/null 2>&1 || codesign --force --sign - "$OUT"

echo "── 4/4 完成"
du -sh "$OUT"
echo "运行：open $OUT"
