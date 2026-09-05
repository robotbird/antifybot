//! 面板页面（内嵌 HTML，无外部依赖，离线可用）
//! 左右分栏：深色侧栏（品牌 + 设备列表 + 拖放提示），暖白主区。
//! 设备上线 → 左侧列表；无设备 → 两侧同时提示「等待设备上线」；
//! 点选设备 → 会话视图：文本/文件按气泡呈现（收到的文件带「显示」），
//! 活动传输为会话流内吸顶进度卡，最下面是聊天输入条（📎 选文件 / Enter 发文本）。
//! 指纹/端口等技术细节收进 ⚙ 节点信息，事件以 toast 呈现。
pub const DASHBOARD: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AntifyBot</title>
<style>
  :root{
    --bg:#f6f5f1; --card:#ffffff; --ink:#1d1d1f; --muted:#86868b;
    --line:rgba(0,0,0,.10); --accent:#d98324; --accent-deep:#b06a15;
    --accent-soft:rgba(217,131,36,.10); --ok:#34a853; --err:#e5484d;
    /* 侧栏在浅色模式下即深色（分栏对比是视觉主体） */
    --side:#1b1b1d; --side-ink:#f5f5f7; --side-muted:rgba(235,235,245,.55);
    --side-line:rgba(255,255,255,.08); --side-hover:rgba(255,255,255,.06);
    --side-sel:rgba(232,147,44,.18);
    --shadow:0 1px 2px rgba(0,0,0,.03), 0 12px 32px -20px rgba(0,0,0,.28);
    --sans:-apple-system,BlinkMacSystemFont,"SF Pro Text","Segoe UI",
           "PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif;
    --mono:ui-monospace,"SF Mono",Menlo,Consolas,monospace;
  }
  @media(prefers-color-scheme:dark){
    :root{ --bg:#141416; --card:#1e1e22; --ink:#f0f0f2; --muted:#9a9aa0;
      --line:rgba(255,255,255,.12); --accent:#e8932c; --accent-deep:#f0a64e;
      --accent-soft:rgba(232,147,44,.14); --ok:#4cc38a; --err:#ff6b6e;
      --side:#101012; --shadow:0 1px 2px rgba(0,0,0,.4), 0 16px 36px -20px rgba(0,0,0,.7); }
  }
  *{box-sizing:border-box}
  html,body{height:100%}
  body{margin:0;background:var(--bg);color:var(--ink);font-family:var(--sans);
    line-height:1.5;overflow:hidden;-webkit-font-smoothing:antialiased;
    transition:background .3s,color .3s}
  .app{display:flex;height:100%}

  /* ── 左侧栏 ── */
  .side{width:248px;flex:none;background:var(--side);color:var(--side-ink);
    display:flex;flex-direction:column}
  .brand{display:flex;align-items:center;gap:8px;padding:15px 14px 10px;
    font-weight:700;font-size:15px;letter-spacing:-.01em}
  .brand .bico{font-size:18px;line-height:1}
  .brand .bsp{flex:1}
  .sbtn{width:26px;height:26px;border-radius:8px;border:none;background:transparent;
    color:var(--side-muted);font-size:14px;line-height:1;display:grid;place-items:center;
    cursor:pointer;transition:.15s}
  .sbtn:hover{background:var(--side-hover);color:var(--side-ink)}
  .side-scroll{flex:1;overflow-y:auto;padding:4px 10px 10px}
  .label{display:flex;align-items:center;gap:6px;padding:10px 8px 6px;
    font-size:11.5px;font-weight:600;color:var(--side-muted);letter-spacing:.08em}
  .label .n{font-weight:500;letter-spacing:0}
  .label .lsp{flex:1}
  .dev{display:flex;align-items:center;gap:10px;padding:9px 10px;border-radius:10px;
    cursor:pointer;transition:background .15s;outline:1.5px dashed transparent;
    outline-offset:-1.5px}
  .dev:hover{background:var(--side-hover)}
  .dev:focus-visible{outline-color:var(--side-muted)}
  .dev.on{background:var(--side-sel);outline-color:var(--accent)}
  .dev .ico{font-size:20px;line-height:1;flex:none}
  .dev .mid{min-width:0;flex:1}
  .dev .nm{font-size:13.5px;font-weight:600;color:var(--side-ink);
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .dev .meta{font-size:11.5px;color:var(--side-muted);
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .dev.dropover{outline:1.5px solid var(--accent);background:var(--side-hover)}
  body.dropping .dev{outline-color:var(--accent)}
  .devempty{margin:12px 8px;padding:20px 10px;text-align:center;font-size:12.5px;
    color:var(--side-muted);border:1.5px dashed var(--side-line);border-radius:12px}
  .devempty small{font-size:11px;opacity:.8}
  .side-foot{padding:12px 16px;border-top:1px solid var(--side-line);
    font-size:12px;color:var(--side-muted)}

  /* ── 主区 ── */
  .main{flex:1;min-width:0;display:flex;flex-direction:column;min-height:0}

  /* 会话视图（选中设备后）：头部 + 气泡流 + 底部聊天输入条 */
  #chatview{flex:1;min-height:0;display:flex;flex-direction:column}
  #chatview[hidden]{display:none}
  .chead{flex:none;padding:13px 26px;border-bottom:1px solid var(--line);
    display:flex;align-items:baseline;gap:10px}
  .chead h2{margin:0;font-size:16px;font-weight:700;letter-spacing:-.01em;
    max-width:50%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .chead .cmeta{font-size:12px;color:var(--muted);min-width:0;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .cscroll{flex:1;min-height:0;overflow-y:auto}
  body.dropping .cscroll{box-shadow:inset 0 0 0 1.5px var(--accent)}
  #chatlist{display:flex;flex-direction:column;gap:12px;padding:18px 26px;
    width:min(720px,100%);margin:0 auto}
  .chathint{margin:60px auto;text-align:center;color:var(--muted);font-size:13px;line-height:1.9}
  .chathint small{font-size:11.5px;opacity:.85}
  .msg{display:flex;flex-direction:column;max-width:78%}
  .msg.out{align-self:flex-end;align-items:flex-end}
  .msg.in{align-self:flex-start;align-items:flex-start}
  .bub{padding:9px 14px;border-radius:16px;font-size:14px;line-height:1.5;
    text-align:left;white-space:pre-wrap;word-break:break-word}
  .msg.out .bub{background:var(--accent);color:#fff;border-bottom-right-radius:5px}
  .msg.in .bub{background:var(--card);border:1px solid var(--line);
    box-shadow:var(--shadow);border-bottom-left-radius:5px}
  .msg .t{font-size:10.5px;color:var(--muted);margin-top:3px;padding:0 4px}
  .fchip{display:flex;align-items:center;gap:9px;min-width:0}
  .fchip .fico{font-size:20px;line-height:1;flex:none}
  .fchip .fmid{min-width:0}
  .fchip .fnm{font-weight:600;font-size:13.5px;max-width:240px;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .fchip .fsz{font-size:11px;opacity:.75;font-family:var(--mono)}
  .rxb{margin-top:7px;border:none;background:transparent;cursor:pointer;
    font-family:var(--sans);font-size:11.5px;color:var(--accent-deep);
    padding:0;font-weight:600}
  .rxb:hover{text-decoration:underline}
  .msg.out .rxb{color:inherit;opacity:.9}
  .cfoot{flex:none;display:flex;gap:10px;align-items:center;
    padding:12px 26px 16px;border-top:1px solid var(--line)}
  .attach{flex:none;width:36px;height:36px;border-radius:50%;cursor:pointer;
    border:1px solid var(--line);background:var(--card);color:var(--ink);
    font-size:15px;line-height:1;display:grid;place-items:center;transition:.15s}
  .attach:hover{border-color:var(--accent)}
  #chatinput{flex:1;min-width:0;background:var(--card);border:1px solid var(--line);
    border-radius:999px;padding:9px 16px;font-size:14px;font-family:var(--sans);
    color:var(--ink);transition:.15s}
  #chatinput:focus{outline:none;border-color:var(--accent)}
  #chatinput:disabled{opacity:.6}
  .sendbtn{flex:none;border:none;border-radius:999px;background:var(--accent);
    color:#fff;font-family:var(--sans);font-weight:600;font-size:13.5px;
    padding:9px 18px;cursor:pointer;transition:.15s}
  .sendbtn:hover{background:var(--accent-deep)}

  /* 等待屏（未选设备） */
  #waitview{flex:1;min-height:0;display:flex;flex-direction:column;overflow-y:auto}
  #waitview[hidden]{display:none}
  .center{flex:1;display:flex;flex-direction:column;align-items:center;
    justify-content:center;padding:32px 28px;text-align:center;min-height:220px}
  .center h1{font-size:clamp(22px,3vw,30px);font-weight:700;letter-spacing:-.02em;
    margin:0 0 8px;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .center .sub{color:var(--muted);font-size:13.5px;margin:0 0 30px}
  .dropzone{width:min(440px,86%);padding:46px 20px;border:1.5px dashed var(--line);
    border-radius:16px;color:var(--muted);font-size:14px;transition:.18s;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  body.dropping .dropzone{border-color:var(--accent);background:var(--accent-soft);
    color:var(--accent-deep)}
  .dropzone.dropover{border-style:solid;border-color:var(--accent);
    background:var(--accent-soft)}
  .acts2{margin-top:24px;display:flex;gap:14px;align-items:center}
  .linkbtn{background:none;border:none;padding:4px 2px;font-family:var(--sans);
    font-size:13.5px;color:var(--muted);cursor:pointer;transition:.15s}
  .linkbtn.strong{color:var(--ink);font-weight:600}
  .linkbtn:hover{color:var(--accent-deep)}
  .acts2 .sep{color:var(--muted);opacity:.45;font-size:12px}

  /* ── 传输进度（会话流内吸顶） ── */
  #busy{position:sticky;top:0;z-index:5;width:min(560px,100%);margin:14px auto 0;
    padding:0 26px;display:flex;flex-direction:column;gap:10px}
  .prog{background:var(--card);border:1px solid var(--line);border-radius:16px;
    padding:16px 18px;box-shadow:var(--shadow)}
  .prog .row{display:flex;justify-content:space-between;align-items:baseline;gap:12px;font-size:13.5px}
  .prog .row b{font-weight:600;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .prog .sz{color:var(--muted);font-family:var(--mono);font-size:12px;flex:none}
  .bar{height:6px;background:var(--accent-soft);border-radius:99px;overflow:hidden;margin:10px 0 2px}
  .bar i{display:block;height:100%;background:var(--accent);border-radius:99px;transition:width .3s}
  .files{margin:8px 0 0;padding:0;list-style:none}
  .files li{font-size:13px;display:flex;gap:8px;align-items:center;padding:3px 0;color:var(--muted)}
  .files li.ok{color:var(--ink)}
  .files .tick{color:var(--ok);width:14px;text-align:center}
  .files .sz{margin-left:auto;font-family:var(--mono);font-size:11.5px}

  /* ── 最近接收（主区底部，空则整节隐藏） ── */
  #rxsec{width:min(560px,100%);margin:0 auto 44px;padding:0 26px}
  .rxhead{display:flex;gap:8px;align-items:baseline;margin-bottom:4px;
    font-size:12px;font-weight:600;color:var(--muted);letter-spacing:.06em}
  .rxhead .n{font-weight:500;letter-spacing:0}
  ul.received{margin:0;padding:0;list-style:none}
  .received li{display:flex;align-items:center;gap:10px;padding:10px 2px;
    border-bottom:1px solid var(--line);font-size:13.5px}
  .received li:last-child{border-bottom:none}
  .received .nm{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .received .sz{color:var(--muted);font-size:12px;font-family:var(--mono);flex:none}

  /* ── 按钮 / 对话框 ── */
  .btn{font-family:var(--sans);font-size:14px;font-weight:600;border-radius:11px;
    padding:9px 18px;cursor:pointer;border:1px solid transparent;transition:.15s}
  .btn.primary{background:var(--accent);color:#fff}
  .btn.primary:hover{background:var(--accent-deep)}
  .btn.ghost{background:var(--card);border-color:var(--line);color:var(--ink);
    box-shadow:var(--shadow)}
  .btn.ghost:hover{border-color:var(--accent);color:var(--accent-deep)}
  .btn:disabled{opacity:.5;cursor:default}
  .btn.small{font-size:12.5px;padding:5px 12px;border-radius:8px;font-weight:500}
  dialog{border:none;border-radius:18px;background:var(--card);color:var(--ink);
    box-shadow:0 30px 80px -20px rgba(0,0,0,.5);padding:22px 24px;width:min(430px,92vw)}
  dialog::backdrop{background:rgba(0,0,0,.35);backdrop-filter:blur(2px)}
  dialog h3{margin:0 0 14px;font-size:16px;font-weight:700}
  .field{margin-bottom:12px}
  .field label{display:block;font-size:12px;color:var(--muted);margin-bottom:5px}
  input[type=text],input[type=number],textarea{width:100%;background:var(--bg);
    border:1px solid var(--line);border-radius:10px;padding:8px 12px;font-size:14px;
    font-family:var(--sans);color:var(--ink)}
  input:focus,textarea:focus{outline:none;border-color:var(--accent)}
  textarea{height:110px;resize:vertical}
  .kv{display:flex;justify-content:space-between;align-items:baseline;gap:14px;
    padding:9px 0;border-bottom:1px solid var(--line);font-size:13.5px}
  .kv:last-of-type{border-bottom:none}
  .kv .k{color:var(--muted);flex:none}
  .kv .v{text-align:right;word-break:break-all;font-family:var(--mono);font-size:12.5px;min-width:0}
  .modalacts{display:flex;justify-content:flex-end;gap:9px;margin-top:16px}
  .chips-select{display:flex;flex-wrap:wrap;gap:8px;margin-bottom:12px}
  .pick{border:1px solid var(--line);background:var(--card);border-radius:999px;
    padding:6px 14px;font-size:13px;cursor:pointer;color:var(--ink);font-family:var(--sans);
    transition:.15s}
  .pick:hover{border-color:var(--accent)}
  .pick.on{border-color:var(--accent);background:var(--accent-soft);
    color:var(--accent-deep);font-weight:600}

  /* ── toast ── */
  #toasts{position:fixed;bottom:26px;left:50%;transform:translateX(-50%);z-index:99;
    display:flex;flex-direction:column;gap:8px;align-items:center;pointer-events:none}
  .toast{background:var(--ink);color:var(--bg);border-radius:12px;padding:9px 18px;
    font-size:13.5px;box-shadow:var(--shadow);opacity:0;translate:0 6px;
    transition:.25s;max-width:80vw}
  .toast.in{opacity:1;translate:0 0}
  .toast.err{background:var(--err);color:#fff}

  @media(max-width:640px){ .side{width:198px} .center{padding:20px 16px} }
</style>
</head>
<body>
<div class="app">

  <aside class="side">
    <div class="brand"><span class="bico">🐜</span><span>蚂蚁快传</span><span class="bsp"></span>
      <button class="sbtn" id="btn-settings" title="节点信息">⚙</button></div>
    <div class="side-scroll">
      <div class="label"><span>设备</span><span class="n" id="devcount"></span><span class="lsp"></span>
        <button class="sbtn" id="btn-add" title="手动添加设备（IP）">＋</button></div>
      <div id="devices"></div>
      <div class="devempty" id="devempty">等待设备上线<br><small>同一网络下自动发现</small></div>
    </div>
    <div class="side-foot">💡 把文件拖到设备上</div>
  </aside>

  <main class="main">

    <!-- 会话视图：选中设备后（头部 + 气泡流 + 聊天输入条） -->
    <section id="chatview" hidden>
      <header class="chead">
        <h2 id="ch-name"></h2>
        <span class="cmeta" id="ch-meta"></span>
      </header>
      <div class="cscroll" id="cscroll">
        <section id="busy" hidden></section>
        <div id="chatlist"></div>
      </div>
      <footer class="cfoot">
        <button class="attach" id="btn-attach" title="选择文件发送">📎</button>
        <input id="chatinput" type="text" placeholder="输入消息，Enter 发送…" autocomplete="off">
        <button class="sendbtn" id="btn-send">发送</button>
      </footer>
    </section>

    <!-- 等待屏：未选设备 -->
    <section id="waitview">
      <section class="center">
        <h1 id="c-title">等待设备上线</h1>
        <p class="sub" id="c-sub">同一 Wi-Fi 下的 LocalSend 设备会自动出现在左侧</p>
        <div class="dropzone" id="dropzone">📁 拖入文件即可发送</div>
        <div class="acts2">
          <button class="linkbtn strong" id="btn-file">选择文件</button>
          <span class="sep">·</span>
          <button class="linkbtn" id="btn-text">发送文本</button>
        </div>
      </section>

      <section id="rxsec" hidden>
        <div class="rxhead"><span>最近接收</span><span class="n" id="rxcount"></span></div>
        <ul class="received" id="received"></ul>
      </section>
    </section>

  </main>

</div>

<input type="file" id="filepick" multiple style="display:none">

<dialog id="pickdlg">
  <h3>发送给谁？</h3>
  <div class="chips-select" id="picklist"></div>
  <div class="modalacts"><button class="btn ghost" id="pickcancel">取消</button></div>
</dialog>

<dialog id="textdlg">
  <h3>发送文本</h3>
  <div class="chips-select" id="texttargets"></div>
  <textarea id="textbody" placeholder="输入要发送的文本…"></textarea>
  <div class="modalacts">
    <button class="btn ghost" id="textcancel">取消</button>
    <button class="btn primary" id="textsend">发送</button>
  </div>
</dialog>

<dialog id="adddlg">
  <h3>手动添加设备</h3>
  <div class="field"><label>IP 地址</label>
    <input type="text" id="ip" placeholder="192.168.1.5" autocomplete="off"></div>
  <div class="field"><label>端口</label><input type="number" id="port" value="53317"></div>
  <div class="modalacts">
    <button class="btn ghost" id="addcancel">取消</button>
    <button class="btn primary" id="addok">添加</button>
  </div>
</dialog>

<dialog id="setdlg">
  <h3>节点信息</h3>
  <div class="kv"><span class="k">别名</span><span class="v" id="st-alias" style="font-family:var(--sans);font-size:13.5px"></span></div>
  <div class="kv"><span class="k">端口</span><span class="v" id="st-port"></span></div>
  <div class="kv"><span class="k">指纹</span><span class="v" id="st-fp"></span></div>
  <div class="kv"><span class="k">保存目录</span><span class="v" id="st-dir" style="font-family:var(--sans);font-size:13px"></span></div>
  <div class="kv"><span class="k">协议</span><span class="v" id="st-ver"></span></div>
  <div class="modalacts">
    <button class="btn ghost small" id="st-copy">复制指纹</button>
    <button class="btn primary" id="st-close">关闭</button>
  </div>
</dialog>

<div id="toasts"></div>

<script>
'use strict';
const $ = id => document.getElementById(id);
const esc = s => String(s??'').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const fmtSize = n => { n=+n||0;
  if(n<1024) return n+' B';
  if(n<1048576) return (n/1024).toFixed(1)+' KB';
  if(n<1073741824) return (n/1048576).toFixed(1)+' MB';
  return (n/1073741824).toFixed(2)+' GB'; };
const fmtAgo = ms => ms<3000?'刚刚':ms<60000?Math.round(ms/1000)+' 秒前':ms<3600000?Math.round(ms/60000)+' 分钟前':Math.round(ms/3600000)+' 小时前';
const fmtTime = ms => new Date(ms).toLocaleTimeString('zh-CN',{hour12:false});

let lastReceivedAt = 0, lastDevJson = '', lastRxJson = '', firstRender = true;
let sel = null;            // 当前选中的设备指纹（null = 未选，主区显示等待屏）
let pendingTarget = null;  // filepick 的目标
let lastChatKey = '', lastChatSel = null; // 会话气泡防抖重绘 + 切换会话时强制贴底
const seenFp = new Set();

function toast(msg, err){
  const box = $('toasts');
  const t = document.createElement('div');
  t.className = 'toast' + (err ? ' err' : '');
  t.textContent = msg;
  box.appendChild(t);
  requestAnimationFrame(() => t.classList.add('in'));
  setTimeout(() => { t.classList.remove('in'); setTimeout(() => t.remove(), 300); }, 3200);
  while (box.children.length > 4) box.firstChild.remove();
}

function emoji(d){
  const t = d.deviceType || '', m = (d.deviceModel||'') + (d.alias||'');
  if (t === 'mobile' || /iPhone|iPad|手机/i.test(m)) return '📱';
  if (t === 'web') return '🌐';
  if (t === 'server') return '🖧';
  if (/Mac|笔记本|laptop/i.test(m)) return '💻';
  return '🖥️';
}

function hasFiles(e){
  return !!(e.dataTransfer && [...(e.dataTransfer.types||[])].includes('Files'));
}

function render(s){
  window._state = s;

  // ── 侧栏设备（内容有变化才重绘，避免打断 hover / 拖放） ──
  const dj = JSON.stringify(s.devices);
  if (dj !== lastDevJson){
    lastDevJson = dj;
    const box = $('devices');
    box.innerHTML = '';
    $('devempty').hidden = s.devices.length > 0;
    $('devcount').textContent = s.devices.length ? '· ' + s.devices.length : '';
    for (const d of s.devices){
      const row = document.createElement('div');
      row.className = 'dev' + (d.fingerprint === sel ? ' on' : '');
      row.dataset.fp = d.fingerprint; row.tabIndex = 0; row.title = d.alias;
      const ico = document.createElement('div'); ico.className = 'ico'; ico.textContent = emoji(d);
      const mid = document.createElement('div'); mid.className = 'mid';
      const nm = document.createElement('div'); nm.className = 'nm'; nm.textContent = d.alias;
      const meta = document.createElement('div'); meta.className = 'meta';
      meta.textContent = `${d.deviceType || '?'} · ${d.ip}`;
      mid.append(nm, meta); row.append(ico, mid);
      row.onclick = () => { sel = (sel === d.fingerprint ? null : d.fingerprint); syncSel(); };
      // 拖文件到设备行 = 发给这台设备
      row.ondragover = e => { if (hasFiles(e)){ e.preventDefault();
        e.dataTransfer.dropEffect = 'copy'; row.classList.add('dropover'); } };
      row.ondragleave = () => row.classList.remove('dropover');
      row.ondrop = e => { if (!hasFiles(e)) return;
        e.preventDefault(); e.stopPropagation();
        row.classList.remove('dropover');
        sel = d.fingerprint; syncSel();
        sendFiles(d.fingerprint, [...e.dataTransfer.files]); };
      box.appendChild(row);
    }
  }

  // 上线 / 重新上线提醒（首轮静默；离线超过服务端 300s 过滤后移除记录）
  const nowFp = new Set(s.devices.map(d => d.fingerprint));
  if (firstRender){ nowFp.forEach(f => seenFp.add(f)); firstRender = false; }
  else {
    for (const d of s.devices){
      if (!seenFp.has(d.fingerprint)){ seenFp.add(d.fingerprint); toast(`上线:${d.alias}`); }
    }
    for (const f of [...seenFp]) if (!nowFp.has(f)) seenFp.delete(f);
  }

  // 选中设备掉线 → 回到等待屏
  if (sel && !s.devices.some(d => d.fingerprint === sel)) sel = null;
  // 正在收文件且没开任何会话 → 自动切到发送者的会话（进度卡可见）
  if (!sel && s.session && s.session.active){
    const d = (s.devices||[]).find(x => x.ip === s.session.senderIp);
    if (d) sel = d.fingerprint;
  }
  renderCenter(s);
  renderChat(s);

  // ── 传输进行中 → 主区顶部进度卡 ──
  const busy = [];
  if (s.sending && s.sending.active){
    const p = s.sending, pct = p.total ? Math.min(100, p.sent/p.total*100) : 0;
    busy.push(`<div class="prog"><div class="row"><b>↑ 发送到「${esc(p.target_alias)}」· ${esc(p.file_name)}</b>
      <span class="sz">${fmtSize(p.sent)} / ${fmtSize(p.total)}</span></div>
      <div class="bar"><i style="width:${pct}%"></i></div></div>`);
  }
  if (s.session && s.session.active){
    const c = s.session.current || {};
    const pct = c.total ? Math.min(100, (c.got||0)/c.total*100) : 0;
    busy.push(`<div class="prog"><div class="row"><b>↓ 来自「${esc(s.session.sender)}」· ${esc(c.name||'')}</b>
      <span class="sz">${fmtSize(c.got)} / ${fmtSize(c.total)}</span></div>
      <div class="bar"><i style="width:${pct}%"></i></div>
      <ul class="files">${(s.session.files||[]).map(f =>
        `<li class="${f.done?'ok':''}"><span class="tick">${f.done?'✔':'○'}</span>${esc(f.name)}
         <span class="sz">${fmtSize(f.size)}</span></li>`).join('')}</ul></div>`);
  }
  $('busy').innerHTML = busy.join('');
  $('busy').hidden = !busy.length;

  // ── 最近接收（空则整节隐藏） ──
  const rj = JSON.stringify(s.received);
  if (rj !== lastRxJson){
    lastRxJson = rj;
    $('rxsec').hidden = !s.received.length;
    $('rxcount').textContent = s.received.length ? '· ' + s.received.length : '';
    const ul = $('received');
    ul.innerHTML = '';
    for (const r of s.received){
      const li = document.createElement('li');
      const nm = document.createElement('span'); nm.className = 'nm';
      nm.textContent = r.name; nm.title = r.name;
      const sz = document.createElement('span'); sz.className = 'sz';
      sz.textContent = `${fmtSize(r.size)} · ${fmtTime(r.at)}`;
      const b = document.createElement('button'); b.className = 'btn ghost small';
      b.textContent = '显示'; b.onclick = () => reveal(r.file);
      li.append(nm, sz, b);
      ul.appendChild(li);
    }
  }
  if (s.received.length && s.received[0].at > lastReceivedAt){
    if (lastReceivedAt)
      s.received.filter(r => r.at > lastReceivedAt).forEach(r => toast(`已接收:${r.name}`));
    lastReceivedAt = s.received[0].at;
  }
}

// 主区：选中设备 → 会话视图（气泡 + 聊天输入条）；未选 → 等待屏
function renderCenter(s){
  s = s || window._state; if (!s) return;
  const d = (s.devices||[]).find(x => x.fingerprint === sel);
  $('chatview').hidden = !d;
  $('waitview').hidden = !!d;
  if (d){
    $('ch-name').textContent = d.alias;
    $('ch-meta').textContent = `${d.deviceType || '?'} · ${d.ip} · ${fmtAgo(d.lastSeenMs)}可见`;
  } else {
    $('c-title').textContent = '等待设备上线';
    $('c-sub').textContent = s.devices.length
      ? '点击左侧的设备开始发送'
      : '同一 Wi-Fi 下的 LocalSend 设备会自动出现在左侧';
    $('dropzone').textContent = '📁 拖入文件即可发送';
  }
}

// 会话气泡流（按选中设备的指纹过滤；内容无变化不重绘）
function renderChat(s){
  const sc = $('cscroll'), box = $('chatlist');
  if (!s || !sel){ lastChatKey = ''; box.innerHTML = ''; return; }
  const msgs = (s.chat||[]).filter(m => m.peer === sel);
  const key = sel + ':' + msgs.map(m => m.id).join(',');
  const switched = lastChatSel !== sel;
  if (key === lastChatKey && !switched) return;
  lastChatKey = key; lastChatSel = sel;
  const nearBottom = sc.scrollHeight - sc.scrollTop - sc.clientHeight < 90;
  box.innerHTML = '';
  if (!msgs.length){
    const d = (s.devices||[]).find(x => x.fingerprint === sel);
    const hint = document.createElement('div'); hint.className = 'chathint';
    hint.innerHTML = `与「${esc(d ? d.alias : '对方')}」的对话会显示在这里` +
      '<br><small>拖入文件、点 📎 选文件，或直接输入文字发送</small>';
    box.appendChild(hint);
  }
  for (const m of msgs){
    const row = document.createElement('div');
    row.className = 'msg ' + (m.out ? 'out' : 'in');
    const bub = document.createElement('div'); bub.className = 'bub';
    if (m.kind === 'text'){
      bub.textContent = m.text;
    } else {
      const chip = document.createElement('div'); chip.className = 'fchip';
      const fi = document.createElement('span'); fi.className = 'fico'; fi.textContent = '📄';
      const mid = document.createElement('div'); mid.className = 'fmid';
      const fn = document.createElement('div'); fn.className = 'fnm';
      fn.textContent = m.name; fn.title = m.name;
      const fs = document.createElement('div'); fs.className = 'fsz';
      fs.textContent = fmtSize(m.size);
      mid.append(fn, fs); chip.append(fi, mid); bub.appendChild(chip);
      if (!m.out && m.file){ // 收到的文件可在 Finder / 资源管理器中定位
        const b = document.createElement('button'); b.className = 'rxb';
        b.textContent = '显示'; b.onclick = () => reveal(m.file);
        bub.appendChild(b);
      }
    }
    const t = document.createElement('div'); t.className = 't';
    t.textContent = fmtTime(m.at);
    row.append(bub, t);
    box.appendChild(row);
  }
  if (nearBottom || switched) sc.scrollTop = sc.scrollHeight;
}

function syncSel(){
  document.querySelectorAll('.dev').forEach(el =>
    el.classList.toggle('on', el.dataset.fp === sel));
  renderCenter(window._state);
  renderChat(window._state);
}

async function poll(){
  try{
    const r = await fetch('/api/ui/state');
    if (r.ok) render(await r.json());
  }catch(_){ /* 服务重启中 */ }
  setTimeout(poll, 1200);
}

// ── 发送 ──
async function sendFiles(fp, files){
  sel = fp; syncSel(); // 发送即切到目标会话（进度卡在会话流顶部可见）
  for (const f of files){
    try{
      const q = `target=${encodeURIComponent(fp)}&name=${encodeURIComponent(f.name)}`+
                `&size=${f.size}&mime=${encodeURIComponent(f.type||'application/octet-stream')}`;
      const r = await fetch('/api/ui/send?' + q, {method:'POST', body:f});
      const v = await r.json().catch(()=>({}));
      if (r.ok) toast(`已送达:${f.name}`);
      else { toast(v.error || '发送失败', true); break; }
    }catch(e){ toast('发送失败:' + e.message, true); break; }
  }
}

function pickThenFile(fp){ pendingTarget = fp; $('filepick').value = ''; $('filepick').click(); }
$('filepick').onchange = async () => {
  const files = [...$('filepick').files];
  if (files.length && pendingTarget) sendFiles(pendingTarget, files);
  pendingTarget = null;
};

// 目标解析：已选中 → 它；仅 1 台 → 它；多台 → 弹选择；0 台 → 报错
function resolveTarget(cb){
  const ds = (window._state && window._state.devices) || [];
  if (!ds.length) return toast('还没有可用设备', true);
  if (sel) return cb(sel);
  if (ds.length === 1) return cb(ds[0].fingerprint);
  openPick(cb);
}

// 多台时弹胶囊选择（选择后回调目标指纹）
function openPick(cb){
  const ds = (window._state && window._state.devices) || [];
  if (!ds.length) return toast('还没有可用设备', true);
  const list = $('picklist');
  list.innerHTML = '';
  for (const d of ds){
    const b = document.createElement('button'); b.className = 'pick';
    b.textContent = `${emoji(d)} ${d.alias}`;
    b.onclick = () => { $('pickdlg').close(); cb(d.fingerprint); };
    list.appendChild(b);
  }
  $('pickdlg').showModal();
}
$('pickcancel').onclick = () => $('pickdlg').close();

$('btn-file').onclick = () => resolveTarget(fp => pickThenFile(fp));

// 主区拖放区：松开即按目标解析发送
const dz = $('dropzone');
dz.ondragover = e => { if (hasFiles(e)){ e.preventDefault();
  e.dataTransfer.dropEffect = 'copy'; dz.classList.add('dropover'); } };
dz.ondragleave = () => dz.classList.remove('dropover');
dz.ondrop = e => { if (!hasFiles(e)) return;
  e.preventDefault(); e.stopPropagation(); dz.classList.remove('dropover');
  const files = [...e.dataTransfer.files];
  resolveTarget(fp => sendFiles(fp, files)); };

// ── 会话视图：拖文件进会话区 = 发给当前设备 ──
const cv = $('chatview');
cv.addEventListener('dragover', e => { if (hasFiles(e)) e.preventDefault(); });
cv.addEventListener('drop', e => { if (!hasFiles(e)) return;
  e.preventDefault(); e.stopPropagation();
  const files = [...e.dataTransfer.files];
  if (sel) sendFiles(sel, files);
  else resolveTarget(fp => sendFiles(fp, files));
});

// ── 聊天输入条：Enter 发送（失败回填输入框） ──
function sendChatText(){
  const inp = $('chatinput');
  const text = inp.value.trim();
  if (!text || !sel || inp.disabled) return;
  inp.value = ''; inp.disabled = true;
  fetch('/api/ui/send-text', {method:'POST', headers:{'Content-Type':'application/json'},
    body: JSON.stringify({target: sel, text})})
    .then(async r => {
      if (!r.ok){
        const v = await r.json().catch(() => ({}));
        toast(v.error || '发送失败', true);
        inp.value = text;
      }
    })
    .catch(e => { toast('发送失败:' + e.message, true); inp.value = text; })
    .finally(() => { inp.disabled = false; inp.focus(); });
}
$('btn-send').onclick = sendChatText;
$('chatinput').addEventListener('keydown', e => {
  if (e.key === 'Enter'){ e.preventDefault(); sendChatText(); }
});

// 📎 = 选择文件发给当前会话（未选设备则先解析目标）
$('btn-attach').onclick = () => {
  if (sel) pickThenFile(sel);
  else resolveTarget(fp => pickThenFile(fp));
};

// 发文本（目标以胶囊选择，从选中设备/按钮进入时预选）
function openText(preselect){
  const ds = (window._state && window._state.devices) || [];
  if (!ds.length) return toast('还没有可用设备', true);
  const box = $('texttargets');
  box.innerHTML = '';
  let cur = preselect || sel || (ds[0] && ds[0].fingerprint);
  const syncs = ds.map(d => {
    const b = document.createElement('button'); b.className = 'pick';
    b.textContent = `${emoji(d)} ${d.alias}`;
    const sync = () => b.classList.toggle('on', cur === d.fingerprint);
    b.onclick = () => { cur = d.fingerprint; syncs.forEach(f => f()); };
    box.appendChild(b); sync();
    return sync;
  });
  $('textbody').value = '';
  $('textdlg').showModal();
  $('textbody').focus();
  $('textsend').onclick = async () => {
    const text = $('textbody').value.trim();
    if (!text) return;
    $('textsend').disabled = true;
    try{
      const r = await fetch('/api/ui/send-text', {method:'POST',
        headers:{'Content-Type':'application/json'},
        body: JSON.stringify({target: cur, text})});
      const v = await r.json().catch(()=>({}));
      if (r.ok){ toast('文本已送达'); $('textdlg').close(); }
      else toast(v.error || '发送失败', true);
    }catch(e){ toast('发送失败:' + e.message, true); }
    $('textsend').disabled = false;
  };
}
$('textcancel').onclick = () => $('textdlg').close();
$('btn-text').onclick = () => openText(null);

// 手动添加
$('btn-add').onclick = () => $('adddlg').showModal();
$('addcancel').onclick = () => $('adddlg').close();
$('addok').onclick = async () => {
  const ip = $('ip').value.trim(), port = +$('port').value || 53317;
  if (!ip) return toast('请填 IP', true);
  try{
    const r = await fetch('/api/ui/add', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({ip, port})});
    const v = await r.json().catch(()=>({}));
    if (r.ok){ toast(`已添加:${v.alias}`); $('adddlg').close(); $('ip').value=''; }
    else toast(v.error || '添加失败', true);
  }catch(e){ toast('添加失败:' + e.message, true); }
};

// 在文件管理器中显示
async function reveal(file){
  const r = await fetch('/api/ui/reveal', {method:'POST',
    headers:{'Content-Type':'application/json'}, body: JSON.stringify({file})});
  if (!r.ok){ const v = await r.json().catch(()=>({})); toast(v.error || '打开失败', true); }
}

// 节点信息
$('btn-settings').onclick = () => {
  const me = window._state && window._state.me;
  if (!me) return;
  $('st-alias').textContent = me.alias;
  $('st-port').textContent = me.port;
  $('st-fp').textContent = me.fingerprint;
  $('st-dir').textContent = me.dir;
  $('st-ver').textContent = 'LocalSend v' + me.version;
  $('setdlg').showModal();
};
$('st-close').onclick = () => $('setdlg').close();
$('st-copy').onclick = () => {
  const t = $('st-fp').textContent;
  const done = () => toast('指纹已复制');
  if (navigator.clipboard) navigator.clipboard.writeText(t).then(done).catch(() => fallbackCopy(t, done));
  else fallbackCopy(t, done);
};
function fallbackCopy(t, done){
  const ta = document.createElement('textarea');
  ta.value = t; ta.style.position = 'fixed'; ta.style.opacity = '0';
  document.body.appendChild(ta); ta.select();
  try{ document.execCommand('copy'); done(); }catch(_){ toast('复制失败', true); }
  ta.remove();
}

// ── 全局拖拽：拖动中高亮所有可投放目标（侧栏设备行 + 主区拖放区） ──
let depth = 0;
document.addEventListener('dragenter', e => {
  if (hasFiles(e)){ e.preventDefault(); depth++; document.body.classList.add('dropping'); }
});
document.addEventListener('dragover', e => { if (hasFiles(e)) e.preventDefault(); });
document.addEventListener('dragleave', e => {
  if (hasFiles(e)){ depth = Math.max(0, depth-1);
    if (!depth) document.body.classList.remove('dropping'); }
});
document.addEventListener('drop', e => {
  if (hasFiles(e)){ e.preventDefault(); depth = 0; document.body.classList.remove('dropping'); }
});

poll();
</script>
</body>
</html>"##;
