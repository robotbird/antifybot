/* Conversation-owned folder cards. No per-file chat messages or progress-driven chat redraws. */
'use strict';
const folderTasks = new Map();
let folderPeer = '', folderPolling = false;
function ft(zh, en){ return lang === 'en' ? en : zh; }
function folderButton(label, action){
  const b = document.createElement('button'); b.className = 'rbtn'; b.textContent = label;
  b.onclick = async () => {
    b.disabled = true;
    try { await action(); } catch(e) { toast(e.message, true); }
    finally { b.disabled = false; }
  };
  return b;
}
function folderCard(message, match){
  const card = document.createElement('section'); card.className = 'transfer-card';
  card.dataset.message = message.id;
  const heading = document.createElement('h3');
  heading.textContent = message.text.split('\n')[0] || ft('📁 文件夹', '📁 Folder');
  const info = document.createElement('div'); info.className = 'transfer-info';
  const progress = document.createElement('progress'); progress.max = 100; progress.hidden = true;
  const actions = document.createElement('div'); actions.className = 'transfer-actions';
  const browser = document.createElement('details');
  const summary = document.createElement('summary'); summary.textContent = ft('查看内容（兼容旧版）', 'Browse contents (legacy compatible)');
  const entries = document.createElement('div'); entries.className = 'transfer-browser';
  browser.append(summary, entries);
  browser.ontoggle = () => {
    if (browser.open && !browser.dataset.loaded){
      browser.dataset.loaded = '1'; folderBrowse(entries, {target:match[1].toUpperCase(),token:match[2],path:'',cursor:''});
    }
  };
  card.append(heading, info, progress, actions);
  if (message.out){
    info.textContent = ft('已提供只读访问，可随时撤销。对方接收进度暂不回传；消息送达不代表文件接收完成。',
      'Read-only access until revoked. Delivery of this invitation does not confirm file receipt.');
    actions.append(folderButton(ft('撤销访问', 'Revoke access'), async () => {
      if (!confirm(ft('撤销后，对方不能继续读取此目录。是否继续？', 'Revoke access to this folder?'))) return;
      await shareApi('shares/revoke', {token:match[2]}); info.textContent = ft('访问已撤销', 'Access revoked');
    }));
    const more = document.createElement('details'), title = document.createElement('summary');
    title.textContent = ft('更多', 'More'); more.append(title);
    more.append(folderButton(ft('复制共享地址', 'Copy share address'), async () => {
      if (navigator.clipboard) await navigator.clipboard.writeText(match[0]);
      else fallbackCopy(match[0], () => {});
      toast(t('copied'));
    })); card.append(more);
    return card;
  }
  let selectedDirectory = null;
  info.textContent = ft('接收整个目录，支持中断后继续。需要双方使用支持目录任务的版本。保存到：',
    'Receive the entire folder with resumable transfers. Both peers need folder-task support. Save to: ') + ((window._state || {}).me || {}).dir;
  actions.append(folderButton(ft('接收文件夹', 'Receive folder'), async () => {
    const v = await shareApi('transfers', {message_id:message.id, directory:selectedDirectory});
    folderTasks.set(String(message.id), v.task); updateFolderCard(card, v.task);
  }), folderButton(ft('更改保存位置', 'Change destination'), async () => {
    const v = await shareApi('pick', {kind:'folder'});
    if (v.paths && v.paths[0]) {selectedDirectory = v.paths[0]; info.textContent = ft('保存到：', 'Save to: ') + selectedDirectory;}
  }));
  card.append(browser);
  const details = document.createElement('details'); details.className = 'transfer-logs'; details.hidden = true;
  const title = document.createElement('summary'); title.textContent = ft('详情 / 实时日志', 'Details / live logs');
  const logActions = document.createElement('div'), output = document.createElement('pre');
  output.tabIndex = 0; output.setAttribute('aria-label', ft('传输日志', 'Transfer log'));
  details.append(title, logActions, output); card.append(details);
  card.folderLog = {after:0, lines:[], busy:false};
  const task = folderTasks.get(String(message.id)); if (task) updateFolderCard(card,task);
  return card;
}
function updateFolderCard(card, task){
  card.dataset.task = task.id;
  const info = card.querySelector('.transfer-info'), bar = card.querySelector('progress');
  const labels = {
    queued:ft('排队中','Queued'), connecting:ft('正在连接','Connecting'), scanning:ft('整理目录，总量统计中','Scanning; total not yet known'),
    receiving:ft('接收中','Receiving'), verifying:ft('数据接收完毕，正在校验','Verifying file content'),
    paused:ft('已暂停，断点保留','Paused; progress saved'), interrupted:ft('传输中断，可继续','Interrupted; resumable'),
    partial:ft('部分完成','Partially complete'), done:ft('已完成，内容校验通过','Complete; content verified'), cancelled:ft('已取消，已收文件和断点保留','Cancelled; downloaded files retained')
  };
  const percent = task.scanned && task.bytes > 0 ? Math.min(100, task.received / task.bytes * 100) : null;
  bar.hidden = !task.scanned || !task.bytes; bar.value = percent || 0;
  const speed = task.state === 'receiving' ? (task.speed || 0) : 0;
  let text = labels[task.state] || task.state;
  if (percent !== null) text += ' · ' + percent.toFixed(1) + '%';
  text += '\n' + fmtSize(task.received) + ' / ' + fmtSize(task.bytes) + ' · ' + (speed/1000000).toFixed(1) + ' MB/s';
  if (Number.isFinite(task.eta_seconds) && task.state === 'receiving') text += ' · ' + ft('约剩 ', 'About ') + Math.max(1,Math.ceil(task.eta_seconds/60)) + ft(' 分钟',' min left');
  text += '\n' + task.completed + ' / ' + task.files + ft(' 个文件', ' files');
  if (task.finished) text += ' · ' + ft('总用时 ', 'Total time ') + Math.max(1,Math.round((task.finished-task.created)/1000)) + ft(' 秒（含暂停）',' s (including pauses)');
  if (task.failed) text += ' · ' + ft('失败 ', 'Failed ') + task.failed;
  if (task.current) text += '\n' + task.current;
  text += '\n' + ft('保存到：', 'Saved to: ') + task.destination;
  if (task.error) text += '\n' + task.error;
  info.textContent = text;
  const actions = card.querySelector('.transfer-actions');
  const signature = task.id + ':' + task.state + ':' + lang;
  if (actions.dataset.signature !== signature){
    actions.dataset.signature = signature; actions.replaceChildren();
    const control = action => shareApi('transfers/'+task.id+'/'+action, {});
    if (['queued','connecting','scanning','receiving','verifying'].includes(task.state)){
      actions.append(folderButton(ft('暂停','Pause'), () => control('pause')));
    } else if (['paused','interrupted'].includes(task.state)) {
      actions.append(folderButton(ft('继续','Resume'), () => control('resume')));
    } else if (task.state === 'partial') {
      actions.append(folderButton(ft('重试失败项','Retry failed files'), () => control('retry')));
    }
    if (task.scanned) actions.append(folderButton(ft('打开文件夹','Open folder'), () => control('open')));
    if (!['done','cancelled'].includes(task.state)){
      const more = document.createElement('details'), summary = document.createElement('summary'); summary.textContent = ft('更多','More');
      more.append(summary, folderButton(ft('取消任务','Cancel task'), async () => {
        if (confirm(ft('停止此任务？已接收文件和断点不会删除。','Cancel this task? Received files and partial data will be kept.'))) await control('cancel');
      })); actions.append(more);
    }
  }
  const logs = card.querySelector('.transfer-logs'); logs.hidden = false;
  const logActions = logs.querySelector('div');
  if (logActions.dataset.task !== task.id){
    logActions.dataset.task = task.id; logActions.replaceChildren();
    const link = document.createElement('a'); link.className='rbtn'; link.textContent=ft('导出完整日志','Export full log');
    link.href='/api/ui/transfers/'+task.id+'/export'; link.download='transfer-log.jsonl';
    logActions.append(link, folderButton(ft('加载下一页历史','Load next history page'), () => folderLogs(card)));
  }
  if (logs.open) folderLogs(card);
}
async function folderLogs(card){
  const state = card.folderLog;
  if (!state || state.busy || !card.dataset.task) return;
  state.busy = true;
  try {
    const v = await shareApi('transfers/'+card.dataset.task+'/logs?after='+state.after);
    if (!card.isConnected) return;
    const pre = card.querySelector('pre'); const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 30;
    for (const e of v.events){state.after=e.id;state.lines.push(new Date(e.at).toLocaleTimeString()+' ['+e.level+'] '+e.message);}
    if (state.lines.length>200) state.lines.splice(0,state.lines.length-200);
    if (v.events.length){pre.textContent=state.lines.join('\n');if(atBottom)pre.scrollTop=pre.scrollHeight;}
  } catch(e) { /* Network interruption must not erase visible logs. */ }
  finally { state.busy = false; }
}
async function folderBrowse(box,q){
  const generation = (box.folderGeneration || 0) + 1; box.folderGeneration = generation;
  box.textContent = ft('正在读取…','Loading…');
  try {
    const v = await shareApi('shares/browse',q);
    if (box.folderGeneration !== generation) return;
    box.replaceChildren();
    const caption = document.createElement('p');caption.textContent='/'+q.path;box.append(caption);
    if(q.path) box.append(folderButton(ft('上一级','Parent'),()=>folderBrowse(box,{...q,path:q.path.split('/').slice(0,-1).join('/'),cursor:''})));
    for(const entry of v.entries){
      const row=document.createElement('div'),name=document.createElement('span');
      name.textContent=(entry.directory?'📁 ':'📄 ')+entry.name;row.append(name);
      const path=q.path?q.path+'/'+entry.name:entry.name;
      row.append(folderButton(entry.directory?ft('打开','Open'):ft('单文件下载','Download file'),async()=>{
        if(entry.directory)return folderBrowse(box,{...q,path,cursor:''});
        const result=await shareApi('shares/pull',{...q,path});toast(ft('已保存：','Saved: ')+result.file);
      }));box.append(row);
    }
    if(!v.entries.length)box.append(document.createTextNode(ft('空目录','Empty folder')));
    if(v.next)box.append(folderButton(ft('下一页','Next page'),()=>folderBrowse(box,{...q,cursor:v.next})));
  } catch(e){ if(box.folderGeneration===generation)box.textContent=e.message; }
}
async function pollFolderTasks(){
  if (folderPolling) return;
  folderPolling = true;
  try {
    if (typeof sel === 'undefined' || !sel) return;
    const peer = sel;
    if(folderPeer!==peer){folderTasks.clear();folderPeer=peer;}
    const v = await shareApi('transfers?peer='+encodeURIComponent(peer));
    if(peer!==sel)return;
    for(const task of v.tasks)folderTasks.set(String(task.message_id),task);
    document.querySelectorAll('.transfer-card[data-message]').forEach(card=>{
      const task=folderTasks.get(card.dataset.message);if(task)updateFolderCard(card,task);
    });
  } catch(e){ /* State is durable; retry without changing the card on transient failures. */ }
  finally { folderPolling=false;setTimeout(pollFolderTasks,1000); }
}
setTimeout(pollFolderTasks,1000);
