//! 面板页面（内嵌 HTML，无外部依赖，离线可用）
pub const DASHBOARD: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AntifyBot · LocalSend 节点</title>
<style>
  :root{
    --paper:#f4ead8; --panel:#fffaf0; --ink:#35281a; --muted:#8a7660;
    --line:#e3d5bc; --amber:#d98324; --amber-deep:#b06a15; --green:#5e8c4a;
    --red:#c0563e; --shadow:0 10px 30px -18px rgba(80,55,20,.45);
    --serif:"Iowan Old Style","Palatino Linotype","Songti SC",Georgia,serif;
    --sans:-apple-system,"PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif;
    --mono:"SF Mono",Menlo,Consolas,monospace;
  }
  *{box-sizing:border-box}
  body{margin:0;background:
      radial-gradient(1200px 500px at 85% -10%, #f9e2b8 0%, transparent 60%),
      radial-gradient(900px 400px at -10% 110%, #eddcc0 0%, transparent 55%),
      var(--paper);
    color:var(--ink);font-family:var(--sans);line-height:1.55}
  main{max-width:1020px;margin:0 auto;padding:28px 20px 90px}
  header{display:flex;flex-wrap:wrap;align-items:baseline;gap:14px;margin-bottom:6px}
  .logo{font-size:44px;line-height:1;transform:translateY(6px)}
  h1{font-family:var(--serif);font-size:30px;margin:0;letter-spacing:.5px;font-weight:700}
  h1 small{font-size:14px;color:var(--muted);font-weight:400;margin-left:10px;letter-spacing:0}
  .chips{display:flex;flex-wrap:wrap;gap:8px;margin:10px 0 26px}
  .chip{background:var(--panel);border:1px solid var(--line);border-radius:999px;
    padding:4px 13px;font-size:12.5px;color:var(--muted);box-shadow:var(--shadow)}
  .chip b{color:var(--ink);font-weight:600}
  .chip .mono{font-family:var(--mono);font-size:11.5px}
  h2{font-family:var(--serif);font-size:19px;margin:34px 0 12px;display:flex;align-items:center;gap:9px}
  h2 .count{font-family:var(--sans);font-size:12px;background:var(--amber);color:#fff;
    border-radius:999px;padding:1px 9px;font-weight:600}
  h2::after{content:"";flex:1;height:1px;background:linear-gradient(90deg,var(--line),transparent)}
  .cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(295px,1fr));gap:14px}
  .card{background:var(--panel);border:1px solid var(--line);border-radius:16px;
    padding:15px 17px;box-shadow:var(--shadow);transition:transform .18s,box-shadow .18s}
  .card:hover{transform:translateY(-2px)}
  .card .who{display:flex;align-items:center;gap:10px}
  .card .who .dot{width:9px;height:9px;border-radius:50%;background:var(--green);
    box-shadow:0 0 0 4px rgba(94,140,74,.18);flex:none}
  .card .alias{font-weight:700;font-size:16px;font-family:var(--serif)}
  .card .meta{font-size:12px;color:var(--muted);margin-top:7px;font-family:var(--mono)}
  .card .acts{display:flex;gap:8px;margin-top:13px}
  button{font-family:var(--sans);cursor:pointer;border-radius:10px;font-size:13.5px;
    padding:7px 14px;border:1px solid var(--line);background:#fdf3e2;color:var(--ink);
    transition:all .15s}
  button:hover{border-color:var(--amber);color:var(--amber-deep)}
  button.primary{background:var(--amber);border-color:var(--amber);color:#fff;font-weight:600}
  button.primary:hover{background:var(--amber-deep);border-color:var(--amber-deep);color:#fff}
  button:disabled{opacity:.45;cursor:default}
  .bar{height:9px;background:#ecdfc6;border-radius:99px;overflow:hidden;margin:8px 0}
  .bar i{display:block;height:100%;background:linear-gradient(90deg,var(--amber),#e8b05c);
    border-radius:99px;transition:width .3s}
  .prog{background:var(--panel);border:1px solid var(--line);border-radius:16px;
    padding:14px 17px;box-shadow:var(--shadow);margin-bottom:14px}
  .prog .row{display:flex;justify-content:space-between;gap:10px;font-size:13.5px}
  .prog .row b{font-weight:600}
  .files li{list-style:none;padding:5px 0;font-size:13.5px;display:flex;gap:8px;align-items:center}
  .files .ok{color:var(--green)} .files .wait{color:var(--muted)}
  ul.files,ul.received,ul.events{margin:0;padding:0}
  .received li{list-style:none;display:flex;align-items:center;gap:10px;padding:9px 2px;
    border-bottom:1px dashed var(--line);font-size:13.5px}
  .received li:last-child{border-bottom:none}
  .received .nm{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .received .sz{color:var(--muted);font-size:12px;font-family:var(--mono)}
  .received button{padding:3px 10px;font-size:12px}
  .events{font-family:var(--mono);font-size:12px;color:var(--muted)}
  .events li{list-style:none;padding:3.5px 0;border-bottom:1px dotted #eee2ca}
  .events .t{color:var(--amber-deep);margin-right:8px}
  .empty{color:var(--muted);font-size:13.5px;padding:18px;text-align:center;
    border:1.5px dashed var(--line);border-radius:14px;background:rgba(255,250,240,.55)}
  .addrow{display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin-bottom:6px}
  .addrow input{background:var(--panel);border:1px solid var(--line);border-radius:10px;
    padding:7px 12px;font-size:13.5px;color:var(--ink);font-family:var(--mono)}
  .addrow input:focus{outline:none;border-color:var(--amber)}
  #ip{width:170px} #port{width:80px} #alias{width:150px}
  .hint{font-size:12px;color:var(--muted);margin-top:4px}
  dialog{border:none;border-radius:18px;background:var(--panel);color:var(--ink);
    box-shadow:0 30px 80px -20px rgba(60,40,10,.5);padding:22px 24px;width:min(440px,92vw)}
  dialog::backdrop{background:rgba(53,40,26,.45);backdrop-filter:blur(3px)}
  dialog h3{font-family:var(--serif);margin:0 0 12px;font-size:19px}
  textarea{width:100%;height:120px;resize:vertical;border:1px solid var(--line);
    border-radius:12px;padding:10px 12px;font-size:14px;font-family:var(--sans);
    background:#fffdf8;color:var(--ink)}
  textarea:focus{outline:none;border-color:var(--amber)}
  .modalacts{display:flex;justify-content:flex-end;gap:9px;margin-top:15px}
  #toast{position:fixed;top:18px;left:50%;transform:translateX(-50%);z-index:99;
    background:var(--ink);color:#fff;border-radius:12px;padding:9px 20px;font-size:13.5px;
    opacity:0;pointer-events:none;transition:opacity .25s,translate .25s;max-width:86vw}
  #toast.show{opacity:1;translate:0 4px}
  #toast.err{background:var(--red)}
  footer{margin-top:44px;text-align:center;font-size:12px;color:var(--muted)}
  @media(max-width:640px){ .logo{font-size:36px} h1{font-size:24px} main{padding:20px 14px 70px} }
</style>
</head>
<body>
<main>
  <header>
    <span class="logo">🐜</span>
    <h1 id="title">AntifyBot <small>LocalSend v2 节点</small></h1>
  </header>
  <div class="chips" id="chips"></div>

  <h2>附近的设备 <span class="count" id="devcount">0</span></h2>
  <div class="cards" id="devices"></div>
  <div class="addrow" style="margin-top:14px">
    <input id="ip" placeholder="IP，如 192.168.1.5">
    <input id="port" placeholder="端口" value="53317">
    <button id="btn-add" class="primary">手动添加</button>
    <span class="hint">对方不在线时，直接填 IP 添加（需对方也是 LocalSend 节点）</span>
  </div>

  <h2>传输</h2>
  <div id="transfer"><div class="empty">暂无进行中的传输</div></div>

  <h2>已接收 <span class="count" id="rxcount">0</span></h2>
  <ul class="received" id="received"></ul>

  <h2>事件日志</h2>
  <ul class="events" id="events"></ul>

  <footer>antify-rs · 与 LocalSend 官方 App 互通 · 自签证书（本页面需要先在浏览器放行）</footer>
</main>

<input type="file" id="filepick" multiple style="display:none">
<dialog id="textdlg">
  <h3>发送文本 → <span id="textwho"></span></h3>
  <textarea id="textbody" placeholder="输入要发送的文本…"></textarea>
  <div class="modalacts">
    <button id="textcancel">取消</button>
    <button id="textsend" class="primary">发送</button>
  </div>
</dialog>
<div id="toast"></div>

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

let lastReceivedAt = 0;
function toast(msg, err){
  const t = $('toast');
  t.textContent = msg; t.className = 'show' + (err ? ' err' : '');
  clearTimeout(t._h); t._h = setTimeout(()=>t.className='', 2600);
}

function render(s){
  const me = s.me;
  $('title').innerHTML = `🐜 AntifyBot — ${esc(me.alias)} <small>LocalSend v2 节点</small>`;
  $('chips').innerHTML = `
    <span class="chip">别名 <b>${esc(me.alias)}</b></span>
    <span class="chip">端口 <b>${me.port}</b></span>
    <span class="chip">指纹 <b class="mono">${esc(me.fingerprint.slice(0,10))}…</b></span>
    <span class="chip">保存到 <b>${esc(me.dir)}</b></span>`;

  // 设备
  $('devcount').textContent = s.devices.length;
  $('devices').innerHTML = s.devices.length ? s.devices.map(d => `
    <div class="card">
      <div class="who"><span class="dot"></span>
        <span class="alias">${esc(d.alias)}</span></div>
      <div class="meta">${esc(d.deviceType)} · ${esc(d.addr)} · ${esc(d.version)}<br>
        ${fmtAgo(d.lastSeenMs)}可见</div>
      <div class="acts">
        <button class="primary" onclick="sendText('${esc(d.fingerprint)}')">发文本</button>
        <button onclick="sendFile('${esc(d.fingerprint)}')">发文件</button>
      </div>
    </div>`).join('') :
    `<div class="empty" style="grid-column:1/-1">还没有发现设备 —— 打开 LocalSend 官方 App，或等对方上线（自动发现约需 2 秒）</div>`;

  // 传输
  let tr = '';
  if (s.sending && s.sending.active){
    const p = s.sending, pct = p.total ? Math.min(100, p.sent/p.total*100) : 0;
    tr += `<div class="prog"><div class="row"><b>→ ${esc(p.target_alias)}：${esc(p.file_name)}</b>
      <span class="sz">${fmtSize(p.sent)} / ${fmtSize(p.total)}</span></div>
      <div class="bar"><i style="width:${pct}%"></i></div></div>`;
  }
  if (s.session && s.session.active){
    const c = s.session.current || {};
    const pct = c.total ? Math.min(100, (c.got||0)/c.total*100) : 0;
    tr += `<div class="prog"><div class="row"><b>← ${esc(s.session.sender)}：${esc(c.name||'')}</b>
      <span>${fmtSize(c.got)} / ${fmtSize(c.total)}</span></div>
      <div class="bar"><i style="width:${pct}%"></i></div>
      <ul class="files">${(s.session.files||[]).map(f =>
        `<li>${f.done?'<span class="ok">✔</span>':'<span class="wait">○</span>'} ${esc(f.name)}
         <span class="sz">${fmtSize(f.size)}</span></li>`).join('')}</ul></div>`;
  }
  $('transfer').innerHTML = tr || '<div class="empty">暂无进行中的传输</div>';

  // 已接收
  $('rxcount').textContent = s.received.length;
  $('received').innerHTML = s.received.length ? s.received.map(r => `
    <li><span>📄</span><span class="nm" title="${esc(r.name)}">${esc(r.name)}</span>
      <span class="sz">${fmtSize(r.size)} · ${fmtTime(r.at)}</span>
      <button onclick="reveal('${esc(r.file)}')">显示</button></li>`).join('') :
    '<div class="empty">还没有收到文件</div>';

  // 提示新文件
  if (s.received.length && s.received[0].at > lastReceivedAt){
    if (lastReceivedAt) toast(`已接收：${s.received[0].name}`);
    lastReceivedAt = s.received[0].at;
  }

  // 事件
  $('events').innerHTML = s.events.length ? s.events.map(e =>
    `<li><span class="t">${fmtTime(e.t)}</span>${esc(e.text)}</li>`).join('') :
    '<div class="empty">无事件</div>';
}

async function poll(){
  try{
    const r = await fetch('/api/ui/state');
    if (r.ok) render(await r.json());
  }catch(_){ /* 服务重启中 */ }
  setTimeout(poll, 1200);
}

// ---- 发送 ----
window.sendText = fp => {
  const s = window._state; const d = s && s.devices.find(x=>x.fingerprint===fp);
  $('textwho').textContent = d ? d.alias : '';
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
        body: JSON.stringify({target: fp, text})});
      const v = await r.json().catch(()=>({}));
      if (r.ok){ toast(`已送达「${$('textwho').textContent}」`); $('textdlg').close(); }
      else toast(v.error||'发送失败', true);
    }catch(e){ toast('发送失败：'+e.message, true); }
    $('textsend').disabled = false;
  };
};
$('textcancel').onclick = () => $('textdlg').close();

let pendingTarget = null;
window.sendFile = fp => { pendingTarget = fp; $('filepick').value=''; $('filepick').click(); };
$('filepick').onchange = async () => {
  const files = [...$('filepick').files];
  if (!files.length || !pendingTarget) return;
  for (const f of files){
    toast(`正在发送 ${f.name}…`);
    try{
      const q = `target=${encodeURIComponent(pendingTarget)}&name=${encodeURIComponent(f.name)}`+
                `&size=${f.size}&mime=${encodeURIComponent(f.type||'application/octet-stream')}`;
      const r = await fetch('/api/ui/send?'+q, {method:'POST', body:f});
      const v = await r.json().catch(()=>({}));
      if (r.ok) toast(`已送达：${f.name}`);
      else { toast(v.error||'发送失败', true); break; }
    }catch(e){ toast('发送失败：'+e.message, true); break; }
  }
  pendingTarget = null;
};

window.reveal = async file => {
  const r = await fetch('/api/ui/reveal', {method:'POST',
    headers:{'Content-Type':'application/json'}, body: JSON.stringify({file})});
  if (!r.ok){ const v = await r.json().catch(()=>({})); toast(v.error||'打开失败', true); }
};

$('btn-add').onclick = async () => {
  const ip = $('ip').value.trim(), port = +$('port').value || 53317;
  if (!ip){ toast('请填 IP', true); return; }
  try{
    const r = await fetch('/api/ui/add', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({ip, port})});
    const v = await r.json().catch(()=>({}));
    if (r.ok){ toast(`已添加「${v.alias}」`); $('ip').value=''; }
    else toast(v.error||'添加失败', true);
  }catch(e){ toast('添加失败：'+e.message, true); }
};

// 状态留一份给交互用
const _origRender = render;
window.render = s => { window._state = s; _origRender(s); };
poll();
</script>
</body>
</html>"##;
