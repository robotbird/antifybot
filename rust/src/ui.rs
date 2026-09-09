//! 面板页面（内嵌 HTML，无外部依赖，离线可用）
//! 左右分栏：ChatGPT 式浅暖灰侧栏（品牌 + 设备列表 + 左下角「设置」入口），
//! 近白主区；深色模式镜像为暗一档。
//! 设备上线 → 左侧会话列表式列表（第二行显示最近一条消息预览）；
//! 本机固定钉在列表首位（自我会话：仅文字，文件 / 文件夹按钮停用，顶栏与头部显示本机 ip:端口）；
//! 点选设备 → 会话视图（ChatGPT 式灰白）：头部横带取侧栏同色（与侧栏连成 L 形），
//! 消息区纯白（深色为 #212121，比侧栏亮半档拉开 L 形层次）——出站灰底大圆角气泡、入站无气泡纯文本、无头像，
//! 图片直接显示缩略图（点击全屏查看，加载失败退回文件卡片）、文件为中性卡片，
//! 时间按间隔居中分组；收到的文件带「显示」、文字带「复制」；
//! 活动传输为会话流内吸顶进度卡，最下面是 ChatGPT 式输入盒（白底大圆角细描边）：
//! 盒内上为输入行、下为工具行 —— 左下角 文件 / 文件夹 / 剪贴板 三个图标按钮，
//! 右侧深色圆形发送钮。
//! 设置面板（侧栏左下角进入）分三节：通用（主题 / 语言）、存储（默认保存地址）、
//! 关于（版本 / 检查更新 / 节点信息）；主题三态 + 中英文界面，localStorage 持久化。
//! 拖文件到窗口任意位置即可发送：设备行定向投放优先，其余位置按 当前会话 → 唯一设备 → 弹窗选择
//! 解析目标；拖动中全窗 accent 内描边高亮 + 顶部胶囊提示目标。
//! 事件以 toast 呈现；消息 / 文件到达与出站成败另发系统通知
//! （面板聚焦时轮询捎带静音；收托盘 / 切后台自动恢复提醒）。
pub const DASHBOARD: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AntifyBot</title>
<style>
  /* ── 主题变量：默认浅色；显式深色 data-theme="dark"；系统深色但未锁定浅色时同样取深色 ── */
  :root{
    color-scheme:light;
    --bg:#f7f7f8; --card:#ffffff; --ink:#202124; --muted:#74777d;
    --line:rgba(0,0,0,.08); --line-strong:rgba(0,0,0,.13);
    --accent:#3979e8; --accent-deep:#2466cf;
    /* 设定页的保存操作沿用界面既有主蓝色，不随深色主题的橙色强调色切换。 */
    --ui-blue:#3979e8; --ui-blue-deep:#2466cf;
    --accent-soft:rgba(57,121,232,.11); --ok:#34a853; --err:#d84b52;
    /* 侧栏：ChatGPT 式浅暖灰，比主区深半档 */
    --side:#f0f0f1; --side-ink:#24262a; --side-muted:#777a80;
    --side-line:rgba(0,0,0,.08); --side-hover:rgba(0,0,0,.045);
    --side-sel:rgba(0,0,0,.085);
    /* 会话区（ChatGPT 式）：纯白消息区、灰底出站气泡、白底输入盒 */
    --chat-bg:#ffffff; --bub-out:#f0f0f0; --comp:#ffffff;
    --soft:rgba(0,0,0,.055); --soft-hover:rgba(0,0,0,.10);
    --scroll-track:transparent; --scroll-thumb:rgba(80,82,88,.32); --scroll-thumb-hover:rgba(80,82,88,.48);
    --shadow:0 1px 2px rgba(0,0,0,.03), 0 12px 32px -20px rgba(0,0,0,.28);
    --sans:-apple-system,BlinkMacSystemFont,"SF Pro Text","Segoe UI",
           "PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif;
    --mono:ui-monospace,"SF Mono",Menlo,Consolas,monospace;
  }
  :root[data-theme="dark"]{
    color-scheme:dark;
    --bg:#1a1a1c; --card:#232327; --ink:#f0f0f2; --muted:#9a9aa0;
    --line:rgba(255,255,255,.11); --line-strong:rgba(255,255,255,.16);
    --accent:#e8932c; --accent-deep:#f0a64e;
    --accent-soft:rgba(232,147,44,.14); --ok:#4cc38a; --err:#ff6b6e;
    --side:#131315; --side-ink:#f0f0f2; --side-muted:#9a9aa0;
    --side-line:rgba(255,255,255,.10); --side-hover:rgba(255,255,255,.05);
    --side-sel:rgba(255,255,255,.09);
    --chat-bg:#212121; --bub-out:#303030; --comp:#303030;
    --soft:rgba(255,255,255,.07); --soft-hover:rgba(255,255,255,.14);
    --scroll-track:#212121; --scroll-thumb:rgba(255,255,255,.22); --scroll-thumb-hover:rgba(255,255,255,.34);
    --shadow:0 1px 2px rgba(0,0,0,.4), 0 16px 36px -20px rgba(0,0,0,.7);
  }
  @media(prefers-color-scheme:dark){
    :root:not([data-theme="light"]){
      color-scheme:dark;
      --bg:#1a1a1c; --card:#232327; --ink:#f0f0f2; --muted:#9a9aa0;
      --line:rgba(255,255,255,.11); --line-strong:rgba(255,255,255,.16);
    --accent:#e8932c; --accent-deep:#f0a64e;
      --accent-soft:rgba(232,147,44,.14); --ok:#4cc38a; --err:#ff6b6e;
      --side:#131315; --side-ink:#f0f0f2; --side-muted:#9a9aa0;
      --side-line:rgba(255,255,255,.10); --side-hover:rgba(255,255,255,.05);
      --side-sel:rgba(255,255,255,.09);
    --chat-bg:#212121; --bub-out:#303030; --comp:#303030;
      --soft:rgba(255,255,255,.07); --soft-hover:rgba(255,255,255,.14);
      --scroll-track:#212121; --scroll-thumb:rgba(255,255,255,.22); --scroll-thumb-hover:rgba(255,255,255,.34);
      --shadow:0 1px 2px rgba(0,0,0,.4), 0 16px 36px -20px rgba(0,0,0,.7);
    }
  }
  *{box-sizing:border-box}
  *{scrollbar-width:thin;scrollbar-color:var(--scroll-thumb) var(--scroll-track)}
  ::-webkit-scrollbar{width:10px;height:10px}
  ::-webkit-scrollbar-track{background:var(--scroll-track)}
  ::-webkit-scrollbar-thumb{background:var(--scroll-thumb);border:3px solid var(--scroll-track);border-radius:999px}
  ::-webkit-scrollbar-thumb:hover{background:var(--scroll-thumb-hover)}
  html,body{height:100%}
  body{margin:0;background:var(--bg);color:var(--ink);font-family:var(--sans);
    line-height:1.5;overflow:hidden;-webkit-font-smoothing:antialiased;
    transition:background .3s,color .3s}
  /* App shell: a restrained desktop chrome above a persistent two-column workspace. */
  .app{display:grid;grid-template-rows:44px minmax(0,1fr);height:100%;min-height:0}
  .appchrome{display:grid;grid-template-columns:248px minmax(0,1fr);user-select:none;
    transition:grid-template-columns .26s cubic-bezier(.2,.8,.2,1)}
  .chrome-side,.chrome-main{display:flex;align-items:center;min-width:0}
  .chrome-side{position:relative;padding:0 14px;gap:10px;background:var(--side)}
  .sidebar-toggle{width:28px;height:28px;border:0;border-radius:8px;background:transparent;color:var(--side-muted);
    display:grid;place-items:center;cursor:pointer;transition:background .15s,color .15s}
  .sidebar-toggle:hover{background:var(--side-hover);color:var(--side-ink)}
  .sidebar-toggle:active{transform:scale(.96)}
  .sidebar-toggle i{position:relative;display:block;width:16px;height:15px;border:1.5px solid currentColor;border-radius:4px}
  .sidebar-toggle i::after{content:"";position:absolute;top:1px;bottom:1px;left:4px;border-left:1.5px solid currentColor}
  .chrome-main{justify-content:space-between;padding:0 20px 0 28px;gap:16px;background:var(--chat-bg)}
  .crumbs{display:flex;align-items:center;gap:8px;min-width:0;color:var(--muted);font-size:12.5px}
  .crumbs b{color:var(--ink);font-weight:680;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .chrome-context{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .workspace{display:flex;min-height:0}
  /* Tauri on macOS overlays the native traffic lights onto this toolbar. */
  .is-macos .chrome-side{padding-left:76px}
  /* 与右侧标题共用工具栏中线；不要再额外上移切换按钮。 */
  .is-macos .sidebar-toggle{margin-top:0}
  /* 原生窗口失焦时，用与 Codex 一致的灰色交通灯覆盖系统的淡色状态。 */
  /* 原生交通灯位于 overlay 标题栏的上方安全区，并不在 44px 工具栏的垂直中心。 */
  .traffic-fallback{display:none;position:absolute;z-index:20;top:18px;left:8px;gap:8px;
    pointer-events:none}
  .traffic-fallback i{width:12px;height:12px;border-radius:50%;background:#d8d8da;border:1px solid rgba(0,0,0,.05)}
  .is-macos body.window-inactive .traffic-fallback{display:flex}
  html.sidebar-collapsed .appchrome{grid-template-columns:64px minmax(0,1fr)}
  /* 收起后主区全宽，侧栏内容轻微淡出并滑离，不留可点击区域。 */
  html.sidebar-collapsed .side{width:0;opacity:0;pointer-events:none}
  html.sidebar-collapsed .side > *{opacity:0;transform:translateX(-10px)}
  html.sidebar-collapsed .chrome-side{background:var(--chat-bg);border-right:none}
  /* macOS 的交通灯占据左上安全区；收起态仍需容纳其后的切换按钮。 */
  html.sidebar-collapsed.is-macos .appchrome{grid-template-columns:120px minmax(0,1fr)}
  @media(prefers-reduced-motion:reduce){.appchrome,.side,.side > *{transition:none}}

  /* ── 左侧栏（ChatGPT 式） ── */
  .side{width:248px;flex:none;overflow:hidden;background:var(--side);color:var(--side-ink);
    display:flex;flex-direction:column;transition:width .26s cubic-bezier(.2,.8,.2,1),opacity .16s ease}
  .side > *{transition:opacity .14s ease,transform .22s cubic-bezier(.2,.8,.2,1)}
  .side-scroll{flex:1;overflow-y:auto;padding:14px 10px 10px}
  /* 设备行：无描边、圆角 10px、选中浅灰底 */
  .dev{display:flex;align-items:center;gap:10px;padding:9px 10px;border-radius:10px;
    cursor:pointer;transition:background .15s;outline:1.5px dashed transparent;
    outline-offset:-1.5px}
  .dev:hover{background:var(--side-hover)}
  .dev:focus-visible{outline-color:var(--side-muted)}
  .dev.on{background:var(--side-sel)}
  /* 头像式圆底：字标（SVG/emoji）在等大圆形浅底内呈现，视觉大小一致 */
  .dev .ico{width:27px;height:27px;font-size:16px;line-height:1;flex:none;
    display:grid;place-items:center;border-radius:50%;background:var(--soft);
    color:var(--side-ink)}
  .dev .ico svg{width:17px;height:17px;display:block}
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
  .devempty{margin:12px 8px;padding:20px 10px;text-align:center;font-size:12.5px;
    color:var(--side-muted);border:1.5px dashed var(--side-line);border-radius:12px}
  .devempty small{font-size:11px;opacity:.8}
  /* 左下角设置入口：普通图标 + 文字行（ChatGPT 式用户位） */
  .sidefoot{flex:none;padding:8px 10px 10px}
  .setrow{display:flex;align-items:center;gap:10px;width:100%;padding:9px 10px;
    border:none;border-radius:10px;background:transparent;color:var(--side-muted);
    font-family:var(--sans);font-size:13.5px;font-weight:600;cursor:pointer;
    transition:background .15s}
  .setrow:hover{background:var(--side-hover)}
  .setrow .sic{width:24px;height:24px;display:grid;place-items:center;font-size:24px;line-height:1;flex:none;transform:translateY(-1px)}
  .setrow .smid{flex:1;text-align:left;min-width:0;overflow:hidden;
    text-overflow:ellipsis;white-space:nowrap}
  .setrow .schev{color:inherit;font-size:13px;line-height:1}

  /* ── 主区 ── */
  .main{flex:1;min-width:0;display:flex;flex-direction:column;min-height:0;background:var(--chat-bg)}

  /* 会话视图（选中设备后）：头部 + 气泡流 + 底部聊天输入条 */
  #chatview{flex:1;min-height:0;display:flex;flex-direction:column;background:var(--chat-bg)}
  #chatview[hidden]{display:none}
  /* 头部横带取侧栏同色，与左侧连成 L 形，和消息区拉开层次 */
  /* Selected-device context lives in the app chrome, leaving the conversation uninterrupted. */
  .chead{display:none}
  .chead h2{margin:0;font-size:16px;font-weight:700;letter-spacing:-.01em;
    max-width:50%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .chead .cmeta{font-size:12px;color:var(--muted);min-width:0;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .cscroll{flex:1;min-height:0;overflow-y:auto;background:var(--chat-bg);
    scrollbar-color:var(--scroll-thumb) var(--chat-bg)}
  .cscroll::-webkit-scrollbar{width:12px;background-color:var(--chat-bg)}
  .cscroll::-webkit-scrollbar-track,.cscroll::-webkit-scrollbar-track-piece,
  .cscroll::-webkit-scrollbar-corner{background-color:var(--chat-bg)!important}
  .cscroll::-webkit-scrollbar-thumb{background-color:var(--scroll-thumb)!important;
    border:3px solid var(--chat-bg)!important;border-radius:999px}
  .cscroll::-webkit-scrollbar-thumb:hover{background-color:var(--scroll-thumb-hover)!important}
  body.dropping .cscroll{box-shadow:inset 0 0 0 1.5px var(--accent)}
  #chatlist{display:flex;flex-direction:column;gap:16px;padding:22px 26px;
    width:min(720px,100%);margin:0 auto}
  .chathint{margin:60px auto;text-align:center;color:var(--muted);font-size:13px;line-height:1.9}
  .chathint small{font-size:11.5px;opacity:.85}
  /* 消息（ChatGPT 式）：无头像；出站灰底大圆角气泡、入站无气泡纯文本；文件用中性卡片 */
  .tm{align-self:center;font-size:11px;color:var(--muted);margin:2px 0}
  .crow{display:flex;align-items:flex-start}
  .crow.out{flex-direction:row-reverse}
  .msgcol{min-width:0;max-width:min(76%,540px);display:flex;flex-direction:column;
    align-items:flex-start}
  .crow.out .msgcol{align-items:flex-end}
  .bub{padding:2px 0;border-radius:18px;font-size:15px;line-height:1.65;
    text-align:left;white-space:pre-wrap;word-break:break-word}
  .crow.out .bub{background:var(--bub-out);color:var(--ink);padding:10px 16px}
  /* 文件消息：无论方向都是中性卡片（选择器带 .crow 抬高优先级，压过出站灰气泡） */
  .crow .bub.file{background:var(--card);color:var(--ink);border:1px solid var(--line);
    box-shadow:var(--shadow);border-radius:14px;padding:9px 14px}
  .fchip{display:flex;align-items:center;gap:9px;min-width:0}
  .fchip .fico{font-size:20px;line-height:1;flex:none}
  .fchip .fmid{min-width:0}
  .fchip .fnm{font-weight:600;font-size:13.5px;max-width:240px;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .fchip .fsz{font-size:11px;opacity:.75;font-family:var(--mono)}
  #filemenu{position:fixed;z-index:100;min-width:142px;padding:5px;border:1px solid var(--line);
    border-radius:10px;background:var(--card);box-shadow:var(--shadow)}
  #filemenu[hidden]{display:none}
  .filemenu-item{display:block;width:100%;border:0;border-radius:7px;padding:7px 9px;background:transparent;
    color:var(--ink);font:500 12.5px var(--sans);text-align:left;cursor:pointer}
  .filemenu-item:hover{background:var(--soft)}
  /* 图片消息：直接渲染缩略图（点击全屏查看）；加载失败由 JS 退回文件卡片 */
  .crow .bub.img{position:relative;padding:0;background:transparent}
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
  /* 出站消息状态行（发送中 / 已送达 / 未送达+重试） */
  .st{margin-top:3px;font-size:11px;color:var(--muted)}
  .st .rbtn{border:none;background:transparent;padding:0;font:inherit;font-weight:600;
    color:var(--err);cursor:pointer}
  .st .rbtn:hover{text-decoration:underline}
  .cfoot{flex:none;padding:10px 26px 18px;background:linear-gradient(0deg,var(--chat-bg) 74%,transparent)}
  /* 输入盒（ChatGPT 式）：白底大圆角细描边软阴影；盒内上输入行、下工具行 */
  .composer{display:flex;flex-direction:column;background:var(--comp);
    border:1px solid var(--line-strong);border-radius:26px;box-shadow:var(--shadow)}
  #chatinput{border:none;background:transparent;outline:none;padding:13px 18px 3px;
    font-size:15px;font-family:var(--sans);color:var(--ink)}
  #chatinput::placeholder{color:var(--muted)}
  #chatinput:disabled{opacity:.6}
  .ctools{display:flex;align-items:center;gap:4px;padding:4px 10px 10px}
  .tool{width:30px;height:30px;border-radius:50%;border:none;background:transparent;
    color:var(--muted);display:grid;place-items:center;cursor:pointer;transition:.15s}
  .tool:hover{background:var(--soft);color:var(--ink)}
  .tool svg{width:17px;height:17px;display:block}
  /* 本机会话：文件 / 文件夹不可用（文字与剪贴板文本仍可用） */
  .tool:disabled{opacity:.32;cursor:default;pointer-events:none}
  .ctools .tsp{flex:1}
  .tool.send{background:var(--ink);color:var(--bg)}
  .tool.send:hover{background:var(--ink);opacity:.85}

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
  /* ── 全窗口拖放反馈（拖动文件中）：accent 内描边 + 浅色蒙层 + 顶部目标提示胶囊 ──
     pointer-events:none，不拦截拖放事件本身（事件全靠 document 级监听收口） */
  #dropveil{position:fixed;inset:0;z-index:97;pointer-events:none;opacity:0;
    transition:opacity .12s;box-shadow:inset 0 0 0 2px var(--accent);
    background:var(--accent-soft)}
  body.dropping #dropveil{opacity:1}
  #dropveil .hint{position:absolute;top:56px;left:50%;transform:translateX(-50%);
    max-width:82vw;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;
    background:var(--accent);color:#fff;border-radius:999px;padding:8px 16px;
    font-size:13px;font-weight:600;box-shadow:var(--shadow)}
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
  /* 进度卡右上 ✕：取消传输（发送侧置取消标志；接收侧清会话 + 短期拒收） */
  .prog .xbtn{flex:none;align-self:center;width:22px;height:22px;border:none;border-radius:50%;
    background:transparent;color:var(--muted);font-size:12px;line-height:1;display:grid;
    place-items:center;cursor:pointer;transition:.15s}
  .prog .xbtn:hover{background:var(--err);color:#fff}
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

  /* ── 按钮（ChatGPT 式：软平面、无边框；主操作保留品牌橙） ── */
  .btn{font-family:var(--sans);font-size:14px;font-weight:600;border-radius:10px;
    padding:9px 18px;cursor:pointer;border:1px solid transparent;transition:.15s}
  .btn.primary{background:var(--accent);color:#fff}
  .btn.primary:hover{background:var(--accent-deep)}
  #set-dirsave{background:var(--ui-blue);color:#fff}
  #set-dirsave:hover:not(:disabled){background:var(--ui-blue-deep)}
  .btn.ghost{background:var(--soft);color:var(--ink)}
  .btn.ghost:hover{background:var(--soft-hover)}
  .btn:disabled{opacity:.5;cursor:default}
  .btn.small{font-size:12.5px;padding:5px 12px;border-radius:8px;font-weight:500}

  /* ── 对话框 ── */
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
  /* 胶囊选择：默认描边、选中黑底白字（单色开关） */
  /* 弹窗目标胶囊里的小圆底；.on 反色态下圆底换成互异色，图标仍清晰 */
  .icochip{display:inline-grid;place-items:center;width:20px;height:20px;
    border-radius:50%;background:var(--soft);color:var(--ink);
    vertical-align:-5px;margin-right:5px}
  .icochip svg{width:13px;height:13px;display:block}
  .pick.on .icochip{background:var(--bg)}
  .pick{border:1px solid var(--line);background:transparent;border-radius:999px;
    padding:6px 14px;font-size:13px;cursor:pointer;color:var(--ink);font-family:var(--sans);
    transition:.15s}
  .pick:hover{border-color:var(--muted)}
  .pick.on{border-color:transparent;background:var(--ink);color:var(--bg);font-weight:600}

  /* ── 设置面板 ── */
  #setdlg{width:min(880px,94vw);height:min(620px,86vh);max-height:86vh;padding:0;overflow:hidden}
  .setshell{display:grid;grid-template-columns:210px minmax(0,1fr);height:100%}
  .setnav{padding:30px 14px;background:var(--side);border-right:1px solid var(--side-line)}
  .setnav h3{margin:0 10px 22px;font-size:19px;letter-spacing:-.02em}
  .settab{display:block;width:100%;border:0;border-radius:10px;padding:10px 12px;background:transparent;
    color:var(--side-muted);font:600 14px var(--sans);text-align:left;cursor:pointer;transition:.15s}
  .settab:hover{background:var(--side-hover);color:var(--side-ink)}
  .settab.on{background:var(--side-sel);color:var(--side-ink)}
  .setcontent{position:relative;min-width:0;overflow-y:auto;padding:30px 36px 32px}
  .setpanel h3{margin:0 0 24px;font-size:20px;letter-spacing:-.02em}
  .setpanel[hidden]{display:none}
  .setclose{position:absolute;top:20px;right:22px;width:30px;height:30px;border:0;border-radius:8px;
    background:transparent;color:var(--muted);font:300 28px/1 var(--sans);cursor:pointer;transition:.15s}
  .setclose:hover{background:var(--soft);color:var(--ink)}
  .setpanel .sect{padding:0;border:0}
  .sect{padding:12px 0 6px;border-top:1px solid var(--line)}
  .sect:first-of-type{border-top:none;padding-top:0}
  .secth{font-size:12px;font-weight:600;color:var(--muted);letter-spacing:.06em;margin-bottom:6px}
  .srow{display:flex;align-items:center;gap:14px;padding:7px 0}
  .srow .rl{flex:1;min-width:0}
  .rk{font-size:13.5px;font-weight:600}
  .rd{font-size:11.5px;color:var(--muted);margin-top:1px}
  .srow .rd{margin:1px 0 0}
  .pickgrp{display:flex;gap:6px;flex:none}
  .pickgrp .pick{padding:5px 12px;font-size:12.5px}
  .dirrow{display:flex;gap:8px;margin:4px 0 6px}
  .dirrow input{flex:1;min-width:0;font-size:12px;font-family:var(--mono)}
  .dirhint{font-size:11.5px;color:var(--muted);margin:0 0 6px}
  .diracts{display:flex;justify-content:flex-end}
  .updstate{font-size:11.5px;color:var(--muted);margin-top:1px;min-height:1em}
  .updresult{display:flex;align-items:center;gap:10px;padding:0 0 6px;font-size:12.5px}
  details.nodeinfo{margin-top:4px;border-top:1px dashed var(--line);padding-top:10px}
  details.nodeinfo summary{font-size:12.5px;color:var(--muted);cursor:pointer;
    list-style:revert;margin-bottom:4px}
  details.nodeinfo summary:hover{color:var(--ink)}
  details.nodeinfo .kv .v{font-size:12px;word-break:break-all}
  @media(max-width:620px){
    #setdlg{width:94vw;height:min(680px,90vh)}.setshell{grid-template-columns:132px minmax(0,1fr)}
    .setnav{padding:24px 8px}.setnav h3{margin-inline:8px}.settab{padding-inline:9px;font-size:13px}
    .setcontent{padding:26px 20px}.pickgrp{flex-wrap:wrap;justify-content:flex-end}
  }

  /* ── toast ── */
  #toasts{position:fixed;bottom:26px;left:50%;transform:translateX(-50%);z-index:99;
    display:flex;flex-direction:column;gap:8px;align-items:center;pointer-events:none}
  .toast{background:var(--ink);color:var(--bg);border-radius:12px;padding:9px 18px;
    font-size:13.5px;box-shadow:var(--shadow);opacity:0;translate:0 6px;
    transition:.25s;max-width:80vw}
  .toast.in{opacity:1;translate:0 0}
  .toast.err{background:var(--err);color:#fff}

  @media(max-width:760px){
    .app{grid-template-rows:40px minmax(0,1fr)}
    .appchrome{grid-template-columns:198px minmax(0,1fr)}
    .chrome-side{padding:0 11px}.chrome-main{padding:0 14px}
    .side{width:198px}.center{padding:20px 16px}.chead{padding-inline:18px}.cfoot{padding-inline:16px}
  }
  @media(max-width:560px){
    .appchrome{grid-template-columns:minmax(0,1fr)}.chrome-main{display:none}
    .side{width:64px}.dev .mid,.dev .xbtn,.sidefoot .smid,.sidefoot .schev,.navnote{display:none}
    .chrome-side{justify-content:center;padding:0}.side-scroll{padding-inline:8px}
    .dev{justify-content:center;padding:10px}.sidefoot{padding-inline:8px}.setrow{justify-content:center;padding:10px}
  }
</style>
<script>
// 提前套主题，避免显式深色用户在浅色系统下首帧闪白（其余 UI 逻辑在页尾主脚本）
try{
  var _th = localStorage.getItem('antify-theme');
  if (_th === 'light' || _th === 'dark') document.documentElement.dataset.theme = _th;
  if (navigator.userAgent.includes('Macintosh')) document.documentElement.classList.add('is-macos');
  if (localStorage.getItem('antify-sidebar') === 'collapsed') document.documentElement.classList.add('sidebar-collapsed');
}catch(_){}
</script>
</head>
<body>
<div class="app">
  <header class="appchrome" aria-label="应用工具栏">
    <div class="chrome-side">
      <div class="traffic-fallback" aria-hidden="true"><i></i><i></i><i></i></div>
      <button class="sidebar-toggle" id="btn-sidebar" title="收起侧栏" aria-label="收起侧栏"><i aria-hidden="true"></i></button>
    </div>
    <div class="chrome-main">
      <div class="crumbs"><b id="chrome-title">设备与会话</b><span class="chrome-context" id="chrome-context"></span></div>
    </div>
  </header>
  <div class="workspace">

  <aside class="side">
    <div class="side-scroll">
      <div id="devices"></div>
      <div class="devempty" id="devempty"><span data-t="devEmpty">等待设备上线</span><br><small data-t="devEmptySub">同一网络下自动发现</small></div>
    </div>
    <div class="sidefoot">
      <button class="setrow" id="btn-settings">
        <span class="sic">⚙</span><span class="smid" data-t="settings">设置</span><span class="schev">›</span>
      </button>
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
          <input id="chatinput" type="text" data-tp="inputPh" placeholder="输入消息，Enter 发送…" autocomplete="off">
          <div class="ctools">
            <button class="tool" id="btn-cfile" data-tt="sendFile" title="选择文件发送">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></svg>
            </button>
            <button class="tool" id="btn-cfolder" data-tt="sendFolder" title="选择文件夹发送（全部文件）">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>
            </button>
            <button class="tool" id="btn-cclip" data-tt="sendClip" title="发送剪贴板内容（截图 / 文本）">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/></svg>
            </button>
            <span class="tsp"></span>
            <button class="tool send" id="btn-send" data-tt="send" title="发送">
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
          <button class="linkbtn strong" id="btn-file" data-t="chooseFiles">选择文件</button>
          <span class="sep">·</span>
          <button class="linkbtn" id="btn-text" data-t="sendText">发送文本</button>
        </div>
      </section>

      <section id="rxsec" hidden>
        <div class="rxhead"><span data-t="recent">最近接收</span><span class="n" id="rxcount"></span></div>
        <ul class="received" id="received"></ul>
      </section>
    </section>

  </main>

</div>
</div>

<input type="file" id="filepick" multiple style="display:none">
<input type="file" id="folderpick" webkitdirectory multiple style="display:none">

<dialog id="pickdlg">
  <h3 data-t="sendTo">发送给谁？</h3>
  <div class="chips-select" id="picklist"></div>
  <div class="modalacts"><button class="btn ghost" id="pickcancel" data-t="cancel">取消</button></div>
</dialog>

<dialog id="textdlg">
  <h3 data-t="sendTextT">发送文本</h3>
  <div class="chips-select" id="texttargets"></div>
  <textarea id="textbody" data-tp="textPh" placeholder="输入要发送的文本…"></textarea>
  <div class="modalacts">
    <button class="btn ghost" id="textcancel" data-t="cancel">取消</button>
    <button class="btn primary" id="textsend" data-t="sendBtn">发送</button>
  </div>
</dialog>

<!-- 设置：通用（主题 / 语言）· 存储（默认保存地址）· 关于（版本 / 检查更新 / 节点信息） -->
<dialog id="setdlg">
  <div class="setshell">
    <nav class="setnav" aria-label="设置分类">
      <h3 data-t="setTitle">设置</h3>
      <button class="settab on" type="button" data-set-tab="general" data-t="secGeneral">通用</button>
      <button class="settab" type="button" data-set-tab="storage" data-t="secStorage">存储</button>
      <button class="settab" type="button" data-set-tab="about" data-t="secAbout">关于</button>
    </nav>
    <div class="setcontent">
      <button class="setclose" id="setclose" type="button" aria-label="关闭">×</button>
      <section class="setpanel" data-set-panel="general">
        <h3 data-t="secGeneral">通用</h3>
        <div class="sect">
    <div class="srow">
      <div class="rl"><div class="rk" data-t="theme">主题</div>
        <div class="rd" data-t="themeDesc">跟随系统或固定浅色 / 深色</div></div>
      <div class="pickgrp" id="themepick">
        <button class="pick" data-v="system" data-t="themeSys">跟随系统</button>
        <button class="pick" data-v="light" data-t="themeLight">浅色</button>
        <button class="pick" data-v="dark" data-t="themeDark">深色</button>
      </div>
    </div>
    <div class="srow">
      <div class="rl"><div class="rk" data-t="langL">语言</div>
        <div class="rd" data-t="langDesc">界面显示语言</div></div>
      <div class="pickgrp" id="langpick">
        <button class="pick" data-v="zh" data-t="langZh">中文</button>
        <button class="pick" data-v="en" data-t="langEn">English</button>
      </div>
    </div>
        </div>
      </section>

      <section class="setpanel" data-set-panel="storage" hidden>
        <h3 data-t="secStorage">存储</h3>
        <div class="sect">
    <div class="rk" data-t="saveDir">默认保存地址</div>
    <div class="rd" data-t="saveDirDesc" style="margin:1px 0 6px">接收文件落盘位置</div>
    <div class="dirrow">
      <input type="text" id="set-dir" spellcheck="false">
      <button class="btn ghost small" id="set-dirbrowse" data-t="browse">浏览…</button>
    </div>
    <div class="dirhint" data-t="saveDirHint">修改后点「保存」生效</div>
    <div class="diracts"><button class="btn primary small" id="set-dirsave" data-t="save">保存</button></div>
        </div>
      </section>

      <section class="setpanel" data-set-panel="about" hidden>
        <h3 data-t="secAbout">关于</h3>
        <div class="sect">
    <div class="kv"><span class="k" data-t="versionL">版本</span>
      <span class="v" id="st-appver" style="font-family:var(--sans);font-size:13px"></span></div>
    <div class="srow">
      <div class="rl"><div class="rk" data-t="checkUpd">检查更新</div>
        <div class="updstate" id="upd-state"></div></div>
      <button class="btn ghost small" id="btn-checkupd" data-t="checkUpd">检查更新</button>
    </div>
    <div class="updresult" id="updresult" hidden>
      <button class="linkbtn strong" id="upd-open" data-t="updOpen">打开下载页</button>
    </div>
    <details class="nodeinfo">
      <summary data-t="nodeInfo">节点信息</summary>
      <div class="kv"><span class="k" data-t="aliasL">别名</span><span class="v" id="st-alias" style="font-family:var(--sans);font-size:13px"></span></div>
      <div class="kv"><span class="k" data-t="portL">端口</span><span class="v" id="st-port"></span></div>
      <div class="kv"><span class="k" data-t="fpL">指纹</span><span class="v" id="st-fp"></span></div>
      <div class="kv"><span class="k" data-t="protocolL">协议</span><span class="v" id="st-ver"></span></div>
      <div class="diracts" style="margin-top:8px">
        <button class="btn ghost small" id="st-copy" data-t="copyFp">复制指纹</button>
      </div>
    </details>
        </div>
      </section>
    </div>
  </div>
</dialog>

<div id="dropveil" aria-hidden="true"><span class="hint" id="dropveil-hint"></span></div>
<div id="lightbox" hidden><img alt=""></div>
<div id="filemenu" role="menu" hidden>
  <button class="filemenu-item" id="filemenu-copy" type="button" data-t="copy">复制</button>
  <button class="filemenu-item" id="filemenu-open" type="button" data-t="openFolder">打开文件夹</button>
</div>
<div id="toasts"></div>

<script>
'use strict';
const $ = id => document.getElementById(id);
const esc = s => String(s??'').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

// ── i18n：字典 + 插值 {0}{1}；localStorage 记忆，切换即全量重渲染 ──
const L = {
  zh: {
    app:'蚂蚁快传', devices:'设备', settings:'设置',
    devEmpty:'等待设备上线', devEmptySub:'同一网络下自动发现',
    removeDev:'从列表移除',
    inputPh:'输入消息，Enter 发送…',
    sendFile:'选择文件发送', sendFolder:'选择文件夹发送（全部文件）',
    sendClip:'发送剪贴板内容（截图 / 文本）', send:'发送',
    previewImg:'[图片]', offline:'离线', offlineLastSeen:'离线 · 最后可见 {0}', seenAgo:'{0}可见',
    devEmptyMeta:'{0} · {1}',
    selfTag:'本机', selfName:'{0}（本机）',
    selfNoFiles:'本机会话不支持发送文件 / 文件夹',
    chatHint1:'与「{0}」的对话会显示在这里',
    chatHint2:'拖入文件，或用左下角按钮选择 文件 / 文件夹 / 剪贴板，也可直接输入文字',
    chatHint2Self:'发给本机的文字备忘会显示在这里（不支持发送文件 / 文件夹）',
    copy:'复制', copied:'已复制', copyFail:'复制失败', show:'显示',
    filePath:'文件位置', openFolder:'打开文件夹',
    stSending:'⏳ 发送中…', stOk:'✓ 已送达', stFail:'⚠ 未送达 ',
    stRetry:'重试', stFailRedrag:'⚠ 未送达 · 请重新拖入文件',
    progSend:'发送到「{0}」· {1}', progRecv:'来自「{0}」· {1}',
    waitTitle:'等待设备上线', waitSubNone:'同一 Wi-Fi 下的 LocalSend 设备会自动出现在左侧',
    waitSubSome:'点击左侧的设备开始发送',
    waitOffTitle:'设备暂不在线', waitOffSub:'左侧保留的设备仍可发送，送达需等对方上线',
    dropHint:'📁 拖入文件即可发送', chooseFiles:'选择文件', sendText:'发送文本', recent:'最近接收',
    dropSendTo:'松开即发送给「{0}」', dropPick:'松开后选择发送对象', dropNoDev:'暂无设备可发送',
    sendTo:'发送给谁？', cancel:'取消', sendBtn:'发送',
    sendTextT:'发送文本', textPh:'输入要发送的文本…',
    portL:'端口',
    noDevices:'还没有可用设备', sendingN:'开始发送 {0} 个文件', folderEmpty:'文件夹里没有可发送的文件',
    clipEmpty:'剪贴板里没有可发送的内容',
    clipDenied:'无法读取剪贴板，可直接 ⌘/Ctrl+V 粘贴', clipImgName:'剪贴板图片',
    sentT:'已送达：{0}', sendFail:'发送失败',
    textSent:'文本已送达', retrying:'重试中…', retryFail:'重试失败',
    bigFileHint:'{0} 较大：拖拽中转不支持断点续传，建议用「发送文件」按钮（可断点续传）',
    cancelT:'取消传输', cancelSendT:'已请求取消发送（重试可断点续传）',
    cancelRxT:'已取消接收（5 分钟内拒收对方重试）', cancelFail:'取消失败',
    onlineT:'上线：{0}', offlineT:'离线：{0}', receivedT:'已接收：{0}',
    removed:'已移除「{0}」，重新上线会自动出现', removeFail:'移除失败', openFail:'打开失败',
    setTitle:'设置', secGeneral:'通用', secStorage:'存储', secAbout:'关于',
    theme:'主题', themeDesc:'跟随系统或固定浅色 / 深色',
    themeSys:'跟随系统', themeLight:'浅色', themeDark:'深色',
    langL:'语言', langDesc:'界面显示语言', langZh:'中文', langEn:'English',
    saveDir:'默认保存地址', saveDirDesc:'接收文件落盘位置', saveDirHint:'修改后点「保存」生效',
    browse:'浏览…', save:'保存', dirEmpty:'目录不能为空', dirSaved:'保存目录已更新', setDirFail:'目录保存失败',
    pickUnsupported:'此环境不支持原生选择器，可直接粘贴路径',
    versionL:'版本', checkUpd:'检查更新', updChecking:'正在检查…',
    updLatest:'已是最新版本（{0}）', updFound:'发现新版本 {0}', updFail:'检查更新失败',
    updOpen:'打开下载页',
    nodeInfo:'节点信息', aliasL:'别名', fpL:'指纹', protocolL:'协议',
    copyFp:'复制指纹', fpCopied:'指纹已复制', close:'关闭',
  },
  en: {
    app:'AntifyBot', devices:'Devices', settings:'Settings',
    devEmpty:'Waiting for devices', devEmptySub:'Auto-discovered on this network',
    removeDev:'Remove from list',
    inputPh:'Type a message and press Enter…',
    sendFile:'Choose files to send', sendFolder:'Choose a folder (all files)',
    sendClip:'Send clipboard (screenshot / text)', send:'Send',
    previewImg:'[Image]', offline:'Offline', offlineLastSeen:'Offline · last seen {0}', seenAgo:'seen {0}',
    devEmptyMeta:'{0} · {1}',
    selfTag:'This device', selfName:'{0} (This device)',
    selfNoFiles:"Can't send files or folders to this device",
    chatHint1:'Your conversation with {0} will appear here',
    chatHint2:'Drop files here, use the buttons at the bottom-left, or just type',
    chatHint2Self:'Text notes to this device appear here (files and folders not supported)',
    copy:'Copy', copied:'Copied', copyFail:'Copy failed', show:'Show',
    filePath:'File location', openFolder:'Open folder',
    stSending:'⏳ Sending…', stOk:'✓ Delivered', stFail:'⚠ Not delivered ',
    stRetry:'Retry', stFailRedrag:'⚠ Not delivered · drag the file again',
    progSend:'Sending to {0} · {1}', progRecv:'From {0} · {1}',
    waitTitle:'Waiting for devices', waitSubNone:'LocalSend devices on the same Wi-Fi appear here automatically',
    waitSubSome:'Pick a device on the left to start sending',
    waitOffTitle:'Devices are offline', waitOffSub:'Kept devices still accept sends; delivery waits until they are online',
    dropHint:'📁 Drop files to send', chooseFiles:'Choose files', sendText:'Send text', recent:'Recent',
    dropSendTo:'Drop to send to "{0}"', dropPick:'Drop, then choose a recipient', dropNoDev:'No device to send to',
    sendTo:'Send to whom?', cancel:'Cancel', sendBtn:'Send',
    sendTextT:'Send text', textPh:'Text to send…',
    portL:'Port',
    noDevices:'No devices yet', sendingN:'Sending {0} files', folderEmpty:'No sendable files in that folder',
    clipEmpty:'Clipboard has nothing to send',
    clipDenied:"Can't read clipboard; paste with ⌘/Ctrl+V instead", clipImgName:'clipboard-image',
    sentT:'Delivered: {0}', sendFail:'Send failed',
    textSent:'Text delivered', retrying:'Retrying…', retryFail:'Retry failed',
    bigFileHint:'{0} is large: drag-and-drop relay has no resume — prefer the "Choose files" button (resumable)',
    cancelT:'Cancel transfer', cancelSendT:'Send cancel requested (retry resumes from checkpoint)',
    cancelRxT:'Reception cancelled (retries declined for 5 minutes)', cancelFail:'Cancel failed',
    onlineT:'Online: {0}', offlineT:'Offline: {0}', receivedT:'Received: {0}',
    removed:'Removed "{0}"; it reappears when back online', removeFail:'Remove failed', openFail:'Open failed',
    setTitle:'Settings', secGeneral:'General', secStorage:'Storage', secAbout:'About',
    theme:'Theme', themeDesc:'Follow system or pin light / dark',
    themeSys:'System', themeLight:'Light', themeDark:'Dark',
    langL:'Language', langDesc:'Interface language', langZh:'中文', langEn:'English',
    saveDir:'Default save location', saveDirDesc:'Where received files are stored', saveDirHint:'Click Save to apply',
    browse:'Browse…', save:'Save', dirEmpty:'Location is empty', dirSaved:'Save location updated', setDirFail:'Could not save location',
    pickUnsupported:'Native picker unsupported here; paste a path instead',
    versionL:'Version', checkUpd:'Check for updates', updChecking:'Checking…',
    updLatest:'Up to date ({0})', updFound:'New version {0} available', updFail:'Check failed',
    updOpen:'Open download page',
    nodeInfo:'Node info', aliasL:'Alias', fpL:'Fingerprint', protocolL:'Protocol',
    copyFp:'Copy fingerprint', fpCopied:'Fingerprint copied', close:'Close',
  },
};
// localStorage 在部分 WebView 环境会抛异常（隐私模式等），读写都包一层
const lsGet = k => { try{ return localStorage.getItem(k); }catch(_){ return null; } };
const lsSet = (k,v) => { try{ localStorage.setItem(k,v); }catch(_){} };
let lang = lsGet('antify-lang') === 'en' ? 'en' : 'zh';
function t(k, ...a){
  let s = (L[lang] && L[lang][k] != null) ? L[lang][k] : (L.zh[k] != null ? L.zh[k] : k);
  a.forEach((v,i) => { s = s.split('{'+i+'}').join(String(v)); }); // 不走 replace，避免 $ 序列
  return s;
}

// ── 主题：system / light / dark，data-theme 属性驱动 CSS 变量 ──
let theme = (v => v==='light'||v==='dark'?v:'system')(lsGet('antify-theme'));
function applyTheme(v){
  theme = v; lsSet('antify-theme', v);
  if (v === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = v;
}

const LOCALE = () => lang === 'zh' ? 'zh-CN' : 'en-US';
const fmtSize = n => { n=+n||0;
  if(n<1024) return n+' B';
  if(n<1048576) return (n/1024).toFixed(1)+' KB';
  if(n<1073741824) return (n/1048576).toFixed(1)+' MB';
  return (n/1073741824).toFixed(2)+' GB'; };
const fmtAgo = ms => {
  if (lang === 'zh'){
    return ms<3000?'刚刚':ms<60000?Math.round(ms/1000)+' 秒前':ms<3600000?Math.round(ms/60000)+' 分钟前'
      :ms<86400000?Math.round(ms/3600000)+' 小时前':ms<30*86400000?Math.round(ms/86400000)+' 天前'
      :new Date(Date.now()-ms).toLocaleDateString('zh-CN');
  }
  return ms<3000?'just now':ms<60000?Math.round(ms/1000)+'s ago':ms<3600000?Math.round(ms/60000)+'min ago'
    :ms<86400000?Math.round(ms/3600000)+'h ago':ms<30*86400000?Math.round(ms/86400000)+'d ago'
    :new Date(Date.now()-ms).toLocaleDateString('en-US');
};
const fmtTime = ms => new Date(ms).toLocaleTimeString(LOCALE(),{hour12:false});
// UI 上不展示设备别名末尾的品牌蚂蚁标记，协议/数据库仍保留原始别名。
const displayAlias = alias => String(alias || '').replace(/\s*🐜\uFE0F?\s*$/u, '').trim();
// 端口可能暂未上报；此时仅显示 IP，绝不把 undefined 渲染到界面上。
const formatAddress = (ip, port) => {
  const host = String(ip ?? '').trim();
  const p = port == null ? '' : String(port).trim();
  return host && p ? `${host}:${p}` : host;
};
// Tauri 的原生交通灯在失焦时会淡到几乎不可见；同步一个灰色覆盖层。
const syncWindowFocus = () => document.body.classList.toggle('window-inactive', !document.hasFocus());
window.addEventListener('focus', syncWindowFocus);
window.addEventListener('blur', syncWindowFocus);
syncWindowFocus();
// 图片判定（与服务端 /api/ui/asset 的扩展名白名单保持一致）
const IMG_RE = /\.(png|jpe?g|jfif|gif|webp|bmp|heic|heif|avif)$/i;
const isImg = n => IMG_RE.test(String(n || ''));

let lastReceivedAt = 0, lastDevJson = '', lastRxJson = '', firstRender = true;
let sel = null;            // 当前选中的设备指纹（null = 未选，主区显示等待屏）
let pendingTarget = null;  // filepick 的目标
let lastChatKey = '', lastChatSel = null; // 会话气泡防抖重绘 + 切换会话时强制贴底
const onlineMap = new Map(); // fp → 上一轮在线态（翻转时提醒上线/离线）
let updUrl = '';           // 检查更新拿到的发布页地址
let fileMenuCtx = null;

function closeFileMenu(){
  $('filemenu').hidden = true;
  fileMenuCtx = null;
}
function showFileMenu(e, path, open){
  e.preventDefault();
  fileMenuCtx = {path, open};
  const menu = $('filemenu'); menu.hidden = false;
  const pad = 8, rect = menu.getBoundingClientRect();
  menu.style.left = Math.min(e.clientX, window.innerWidth - rect.width - pad) + 'px';
  menu.style.top = Math.min(e.clientY, window.innerHeight - rect.height - pad) + 'px';
}
$('filemenu-copy').onclick = () => {
  if (!fileMenuCtx) return;
  const {path} = fileMenuCtx, done = () => toast(t('copied'));
  if (navigator.clipboard) navigator.clipboard.writeText(path).then(done).catch(() => fallbackCopy(path, done));
  else fallbackCopy(path, done);
  closeFileMenu();
};
$('filemenu-open').onclick = () => { if (fileMenuCtx) fileMenuCtx.open(); closeFileMenu(); };
document.addEventListener('pointerdown', e => { if (!e.target.closest('#filemenu')) closeFileMenu(); });
document.addEventListener('keydown', e => { if (e.key === 'Escape') closeFileMenu(); });
document.addEventListener('scroll', closeFileMenu, true);

function toast(msg, err){
  const box = $('toasts');
  const el = document.createElement('div');
  el.className = 'toast' + (err ? ' err' : '');
  el.textContent = msg;
  box.appendChild(el);
  requestAnimationFrame(() => el.classList.add('in'));
  setTimeout(() => { el.classList.remove('in'); setTimeout(() => el.remove(), 300); }, 3200);
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

// 系统品牌图标（设备行 .ico 与目标选择胶囊共用）：mac 为 Apple 标志剪影（iconfont），
// currentColor 随主题文字色；Windows 为微软四格标，微软品牌蓝。deviceModel 是官方 App
// 桌面端固定上报的 "macOS"/"Windows"/"Linux"（AntifyBot 对端为 "macOS (antify-rs)" 等），
// 别名兜底（官方允许自定义 deviceModel）
const ICO_MAC = '<svg viewBox="130 92 725 842" aria-hidden="true"><path fill="currentColor" d="M849.124134 704.896288c-1.040702 3.157923-17.300015 59.872622-57.250912 118.190843-34.577516 50.305733-70.331835 101.018741-126.801964 101.909018-55.532781 0.976234-73.303516-33.134655-136.707568-33.134655-63.323211 0-83.23061 32.244378-135.712915 34.110889-54.254671 2.220574-96.003518-54.951543-130.712017-105.011682-70.934562-102.549607-125.552507-290.600541-52.30118-416.625816 36.040844-63.055105 100.821243-103.135962 171.364903-104.230899 53.160757-1.004887 103.739712 36.012192 136.028093 36.012192 33.171494 0 94.357018-44.791136 158.90615-38.089503 27.02654 1.151219 102.622262 11.298324 151.328567 81.891102-3.832282 2.607384-90.452081 53.724599-89.487104 157.76107C739.079832 663.275355 847.952448 704.467523 849.124134 704.896288M633.69669 230.749408c29.107945-35.506678 48.235584-84.314291 43.202964-132.785236-41.560558 1.630127-92.196819 27.600615-122.291231 62.896492-26.609031 30.794353-50.062186 80.362282-43.521213 128.270409C557.264926 291.935955 604.745311 264.949324 633.69669 230.749408"/></svg>';
const ICO_WIN = '<svg viewBox="0 0 1024 1024" aria-hidden="true"><path fill="#0078D7" d="M0 139.392L409.429333 81.92l0.170667 407.210667-409.216 2.389333L0 139.392z m409.301333 395.818667L409.6 942.08 0 884.181333V532.48l409.301333 2.730667z m41.258667-454.186667L1024 0v487.125333l-573.44 4.394667V81.024zM1024 533.333333L1023.872 1024l-572.501333-79.274667-0.810667-412.245333 573.44 0.896z"/></svg>';
// 本机行：面板跑在宿主机上，UA 即宿主系统
const SELF_ICON = /windows/i.test(navigator.userAgent) ? ICO_WIN
  : /mac/i.test(navigator.userAgent) ? ICO_MAC : '🖥️';

function emoji(d){
  const t = d.deviceType || '', m = (d.deviceModel||'') + (d.alias||'');
  if (t === 'mobile' || /iPhone|iPad|手机/i.test(m)) return '📱';
  if (t === 'web') return '🌐';
  if (t === 'server') return '🖧';
  return '🖥️';
}
function devIcon(d){
  const mo = d.deviceModel || '';
  if (/mac/i.test(mo)) return ICO_MAC;
  if (/win/i.test(mo)) return ICO_WIN;
  const a = d.alias || '';
  if (/mac|苹果/i.test(a)) return ICO_MAC;
  if (/windows|win|pc|电脑/i.test(a)) return ICO_WIN;
  return emoji(d);
}

function hasFiles(e){
  return !!(e.dataTransfer && [...(e.dataTransfer.types||[])].includes('Files'));
}

function render(s){
  window._state = s;
  const me = s.me || {};
  const selfFp = me.fingerprint || '';
  const selfIps = me.ips || [];
  // 本机地址串（自我会话与左侧本机设备项展示）
  const selfAddrs = selfIps.map(ip => formatAddress(ip, me.port)).filter(Boolean);
  const selfAddr0 = selfAddrs[0] || '';

  // ── 侧栏设备（内容有变化才重绘，避免打断 hover / 拖放；key 含语言，切语言强制重绘） ──
  // 会话列表式：第二行显示与该设备的最近一条消息预览；离线设备置灰保留；
  // 本机钉在首位（自我会话，无 ✕、拒收文件拖放）
  const lastByFp = {};
  for (const m of (s.chat||[])) lastByFp[m.peer] = m;
  // 防抖 key：lastSeenMs 每轮轮询都在变，按分钟分桶；identity/在线态/预览 id 变了才重绘
  const dj = lang + ':' + me.alias + ':' + selfAddrs.join(',') + ':' +
    ((lastByFp[selfFp]||{}).id||0) + '|' + s.devices.map(d =>
    d.fingerprint + ':' + d.alias + ':' + d.ip + ':' + (d.online?1:0) + ':' +
    Math.floor(d.lastSeenMs/60000) + ':' + ((lastByFp[d.fingerprint]||{}).id||0)
  ).join('|');
  if (dj !== lastDevJson){
    lastDevJson = dj;
    const box = $('devices');
    box.innerHTML = '';
    $('devempty').hidden = s.devices.length > 0;
    if (selfFp){
      const row = document.createElement('div');
      row.className = 'dev' + (sel === selfFp ? ' on' : '');
      row.dataset.fp = selfFp; row.tabIndex = 0; row.title = t('selfName', me.alias);
      const ico = document.createElement('div'); ico.className = 'ico'; ico.innerHTML = SELF_ICON;
      const mid = document.createElement('div'); mid.className = 'mid';
      const nm = document.createElement('div'); nm.className = 'nm'; nm.textContent = t('selfName', me.alias);
      const meta = document.createElement('div'); meta.className = 'meta';
      meta.textContent = selfAddr0 || formatAddress('127.0.0.1', me.port);
      mid.append(nm, meta);
      row.append(ico, mid);
      row.onclick = () => { sel = (sel === selfFp ? null : selfFp); syncSel(); };
      // 拖文件到本机行：高亮照常，松手明确提示不支持
      row.ondragover = e => { if (hasFiles(e)){ e.preventDefault();
        e.dataTransfer.dropEffect = 'copy'; row.classList.add('dropover'); } };
      row.ondragleave = () => row.classList.remove('dropover');
      row.ondrop = e => { if (!hasFiles(e)) return;
        e.preventDefault(); e.stopPropagation(); row.classList.remove('dropover');
        toast(t('selfNoFiles'), true); };
      box.appendChild(row);
    }
    for (const d of s.devices){
      const row = document.createElement('div');
      row.className = 'dev' + (d.fingerprint === sel ? ' on' : '') + (d.online ? '' : ' off');
      row.dataset.fp = d.fingerprint; row.tabIndex = 0; row.title = displayAlias(d.alias);
      const ico = document.createElement('div'); ico.className = 'ico'; ico.innerHTML = devIcon(d);
      const mid = document.createElement('div'); mid.className = 'mid';
      const nm = document.createElement('div'); nm.className = 'nm'; nm.textContent = displayAlias(d.alias);
      const meta = document.createElement('div'); meta.className = 'meta';
      meta.textContent = formatAddress(d.ip, d.port);
      mid.append(nm, meta);
      const x = document.createElement('button'); x.className = 'xbtn';
      x.textContent = '✕'; x.title = t('removeDev');
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
      if (prev === undefined || (!prev && d.online)) toast(t('onlineT', d.alias));
      else if (prev && !d.online) toast(t('offlineT', d.alias));
      onlineMap.set(d.fingerprint, d.online);
    }
    // 从列表消失（被移除的设备有自己的 toast）→ 静默清记录
    for (const f of [...onlineMap.keys()]) if (!nowFp.has(f)) onlineMap.delete(f);
  }

  // 选中设备被移除 → 回到等待屏（离线设备保留在列表，不触发；本机是钉住项，不参与）
  if (sel && sel !== selfFp && !s.devices.some(d => d.fingerprint === sel)) sel = null;
  // 正在收文件且没开任何会话 → 自动切到发送者的会话（进度卡可见）
  if (!sel && s.session && s.session.active){
    const d = (s.devices||[]).find(x => x.ip === s.session.senderIp);
    if (d) sel = d.fingerprint;
  }
  renderCenter(s);
  renderChat(s);

  // ── 传输进行中 → 主区顶部进度卡（右上 ✕ 取消） ──
  const busy = [];
  if (s.sending && s.sending.active){
    const p = s.sending, pct = p.total ? Math.min(100, p.sent/p.total*100) : 0;
    busy.push(`<div class="prog"><div class="row"><b>↑ ${esc(t('progSend', p.target_alias, p.file_name))}</b>
      <span class="sz">${fmtSize(p.sent)} / ${fmtSize(p.total)}</span>
      <button class="xbtn" title="${esc(t('cancelT'))}" aria-label="${esc(t('cancelT'))}" onclick="cancelSend()">✕</button></div>
      <div class="bar"><i style="width:${pct}%"></i></div></div>`);
  }
  if (s.session && s.session.active){
    const c = s.session.current || {};
    const pct = c.total ? Math.min(100, (c.got||0)/c.total*100) : 0;
    busy.push(`<div class="prog"><div class="row"><b>↓ ${esc(t('progRecv', s.session.sender, c.name||''))}</b>
      <span class="sz">${fmtSize(c.got)} / ${fmtSize(c.total)}</span>
      <button class="xbtn" title="${esc(t('cancelT'))}" aria-label="${esc(t('cancelT'))}" onclick="cancelRx()">✕</button></div>
      <div class="bar"><i style="width:${pct}%"></i></div>
      <ul class="files">${(s.session.files||[]).map(f =>
        `<li class="${f.done?'ok':''}"><span class="tick">${f.done?'✔':'○'}</span>${esc(f.name)}
         <span class="sz">${fmtSize(f.size)}</span></li>`).join('')}</ul></div>`);
  }
  $('busy').innerHTML = busy.join('');
  $('busy').hidden = !busy.length;

  // ── 最近接收（空则整节隐藏） ──
  const rj = lang + ':' + JSON.stringify(s.received);
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
      b.textContent = t('show'); b.onclick = () => reveal(r.file);
      li.append(nm, sz, b);
      ul.appendChild(li);
    }
  }
  if (s.received.length && s.received[0].at > lastReceivedAt){
    if (lastReceivedAt)
      s.received.filter(r => r.at > lastReceivedAt).forEach(r => toast(t('receivedT', r.name)));
    lastReceivedAt = s.received[0].at;
  }
}

// 主区：选中设备 → 会话视图（气泡 + 聊天输入条）；未选 → 等待屏
// 本机（侧栏钉住项）也走会话视图：头部显示本机全部 ip:端口，文件 / 文件夹工具停用
function renderCenter(s){
  s = s || window._state; if (!s) return;
  const me = s.me || {};
  const selfFp = me.fingerprint || '';
  const isSelf = !!selfFp && sel === selfFp;
  const d = isSelf
    ? { alias: me.alias, online: true, deviceType: 'self', lastSeenMs: 0 }
    : (s.devices||[]).find(x => x.fingerprint === sel);
  $('chatview').hidden = !d;
  $('waitview').hidden = !!d;
  // 本机会话仅文字：文件 / 文件夹按钮停用（剪贴板按钮保留——粘贴文本仍可用）
  $('btn-cfile').disabled = isSelf;
  $('btn-cfolder').disabled = isSelf;
  if (d){
    let meta, alias;
    if (isSelf){
      const addrs = (me.ips||[]).map(ip => formatAddress(ip, me.port)).filter(Boolean);
      meta = addrs.length ? addrs.join(' · ') : formatAddress('127.0.0.1', me.port);
      alias = displayAlias(me.alias);
      $('ch-name').textContent = alias;
    } else {
      alias = displayAlias(d.alias);
      $('ch-name').textContent = alias;
      meta = formatAddress(d.ip, d.port);
    }
    $('ch-meta').textContent = meta;
    $('chrome-title').textContent = alias;
    $('chrome-context').textContent = meta;
  } else {
    $('chrome-title').textContent = t('devices');
    $('chrome-context').textContent = '';
    const allOff = s.devices.length && !s.devices.some(x => x.online);
    $('c-title').textContent = allOff ? t('waitOffTitle') : t('waitTitle');
    $('c-sub').textContent = allOff
      ? t('waitOffSub')
      : s.devices.length
        ? t('waitSubSome')
        : t('waitSubNone');
    $('dropzone').textContent = t('dropHint');
  }
}

// 时间分组标签（居中显示；跨天带日期）
function fmtDay(ms){
  const d = new Date(ms), n = new Date();
  const hm = d.toLocaleTimeString(LOCALE(), {hour12:false, hour:'2-digit', minute:'2-digit'});
  return d.toDateString() === n.toDateString()
    ? hm
    : d.toLocaleDateString(LOCALE(), {month:'numeric', day:'numeric'}) + ' ' + hm;
}

// 会话消息流（ChatGPT 式：出站灰气泡、入站纯文本、时间居中分组、文件中性卡片；内容无变化不重绘）
function renderChat(s){
  const sc = $('cscroll'), box = $('chatlist');
  if (!s || !sel){ lastChatKey = ''; box.innerHTML = ''; return; }
  const selfFp = (s.me||{}).fingerprint || '';
  const isSelf = !!selfFp && sel === selfFp; // 本机会话：无出站状态行，空态文案不同
  const msgs = (s.chat||[]).filter(m => m.peer === sel);
  // 防抖 key 含语言与出站状态：⏳→✓/⚠ 翻转、切语言都必须重绘
  const key = lang + ':' + sel + ':' + msgs.map(m => m.id + ':' + (m.out ? m.status : '')).join(',');
  const switched = lastChatSel !== sel;
  if (key === lastChatKey && !switched) return;
  lastChatKey = key; lastChatSel = sel;
  const nearBottom = sc.scrollHeight - sc.scrollTop - sc.clientHeight < 90;
  box.innerHTML = '';
  if (!msgs.length){
    const d = isSelf
      ? { alias: (s.me||{}).alias }
      : (s.devices||[]).find(x => x.fingerprint === sel);
    const hint = document.createElement('div'); hint.className = 'chathint';
    hint.innerHTML = `${esc(t('chatHint1', d ? d.alias : ''))}<br><small>${esc(t(isSelf ? 'chatHint2Self' : 'chatHint2'))}</small>`;
    box.appendChild(hint);
  }
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
    if (m.kind === 'file'){
      // 收到文件位于下载目录，原生选择器发送的文件保存了源绝对路径。
      const localPath = m.file
        ? ((s.me && s.me.dir ? s.me.dir.replace(/[\\/]$/, '') +
            (s.me.dir.includes('\\') ? '\\' : '/') : '') + m.file)
        : (m.srcPath || '');
      if (localPath){
        bub.title = t('filePath');
        bub.oncontextmenu = e => showFileMenu(e, localPath,
          () => { m.file ? reveal(m.file) : revealSource(m.id); });
      }
    }
    if (m.kind === 'text' && !m.out){ // 文字不落盘，「复制」是取走内容的唯一途径
      const cp = document.createElement('button'); cp.className = 'rxb';
      cp.textContent = t('copy');
      cp.onclick = () => { const txt = m.text, done = () => toast(t('copied'));
        if (navigator.clipboard)
          navigator.clipboard.writeText(txt).then(done).catch(() => fallbackCopy(txt, done));
        else fallbackCopy(txt, done); };
      col.appendChild(cp);
    }
    if (m.out && !isSelf){ // 出站消息状态行：⏳ 发送中 / ✓ 已送达 / ⚠ 未送达（文本与带源路径的可重试；本机即时落库无需状态）
      const st = document.createElement('div'); st.className = 'st';
      if (m.status === 'sending') st.textContent = t('stSending');
      else if (m.status === 'ok') st.textContent = t('stOk');
      else if (m.status === 'fail'){
        if (m.kind === 'text' || m.srcPath){
          st.textContent = t('stFail');
          const rb = document.createElement('button'); rb.className = 'rbtn';
          rb.textContent = t('stRetry');
          rb.onclick = () => retryMsg(m);
          st.appendChild(rb);
        } else st.textContent = t('stFailRedrag');
      }
      if (st.textContent) col.appendChild(st); // 入站与旧数据无状态行
    }
    crow.appendChild(col);
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
    // 捎带焦点上报：可见且聚焦 → 节点静音系统通知（收托盘 / 切后台后停报，~6s 自动恢复提醒）
    const watching = document.visibilityState === 'visible' && document.hasFocus() ? 1 : 0;
    const r = await fetch('/api/ui/state?focused=' + watching);
    if (r.ok) render(await r.json());
  }catch(_){ /* 服务重启中 */ }
  setTimeout(poll, 1200);
}

// ── 发送 ──
// 本机会话仅文字：文件 / 文件夹在此统一拦截（拖放、粘贴、剪贴板、按钮全部收口）
const isSelfTarget = fp => !!fp && window._state && fp === (window._state.me||{}).fingerprint;
async function sendFiles(fp, files){
  if (isSelfTarget(fp)) return toast(t('selfNoFiles'), true);
  sel = fp; syncSel(); // 发送即切到目标会话（进度卡在会话流顶部可见）
  // 浏览器流式中转不可 seek、无断点续传：大文件只提示（不阻断），引导走原生选择器
  const BIG = 512 * 1024 * 1024;
  const big = files.filter(f => f.size > BIG).sort((a,b) => b.size - a.size)[0];
  if (big) toast(t('bigFileHint', big.name), true);
  for (const f of files){
    try{
      const q = `target=${encodeURIComponent(fp)}&name=${encodeURIComponent(f.name)}`+
                `&size=${f.size}&mime=${encodeURIComponent(f.type||'application/octet-stream')}`;
      const r = await fetch('/api/ui/send?' + q, {method:'POST', body:f});
      const v = await r.json().catch(()=>({}));
      if (r.ok) toast(t('sentT', f.name));
      else { toast(v.error || t('sendFail'), true); break; }
    }catch(e){ toast(t('sendFail')+'：'+e.message, true); break; }
  }
}

// ── 取消传输（进度卡 ✕）：发送侧置标志由发送循环感知；接收侧清会话 + 短期拒收 ──
async function cancelSend(){
  try{
    const r = await fetch('/api/ui/cancel-send', {method:'POST'});
    if (r.ok) toast(t('cancelSendT'));
    else { const v = await r.json().catch(()=>({})); toast(v.error || t('cancelFail'), true); }
  }catch(_){ toast(t('cancelFail'), true); }
}
async function cancelRx(){
  try{
    const r = await fetch('/api/ui/cancel-rx', {method:'POST'});
    if (r.ok) toast(t('cancelRxT'));
    else { const v = await r.json().catch(()=>({})); toast(v.error || t('cancelFail'), true); }
  }catch(_){ toast(t('cancelFail'), true); }
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
  if (isSelfTarget(fp)) return toast(t('selfNoFiles'), true);
  sel = fp; syncSel();
  for (const p of paths){
    try{
      const r = await fetch('/api/ui/send-path', {method:'POST',
        headers:{'Content-Type':'application/json'}, body: JSON.stringify({target: fp, path: p})});
      const v = await r.json().catch(()=>({}));
      if (!r.ok && !((v.ids||[]).length)){ toast(v.error || t('sendFail'), true); break; }
      if (!r.ok && v.error) toast(v.error, true); // 部分成功：失败项已有 ⚠ 气泡
    }catch(e){ toast(t('sendFail')+'：'+e.message, true); break; }
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
  if (!ds.length) return toast(t('noDevices'), true);
  if (sel) return cb(sel);
  if (ds.length === 1) return cb(ds[0].fingerprint);
  openPick(cb);
}

// 多台时弹胶囊选择（选择后回调目标指纹）
function openPick(cb){
  const ds = (window._state && window._state.devices) || [];
  if (!ds.length) return toast(t('noDevices'), true);
  const list = $('picklist');
  list.innerHTML = '';
  for (const d of ds){
    const b = document.createElement('button'); b.className = 'pick';
    b.innerHTML = '<span class="icochip">' + devIcon(d) + '</span>' + esc(d.alias);
    b.onclick = () => { $('pickdlg').close(); cb(d.fingerprint); };
    list.appendChild(b);
  }
  $('pickdlg').showModal();
}
$('pickcancel').onclick = () => $('pickdlg').close();

$('btn-file').onclick = () => resolveTarget(fp => pickNative('file', fp));

// 拖放已收口到页尾「全局拖拽」：窗口任意位置松手即发送（设备行定向投放优先，见下）

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
        toast(v.error || t('sendFail'), true);
      }
    })
    .catch(e => toast(t('sendFail')+'：'+e.message, true))
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
    toast(t('sendingN', files.length));
    sendFiles(pendingFolderTarget, files);
  } else if (!files.length) toast(t('folderEmpty'), true);
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
        const imgType = it.types.find(x => x.startsWith('image/'));
        if (imgType){
          const blob = await it.getType(imgType);
          const ext = (imgType.split('/')[1] || 'png').replace('+xml', '');
          const f = new File([blob], `${t('clipImgName')}.${ext}`, {type: imgType});
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
      toast(t('clipEmpty'), true);
      return;
    }
  }catch(_){ /* 无剪贴板读取权限 */ }
  toast(t('clipDenied'), true);
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
  if (!ds.length) return toast(t('noDevices'), true);
  const box = $('texttargets');
  box.innerHTML = '';
  let cur = preselect || sel || (ds[0] && ds[0].fingerprint);
  const syncs = ds.map(d => {
    const b = document.createElement('button'); b.className = 'pick';
    b.innerHTML = '<span class="icochip">' + devIcon(d) + '</span>' + esc(d.alias);
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
      if (r.ok){ toast(t('textSent')); $('textdlg').close(); }
      else toast(v.error || t('sendFail'), true);
    }catch(e){ toast(t('sendFail')+'：'+e.message, true); }
    $('textsend').disabled = false;
  };
}
$('textcancel').onclick = () => $('textdlg').close();
$('btn-text').onclick = () => openText(null);

// 重试未送达的消息（服务端校验可重试性：文本或有源路径的文件；返回后状态经轮询刷新）
async function retryMsg(m){
  try{
    const r = await fetch('/api/ui/retry', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({id: m.id})});
    const v = await r.json().catch(()=>({}));
    if (r.ok) toast(t('retrying'));
    else toast(v.error || t('retryFail'), true);
  }catch(e){ toast(t('retryFail')+'：'+e.message, true); }
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
      toast(t('removed', d.alias));
    } else toast(v.error || t('removeFail'), true);
  }catch(e){ toast(t('removeFail')+'：'+e.message, true); }
}

// 在文件管理器中显示
async function reveal(file){
  const r = await fetch('/api/ui/reveal', {method:'POST',
    headers:{'Content-Type':'application/json'}, body: JSON.stringify({file})});
  if (!r.ok){ const v = await r.json().catch(()=>({})); toast(v.error || t('openFail'), true); }
}

async function revealSource(messageId){
  const r = await fetch('/api/ui/reveal', {method:'POST',
    headers:{'Content-Type':'application/json'}, body: JSON.stringify({messageId})});
  if (!r.ok){ const v = await r.json().catch(()=>({})); toast(v.error || t('openFail'), true); }
}

function fallbackCopy(txt, done){
  const ta = document.createElement('textarea');
  ta.value = txt; ta.style.position = 'fixed'; ta.style.opacity = '0';
  document.body.appendChild(ta); ta.select();
  try{ document.execCommand('copy'); done(); }catch(_){ toast(t('copyFail'), true); }
  ta.remove();
}

// ── 设置面板：通用（主题 / 语言）· 存储（保存地址）· 关于（版本 / 更新 / 节点信息） ──
function applyLang(v){
  lang = v; lsSet('antify-lang', v);
  document.documentElement.lang = v === 'zh' ? 'zh-CN' : 'en';
  document.querySelectorAll('[data-t]').forEach(el => el.textContent = t(el.dataset.t));
  document.querySelectorAll('[data-tp]').forEach(el => el.placeholder = t(el.dataset.tp));
  document.querySelectorAll('[data-tt]').forEach(el => el.title = t(el.dataset.tt));
  lastDevJson = ''; lastRxJson = ''; lastChatKey = ''; // 切语言强制重绘所有动态文案
  syncPicks();
  if (window._state) render(window._state);
}

function syncPicks(){
  document.querySelectorAll('#themepick .pick').forEach(b =>
    b.classList.toggle('on', b.dataset.v === theme));
  document.querySelectorAll('#langpick .pick').forEach(b =>
    b.classList.toggle('on', b.dataset.v === lang));
}
function selectSettingsPanel(name){
  document.querySelectorAll('[data-set-tab]').forEach(b => {
    const on = b.dataset.setTab === name;
    b.classList.toggle('on', on);
    b.setAttribute('aria-selected', String(on));
  });
  document.querySelectorAll('[data-set-panel]').forEach(p => {
    p.hidden = p.dataset.setPanel !== name;
  });
}
document.querySelectorAll('[data-set-tab]').forEach(b => {
  b.onclick = () => selectSettingsPanel(b.dataset.setTab);
});
$('themepick').addEventListener('click', e => {
  const b = e.target.closest('.pick'); if (!b) return;
  applyTheme(b.dataset.v); syncPicks();
});
$('langpick').addEventListener('click', e => {
  const b = e.target.closest('.pick'); if (!b) return;
  applyLang(b.dataset.v);
});

$('btn-sidebar').onclick = () => {
  const root = document.documentElement;
  root.classList.toggle('sidebar-collapsed');
  const collapsed = root.classList.contains('sidebar-collapsed');
  $('btn-sidebar').title = collapsed ? '展开侧栏' : '收起侧栏';
  $('btn-sidebar').setAttribute('aria-label', collapsed ? '展开侧栏' : '收起侧栏');
  lsSet('antify-sidebar', collapsed ? 'collapsed' : 'expanded');
};

$('btn-settings').onclick = () => {
  const me = window._state && window._state.me;
  if (!me) return;
  $('st-alias').textContent = me.alias;
  $('st-port').textContent = me.port;
  $('st-fp').textContent = me.fingerprint;
  $('st-ver').textContent = 'LocalSend v' + me.version;
  $('st-appver').textContent = 'v' + (me.appVersion || '?');
  $('set-dir').value = me.dir;
  updUrl = '';
  $('upd-state').textContent = '';
  $('updresult').hidden = true;
  selectSettingsPanel('general');
  syncPicks();
  $('setdlg').showModal();
};
$('setclose').onclick = () => $('setdlg').close();

$('st-copy').onclick = () => {
  const txt = $('st-fp').textContent;
  const done = () => toast(t('fpCopied'));
  if (navigator.clipboard) navigator.clipboard.writeText(txt).then(done).catch(() => fallbackCopy(txt, done));
  else fallbackCopy(txt, done);
};

// 保存地址：原生目录选择器（501 / 失败时提示直接粘贴路径）+ 显式保存
$('set-dirbrowse').onclick = async () => {
  try{
    const r = await fetch('/api/ui/pick', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({kind:'folder'})});
    const v = await r.json().catch(()=>null);
    if (r.ok && v && Array.isArray(v.paths) && v.paths[0]){
      $('set-dir').value = v.paths[0];
      return;
    }
    if (r.status === 501) toast(t('pickUnsupported'), true);
  }catch(_){ toast(t('pickUnsupported'), true); }
};
$('set-dirsave').onclick = async () => {
  const dir = $('set-dir').value.trim();
  if (!dir) return toast(t('dirEmpty'), true);
  const btn = $('set-dirsave'); btn.disabled = true;
  try{
    const r = await fetch('/api/ui/set-dir', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({dir})});
    const v = await r.json().catch(()=>({}));
    if (r.ok){ toast(t('dirSaved')); if (v.dir) $('set-dir').value = v.dir; }
    else toast(v.error || t('setDirFail'), true);
  }catch(_){ toast(t('setDirFail'), true); }
  btn.disabled = false;
};

// 检查更新：GitHub Releases；有新版 →「打开下载页」走后端系统浏览器（WebView 打不开外链）
$('btn-checkupd').onclick = async () => {
  const btn = $('btn-checkupd'), st = $('upd-state');
  btn.disabled = true;
  st.textContent = t('updChecking');
  try{
    const r = await fetch('/api/ui/check-update', {method:'POST'});
    const v = await r.json().catch(()=>({}));
    if (!r.ok){ st.textContent = t('updFail'); toast(v.error || t('updFail'), true); }
    else if (v.newer){
      st.textContent = t('updFound', v.latest);
      updUrl = v.url || '';
      $('updresult').hidden = !updUrl;
    } else st.textContent = t('updLatest', v.latest);
  }catch(_){ st.textContent = t('updFail'); }
  btn.disabled = false;
};
$('upd-open').onclick = async () => {
  if (!updUrl) return;
  try{
    const r = await fetch('/api/ui/open-url', {method:'POST',
      headers:{'Content-Type':'application/json'}, body: JSON.stringify({url: updUrl})});
    if (!r.ok){ const v = await r.json().catch(()=>({})); toast(v.error || t('openFail'), true); }
  }catch(_){ toast(t('openFail'), true); }
};

// ── 全局拖拽：整个窗口都是投放区 ──
// 拖动中：全窗 accent 高亮 + 顶部胶囊提示默认目标（设备行的 dropover 定向高亮优先于胶囊文案）；
// 松手：侧栏设备行自带定向投放（ondrop 已 stopPropagation，不会冒泡到这里），
// 其余任意位置（顶栏 / 侧栏空白 / 会话区 / 等待屏 / 输入盒……）按
// 当前会话 → 唯一设备 → 弹窗选择 解析目标。
function dropTargetText(){
  const st = window._state || {}, ds = st.devices || [];
  const meFp = (st.me||{}).fingerprint || '';
  if (sel){
    if (sel === meFp) return t('selfNoFiles');
    const d = ds.find(x => x.fingerprint === sel);
    if (d) return t('dropSendTo', d.alias);
  }
  if (!ds.length) return t('dropNoDev');
  if (ds.length === 1) return t('dropSendTo', ds[0].alias);
  return t('dropPick');
}
let depth = 0;
document.addEventListener('dragenter', e => {
  if (hasFiles(e)){ e.preventDefault(); depth++; document.body.classList.add('dropping');
    $('dropveil-hint').textContent = dropTargetText(); }
});
document.addEventListener('dragover', e => { if (hasFiles(e)) e.preventDefault(); });
document.addEventListener('dragleave', e => {
  if (hasFiles(e)){ depth = Math.max(0, depth-1);
    if (!depth) document.body.classList.remove('dropping'); }
});
document.addEventListener('drop', e => {
  if (!hasFiles(e)) return;
  e.preventDefault(); depth = 0; document.body.classList.remove('dropping');
  const files = [...e.dataTransfer.files];
  if (sel) sendFiles(sel, files); // 有会话 → 当前设备（本机目标已在 sendFiles 内拦截提示）
  else resolveTarget(fp => sendFiles(fp, files)); // 无会话 → 唯一设备直发 / 多台弹窗选择
});

// 启动：套用已保存的主题 / 语言，随后开始轮询
applyTheme(theme);
applyLang(lang);
poll();
</script>
</body>
</html>"##;
