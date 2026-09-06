//! 面板页面（内嵌 HTML，无外部依赖，离线可用）
//! 左右分栏：深色侧栏（品牌 + 设备列表 + 拖放提示），暖白主区。
//! 设备上线 → 左侧列表（微信会话列表式：第二行显示最近一条消息预览）；
//! 无设备 → 两侧同时提示「等待设备上线」；
//! 点选设备 → 会话视图（参考微信「文件传输助手」）：文字为彩色气泡、图片直接显示
//! 缩略图（点击全屏查看，加载失败退回文件卡片）、文件为中性卡片，
//! 头像在每条消息最外侧、时间按间隔居中分组；收到的文件带「显示」、文字带「复制」；
//! 活动传输为会话流内吸顶进度卡，最下面是微信式输入框：一个盒子内上为输入行、
//! 下为工具行 —— 左下角 文件 / 文件夹 / 剪贴板 三个图标按钮，右侧圆形图标发送。
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
  /* 离线设备：保留可见但置灰排后；hover 出 ✕ 移除按钮 */
  .dev.off{opacity:.55}
  .dev .xbtn{flex:none;width:20px;height:20px;border:none;border-radius:6px;
    background:transparent;color:var(--side-muted);font-size:11px;line-height:1;
    display:none;place-items:center;cursor:pointer;transition:.15s}
  .dev:hover .xbtn{display:grid}
  .dev .xbtn:hover{background:rgba(255,99,72,.28);color:#ff8a80}
  /* 出站消息状态行（发送中 / 已送达 / 未送达+重试） */
  .st{margin-top:3px;font-size:11px;color:var(--muted)}
  .st .rbtn{border:none;background:transparent;padding:0;font:inherit;font-weight:600;
    color:var(--err);cursor:pointer}
  .st .rbtn:hover{text-decoration:underline}
  .devempty{margin:12px 8px;padding:20px 10px;text-align:center;font-size:12.5px;
    color:var(--side-muted);border:1.5px dashed var(--side-line);border-radius:12px}
  .devempty small{font-size:11px;opacity:.8}

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
  /* 气泡（微信文件传输助手式）：头像在最外侧、时间居中分组、文件用中性卡片 */
  .tm{align-self:center;font-size:11px;color:var(--muted);margin:2px 0}
  .crow{display:flex;gap:10px;align-items:flex-start}
  .crow.out{flex-direction:row-reverse}
  .ava{flex:none;width:34px;height:34px;border-radius:9px;background:var(--card);
    border:1px solid var(--line);display:grid;place-items:center;font-size:17px}
  .crow.out .ava{background:var(--side);border-color:transparent}
  .msgcol{min-width:0;max-width:min(76%,540px);display:flex;flex-direction:column;
    align-items:flex-start}
  .crow.out .msgcol{align-items:flex-end}
  .bub{padding:9px 14px;border-radius:14px;font-size:14px;line-height:1.55;
    text-align:left;white-space:pre-wrap;word-break:break-word}
  .crow.in .bub{background:var(--card);border:1px solid var(--line);
    box-shadow:var(--shadow);border-top-left-radius:5px}
  .crow.out .bub{background:var(--accent);color:#fff;border-top-right-radius:5px}
  /* 文件消息：无论方向都是中性卡片（选择器带 .crow 抬高优先级，压过出站彩色气泡） */
  .crow .bub.file{background:var(--card);color:var(--ink);border:1px solid var(--line);
    box-shadow:var(--shadow);border-radius:14px}
  .fchip{display:flex;align-items:center;gap:9px;min-width:0}
  .fchip .fico{font-size:20px;line-height:1;flex:none}
  .fchip .fmid{min-width:0}
  .fchip .fnm{font-weight:600;font-size:13.5px;max-width:240px;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .fchip .fsz{font-size:11px;opacity:.75;font-family:var(--mono)}
  /* 图片消息：直接渲染缩略图（点击全屏查看）；加载失败由 JS 退回文件卡片 */
  .crow .bub.img{padding:4px}
  .bub img.chatimg{display:block;max-width:min(320px,72vw);max-height:320px;
    border-radius:10px;cursor:zoom-in;background:var(--bg)}
  #lightbox{position:fixed;inset:0;z-index:98;background:rgba(0,0,0,.82);
    display:grid;place-items:center;cursor:zoom-out;padding:24px}
  #lightbox[hidden]{display:none}
  #lightbox img{max-width:92vw;max-height:92vh;border-radius:8px;
    box-shadow:0 24px 80px rgba(0,0,0,.6)}
  .rxb{margin-top:6px;border:none;background:transparent;cursor:pointer;
    font-family:var(--sans);font-size:11.5px;color:var(--accent-deep);
    padding:0;font-weight:600}
  .rxb:hover{text-decoration:underline}
  .cfoot{flex:none;padding:12px 26px 16px;border-top:1px solid var(--line)}
  /* 微信式输入框：盒子内上为输入行、下为工具行（左：文件/文件夹/剪贴板；右：发送） */
  .composer{display:flex;flex-direction:column;background:var(--card);
    border:1px solid var(--line);border-radius:14px;box-shadow:var(--shadow);
    transition:border-color .15s}
  .composer:focus-within{border-color:var(--accent)}
  #chatinput{border:none;background:transparent;outline:none;padding:11px 14px 3px;
    font-size:14px;font-family:var(--sans);color:var(--ink)}
  #chatinput::placeholder{color:var(--muted)}
  #chatinput:disabled{opacity:.6}
  .ctools{display:flex;align-items:center;gap:2px;padding:4px 8px 8px}
  .tool{width:30px;height:30px;border-radius:8px;border:none;background:transparent;
    color:var(--muted);display:grid;place-items:center;cursor:pointer;transition:.15s}
  .tool:hover{background:var(--accent-soft);color:var(--accent-deep)}
  .tool svg{width:17px;height:17px;display:block}
  .ctools .tsp{flex:1}
  .tool.send{background:var(--accent);color:#fff;border-radius:50%}
  .tool.send:hover{background:var(--accent-deep);color:#fff}

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
        <div class="composer">
          <input id="chatinput" type="text" placeholder="输入消息，Enter 发送…" autocomplete="off">
          <div class="ctools">
            <button class="tool" id="btn-cfile" title="选择文件发送">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></svg>
            </button>
            <button class="tool" id="btn-cfolder" title="选择文件夹发送（全部文件）">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
            </button>
            <button class="tool" id="btn-cclip" title="发送剪贴板内容（截图 / 文本）">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/></svg>
            </button>
            <span class="tsp"></span>
            <button class="tool send" id="btn-send" title="发送">
              <svg viewBox="0 0 24 24" fill="currentColor"><path d="M2.01 21 23 12 2.01 3 2 10l15 2-15 2z"/></svg>
            </button>
          </div>
        </div>
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
<input type="file" id="folderpick" webkitdirectory multiple style="display:none">

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

<div id="lightbox" hidden><img alt=""></div>
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
const fmtAgo = ms => ms<3000?'刚刚':ms<60000?Math.round(ms/1000)+' 秒前':ms<3600000?Math.round(ms/60000)+' 分钟前'
  :ms<86400000?Math.round(ms/3600000)+' 小时前':ms<30*86400000?Math.round(ms/86400000)+' 天前'
  :new Date(Date.now()-ms).toLocaleDateString('zh-CN');
const fmtTime = ms => new Date(ms).toLocaleTimeString('zh-CN',{hour12:false});
// 图片判定（与服务端 /api/ui/asset 的扩展名白名单保持一致）
const IMG_RE = /\.(png|jpe?g|jfif|gif|webp|bmp|heic|heif|avif)$/i;
const isImg = n => IMG_RE.test(String(n || ''));

let lastReceivedAt = 0, lastDevJson = '', lastRxJson = '', firstRender = true;
let sel = null;            // 当前选中的设备指纹（null = 未选，主区显示等待屏）
let pendingTarget = null;  // filepick 的目标
let lastChatKey = '', lastChatSel = null; // 会话气泡防抖重绘 + 切换会话时强制贴底
const onlineMap = new Map(); // fp → 上一轮在线态（翻转时提醒上线/离线）

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

// 图片全屏查看：点缩略图放大，点击任意处 / Esc 关闭
const lb = $('lightbox'), lbimg = lb.querySelector('img');
function zoomImg(src, name){
  lbimg.src = src; lbimg.alt = name || '';
  lb.hidden = false;
}
lb.onclick = () => { lb.hidden = true; lbimg.src = ''; };
document.addEventListener('keydown', e => {
  if (e.key === 'Escape' && !lb.hidden){ lb.hidden = true; lbimg.src = ''; }
});

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
  // 微信会话列表式：第二行显示与该设备的最近一条消息预览；离线设备置灰保留
  const lastByFp = {};
  for (const m of (s.chat||[])) lastByFp[m.peer] = m;
  // 防抖 key：lastSeenMs 每轮轮询都在变，按分钟分桶；identity/在线态/预览 id 变了才重绘
  const dj = s.devices.map(d =>
    d.fingerprint + ':' + d.alias + ':' + d.ip + ':' + (d.online?1:0) + ':' +
    Math.floor(d.lastSeenMs/60000) + ':' + ((lastByFp[d.fingerprint]||{}).id||0)
  ).join('|');
  if (dj !== lastDevJson){
    lastDevJson = dj;
    const box = $('devices');
    box.innerHTML = '';
    $('devempty').hidden = s.devices.length > 0;
    const onlineN = s.devices.filter(d => d.online).length;
    $('devcount').textContent = s.devices.length
      ? `· 在线${onlineN}/${s.devices.length}` : '';
    for (const d of s.devices){
      const row = document.createElement('div');
      row.className = 'dev' + (d.fingerprint === sel ? ' on' : '') + (d.online ? '' : ' off');
      row.dataset.fp = d.fingerprint; row.tabIndex = 0; row.title = d.alias;
      const ico = document.createElement('div'); ico.className = 'ico'; ico.textContent = emoji(d);
      const mid = document.createElement('div'); mid.className = 'mid';
      const nm = document.createElement('div'); nm.className = 'nm'; nm.textContent = d.alias;
      const meta = document.createElement('div'); meta.className = 'meta';
      const lm = lastByFp[d.fingerprint];
      const preview = lm
        ? (lm.kind === 'text'
            ? lm.text.replace(/\s+/g, ' ').slice(0, 42)
            : (isImg(lm.name) ? '[图片]' : '📄 ' + lm.name))
        : '';
      meta.textContent = d.online
        ? (preview || `${d.deviceType || '?'} · ${d.ip}`)
        : (preview ? `离线 · ${preview}` : `离线 · 最后可见 ${fmtAgo(d.lastSeenMs)}`);
      mid.append(nm, meta);
      const x = document.createElement('button'); x.className = 'xbtn';
      x.textContent = '✕'; x.title = '从列表移除';
      x.onclick = e => { e.stopPropagation(); removeDevice(d); };
      row.append(ico, mid, x);
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

  // 上线 / 离线翻转提醒（按 online 布尔翻转，重上线会再提示；首轮静默）
  const nowFp = new Set(s.devices.map(d => d.fingerprint));
  if (firstRender){
    for (const d of s.devices) onlineMap.set(d.fingerprint, d.online);
    firstRender = false;
  } else {
    for (const d of s.devices){
      const prev = onlineMap.get(d.fingerprint);
      if (prev === undefined || (!prev && d.online)) toast(`上线:${d.alias}`);
      else if (prev && !d.online) toast(`离线:${d.alias}`);
      onlineMap.set(d.fingerprint, d.online);
    }
    // 从列表消失（被移除的设备有自己的 toast）→ 静默清记录
    for (const f of [...onlineMap.keys()]) if (!nowFp.has(f)) onlineMap.delete(f);
  }

  // 选中设备被移除 → 回到等待屏（离线设备保留在列表，不触发）
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
    $('ch-meta').textContent = d.online
      ? `${d.deviceType || '?'} · ${d.ip} · ${fmtAgo(d.lastSeenMs)}可见`
      : `离线 · 最后可见 ${fmtAgo(d.lastSeenMs)}`;
  } else {
    const allOff = s.devices.length && !s.devices.some(x => x.online);
    $('c-title').textContent = allOff ? '设备暂不在线' : '等待设备上线';
    $('c-sub').textContent = allOff
      ? '左侧保留的设备仍可发送，送达需等对方上线'
      : s.devices.length
        ? '点击左侧的设备开始发送'
        : '同一 Wi-Fi 下的 LocalSend 设备会自动出现在左侧';
    $('dropzone').textContent = '📁 拖入文件即可发送';
  }
}

// 时间分组标签（微信式居中显示；跨天带日期）
function fmtDay(ms){
  const d = new Date(ms), n = new Date();
  const hm = d.toLocaleTimeString('zh-CN', {hour12:false, hour:'2-digit', minute:'2-digit'});
  return d.toDateString() === n.toDateString()
    ? hm
    : d.toLocaleDateString('zh-CN', {month:'numeric', day:'numeric'}) + ' ' + hm;
}

// 会话气泡流（微信式：头像在最外侧、时间居中分组、文件为中性卡片；内容无变化不重绘）
function renderChat(s){
  const sc = $('cscroll'), box = $('chatlist');
  if (!s || !sel){ lastChatKey = ''; box.innerHTML = ''; return; }
  const msgs = (s.chat||[]).filter(m => m.peer === sel);
  // 防抖 key 含出站状态：⏳→✓/⚠ 翻转必须重绘
  const key = sel + ':' + msgs.map(m => m.id + ':' + (m.out ? m.status : '')).join(',');
  const switched = lastChatSel !== sel;
  if (key === lastChatKey && !switched) return;
  lastChatKey = key; lastChatSel = sel;
  const nearBottom = sc.scrollHeight - sc.scrollTop - sc.clientHeight < 90;
  box.innerHTML = '';
  if (!msgs.length){
    const d = (s.devices||[]).find(x => x.fingerprint === sel);
    const hint = document.createElement('div'); hint.className = 'chathint';
    hint.innerHTML = `与「${esc(d ? d.alias : '对方')}」的对话会显示在这里` +
      '<br><small>拖入文件，或用左下角按钮选择 文件 / 文件夹 / 剪贴板，也可直接输入文字</small>';
    box.appendChild(hint);
  }
  const dev = (s.devices||[]).find(x => x.fingerprint === sel);
  const peerAva = dev ? emoji(dev) : '🖥️';
  let prevAt = 0;
  for (const m of msgs){
    if (!prevAt || m.at - prevAt > 5*60*1000){ // 间隔 > 5 分钟插一条居中时间
      const tm = document.createElement('div'); tm.className = 'tm';
      tm.textContent = fmtDay(m.at);
      box.appendChild(tm);
    }
    prevAt = m.at;
    const crow = document.createElement('div');
    crow.className = 'crow ' + (m.out ? 'out' : 'in');
    const ava = document.createElement('div'); ava.className = 'ava';
    ava.textContent = m.out ? '🐜' : peerAva;
    const col = document.createElement('div'); col.className = 'msgcol';
    const bub = document.createElement('div'); bub.className = 'bub';
    if (m.kind === 'text'){
      bub.textContent = m.text;
    } else {
      // 中性文件卡片（图片加载失败时的退路）
      const fillChip = () => {
        bub.classList.add('file');
        const chip = document.createElement('div'); chip.className = 'fchip';
        const fi = document.createElement('span'); fi.className = 'fico'; fi.textContent = '📄';
        const mid = document.createElement('div'); mid.className = 'fmid';
        const fn = document.createElement('div'); fn.className = 'fnm';
        fn.textContent = m.name; fn.title = m.name;
        const fs = document.createElement('div'); fs.className = 'fsz';
        fs.textContent = fmtSize(m.size);
        mid.append(fn, fs); chip.append(fi, mid); bub.appendChild(chip);
      };
      if (isImg(m.name)){
        // 图片直接显示：收到的读下载目录，出站的走内存缓存（已淘汰则退回卡片）
        bub.classList.add('img');
        const img = document.createElement('img');
        img.className = 'chatimg'; img.alt = m.name; img.loading = 'lazy';
        img.src = m.file
          ? '/api/ui/asset?file=' + encodeURIComponent(m.file)
          : '/api/ui/asset?id=' + m.id;
        img.onerror = () => { img.remove();
          if (!bub.firstChild){ bub.classList.remove('img'); fillChip(); } };
        img.onclick = () => zoomImg(img.src, m.name);
        bub.appendChild(img);
      } else fillChip();
    }
    col.appendChild(bub);
    if (m.kind === 'text' && !m.out){ // 文字不落盘，「复制」是取走内容的唯一途径
      const cp = document.createElement('button'); cp.className = 'rxb';
      cp.textContent = '复制';
      cp.onclick = () => { const t = m.text, done = () => toast('已复制');
        if (navigator.clipboard)
          navigator.clipboard.writeText(t).then(done).catch(() => fallbackCopy(t, done));
        else fallbackCopy(t, done); };
      col.appendChild(cp);
    }
    if (m.kind === 'file' && !m.out && m.file){ // 收到的文件可在 Finder / 资源管理器中定位
      const b = document.createElement('button'); b.className = 'rxb';
      b.textContent = '显示'; b.onclick = () => reveal(m.file);
      col.appendChild(b);
    }
    if (m.out){ // 出站消息状态行：⏳ 发送中 / ✓ 已送达 / ⚠ 未送达（文本与带源路径的可重试）
      const st = document.createElement('div'); st.className = 'st';
      if (m.status === 'sending') st.textContent = '⏳ 发送中…';
      else if (m.status === 'ok') st.textContent = '✓ 已送达';
      else if (m.status === 'fail'){
        if (m.kind === 'text' || m.srcPath){
          st.textContent = '⚠ 未送达 ';
          const rb = document.createElement('button'); rb.className = 'rbtn';
          rb.textContent = '重试';
          rb.onclick = () => retryMsg(m);
          st.appendChild(rb);
        } else st.textContent = '⚠ 未送达 · 请重新拖入文件';
      }
      if (st.textContent) col.appendChild(st); // 入站与旧数据无状态行
    }
    crow.append(ava, col);
    box.appendChild(crow);
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

// ── 原生选择器（服务端 rfd）+ 路径发送（记录 src_path，失败可重试） ──
// 目录会由服务端递归展开，每个文件独立气泡
async function sendPaths(fp, paths){
  sel = fp; syncSel();
  for (const p of paths){
    try{
      const r = await fetch('/api/ui/send-path', {method:'POST',
        headers:{'Content-Type':'application/json'}, body: JSON.stringify({target: fp, path: p})});
      const v = await r.json().catch(()=>({}));
      if (!r.ok && !((v.ids||[]).length)){ toast(v.error || '发送失败', true); break; }
      if (!r.ok && v.error) toast(v.error, true); // 部分成功：失败项已有 ⚠ 气泡
    }catch(e){ toast('发送失败:' + e.message, true); break; }
  }
}

// 原生选择优先；服务端不支持（无桌面环境）或请求失败 → 回退浏览器 <input>（流式，无重试）
async function pickNative(kind, fp){
  let v = null, ok = false;
  try{
    const r = await fetch('/api/ui/pick', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({kind})});
    v = await r.json().catch(()=>null);
    ok = r.ok;
  }catch(_){}
  if (ok && v && (v.paths === null || Array.isArray(v.paths))){
    if (Array.isArray(v.paths) && v.paths.length) sendPaths(fp, v.paths);
    return; // paths:null / [] = 用户取消
  }
  (kind === 'folder' ? pickThenFolder : pickThenFile)(fp);
}

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

$('btn-file').onclick = () => resolveTarget(fp => pickNative('file', fp));

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

// ── 聊天输入条：Enter 发送（失败已在会话流留 ⚠ 气泡，不回填输入框） ──
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
        toast(v.error || '发送失败（消息已标记未送达）', true);
      }
    })
    .catch(e => toast('发送失败:' + e.message, true))
    .finally(() => { inp.disabled = false; inp.focus(); });
}
$('btn-send').onclick = sendChatText;
$('chatinput').addEventListener('keydown', e => {
  if (e.key === 'Enter'){ e.preventDefault(); sendChatText(); }
});

// ── 输入框工具行（微信式：左下角 文件/文件夹/剪贴板，右侧发送） ──
// 文件/文件夹：原生选择器优先（带源路径、失败可重试），不支持再回退浏览器选择
$('btn-cfile').onclick = () => {
  if (sel) pickNative('file', sel);
  else resolveTarget(fp => pickNative('file', fp));
};

// 文件夹：webkitdirectory 一次选中整棵目录，逐个文件发送（接收端按文件名平铺保存）
let pendingFolderTarget = null;
function pickThenFolder(fp){ pendingFolderTarget = fp; $('folderpick').value = ''; $('folderpick').click(); }
$('folderpick').onchange = () => {
  const files = [...$('folderpick').files]
    .filter(f => !f.webkitRelativePath.split('/').some(seg => seg.startsWith('.'))); // 滤掉 .DS_Store 等隐藏文件
  $('folderpick').value = '';
  if (files.length && pendingFolderTarget){
    toast(`开始发送 ${files.length} 个文件`);
    sendFiles(pendingFolderTarget, files);
  } else if (!files.length) toast('文件夹里没有可发送的文件', true);
  pendingFolderTarget = null;
};
$('btn-cfolder').onclick = () => {
  if (sel) pickNative('folder', sel);
  else resolveTarget(fp => pickNative('folder', fp));
};

// 剪贴板：图片（截图场景）直接作为文件发送，文本填入输入框待编辑；
// WKWebView 可能拒绝 read()，逐级降级到 readText，再不行提示手动粘贴。
async function sendClipboard(){
  const withTarget = send => { if (sel) send(sel); else resolveTarget(send); };
  try{
    if (navigator.clipboard && navigator.clipboard.read){
      const items = await navigator.clipboard.read();
      for (const it of items){
        const imgType = it.types.find(t => t.startsWith('image/'));
        if (imgType){
          const blob = await it.getType(imgType);
          const ext = (imgType.split('/')[1] || 'png').replace('+xml', '');
          const f = new File([blob], `剪贴板图片.${ext}`, {type: imgType});
          withTarget(fp => sendFiles(fp, [f]));
          return;
        }
      }
    }
  }catch(_){ /* 退回 readText */ }
  try{
    if (navigator.clipboard && navigator.clipboard.readText){
      const text = await navigator.clipboard.readText();
      if (text && text.trim()){
        const inp = $('chatinput');
        inp.value += text; inp.focus();
        return;
      }
      toast('剪贴板里没有可发送的内容', true);
      return;
    }
  }catch(_){ /* 无剪贴板读取权限 */ }
  toast('无法读取剪贴板，可直接 ⌘/Ctrl+V 粘贴', true);
}
$('btn-cclip').onclick = sendClipboard;

// 输入框直接粘贴图片（WebView2 / Chromium 支持粘贴截图）＝ 发给当前会话
$('chatinput').addEventListener('paste', e => {
  const files = [...((e.clipboardData && e.clipboardData.files) || [])];
  if (!files.length) return;
  e.preventDefault();
  if (sel) sendFiles(sel, files);
  else resolveTarget(fp => sendFiles(fp, files));
});

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

// 重试未送达的消息（服务端校验可重试性：文本或有源路径的文件；返回后状态经轮询刷新）
async function retryMsg(m){
  try{
    const r = await fetch('/api/ui/retry', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({id: m.id})});
    const v = await r.json().catch(()=>({}));
    if (r.ok) toast('重试中…');
    else toast(v.error || '重试失败', true);
  }catch(e){ toast('重试失败:' + e.message, true); }
}

// 移除设备：内存 + DB 删行，消息历史保留（对方重新 announce 会自动回来）
async function removeDevice(d){
  try{
    const r = await fetch('/api/ui/remove-device', {method:'POST',
      headers:{'Content-Type':'application/json'},
      body: JSON.stringify({fingerprint: d.fingerprint})});
    const v = await r.json().catch(()=>({}));
    if (r.ok){
      if (sel === d.fingerprint) sel = null;
      lastDevJson = ''; // 下一轮立即重绘侧栏
      toast(`已移除「${d.alias}」，重新上线会自动出现`);
    } else toast(v.error || '移除失败', true);
  }catch(e){ toast('移除失败:' + e.message, true); }
}

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
