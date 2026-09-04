/* HTTP API：分块上传（XHR 拿上传进度、单块失败自动重试）、JSON 便捷封装 */

async function json(url, opts = {}) {
  const res = await fetch(url, opts);
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.error || `HTTP ${res.status}`);
  return body;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * 分块上传一个文件。
 * @param {File} file
 * @param {{deviceId:string, name:string}} who  发送者身份（服务端据此标注消息来源）
 * @param {{onProgress?:(sent:number,total:number)=>void}} cb
 * @returns 完成接口结果 {fileId, url, messageId}
 */
export async function uploadFile(file, who, cb = {}) {
  const init = await json('/api/upload/init', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      name: file.name, size: file.size,
      mime: file.type || 'application/octet-stream',
      sender: who.name,
    }),
  });
  const { fileId, chunkSize } = init;

  let offset = 0;
  while (offset < file.size) {
    const end = Math.min(offset + chunkSize, file.size);
    const buf = await file.slice(offset, end).arrayBuffer();

    let lastErr = null;
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        await putChunk(fileId, offset, buf, (loaded) => cb.onProgress?.(offset + loaded, file.size));
        lastErr = null;
        break;
      } catch (err) {
        lastErr = err;
        await sleep(400 * (attempt + 1)); // 局域网偶发抖动：稍候重试同一块
      }
    }
    if (lastErr) throw lastErr;
    offset = end;
    cb.onProgress?.(offset, file.size);
  }

  if (file.size === 0) cb.onProgress?.(0, 0);

  return json(`/api/upload/complete/${encodeURIComponent(fileId)}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ sender: who.name, senderId: who.deviceId }),
  });
}

function putChunk(fileId, offset, buf, onProgress) {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('PUT', `/api/upload/chunk/${encodeURIComponent(fileId)}`);
    xhr.setRequestHeader('content-type', 'application/octet-stream');
    xhr.setRequestHeader('x-offset', String(offset));
    xhr.upload.onprogress = (e) => onProgress(e.loaded);
    xhr.onload = () => (xhr.status >= 200 && xhr.status < 300)
      ? resolve()
      : reject(new Error(`分块上传失败 (${xhr.status})`));
    xhr.onerror = () => reject(new Error('网络中断'));
    xhr.ontimeout = () => reject(new Error('超时'));
    xhr.timeout = 120_000;
    xhr.send(buf);
  });
}

export const getLobby = () => json('/api/lobby');
