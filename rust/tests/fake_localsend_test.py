#!/usr/bin/env python3
"""端到端验证：Python 伪装 LocalSend v2 设备 ⇄ antify-rs 节点互操作
用法：python3 fake_localsend_test.py <节点面板HTTP> <节点HTTPS端口> [下载目录]
依赖节点：Tauri GUI（或 antify-rs serve --ui-port …）已在运行
"""
import hashlib, json, os, random, socket, ssl, subprocess, sys, threading, time
import urllib.request, urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

NODE_UI   = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:53318"
NODE_HTTPS_PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 53317
RX_DIR    = sys.argv[3] if len(sys.argv) > 3 else None  # None = 从节点 state 的 me.dir 读
FAKE_PORT = 53390
FAKE_FP   = "DEADBEEFCAFEF00D" + "A" * 48   # 伪指纹
FAKE_ALIAS = "测试机 📱"

results = []
def check(name, ok, detail=""):
    results.append((name, ok, detail))
    print(f"{'✅' if ok else '❌'} {name}" + (f"  — {detail}" if detail else ""))

# ---------- 0. 多播监听+公告（线程在服务器就绪后启动，见下） ----------
def announcer():
    # 绑多播端口（与节点 REUSEPORT 共存）+ 加入组；同机时收不到节点公告多半是
    # REUSEPORT 分发给了先绑定者，因此这里也作 120s 周期的兜底监听
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try: s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    except OSError: pass
    s.bind(("0.0.0.0", 53317))
    mreq = socket.inet_aton("224.0.0.167") + socket.inet_aton("0.0.0.0")
    s.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP, mreq)
    try: s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
    except OSError: pass
    s.settimeout(0.3)
    msg = json.dumps({**fake_info(with_port=True), "announce": True}).encode()
    group = ("224.0.0.167", 53317)
    while True:
        try:
            s.sendto(msg, group)
            while True:
                data, _ = s.recvfrom(65536)
                try:
                    m = json.loads(data)
                    fp = m.get("fingerprint")
                    if fp and fp != FAKE_FP:
                        fake["heard_fps"][fp[:16]] = fake["heard_fps"].get(fp[:16], 0) + 1
                        fake["got_multicast_from_node"] = True
                except ValueError: pass
        except socket.timeout:
            pass
        except OSError:
            pass
        time.sleep(1.0)

# ---------- 1. 自签证书 ----------
os.makedirs("/tmp/fakels", exist_ok=True)
subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                "-keyout", "/tmp/fakels/key.pem", "-out", "/tmp/fakels/cert.pem",
                "-days", "2", "-subj", "/CN=fakels"], check=True, capture_output=True)

# ---------- 2. 伪装设备状态 ----------
fake = {
    "registered_by_node": False,   # 节点听到我们的公告后回 register
    "prepared": [],                # 收到的 prepare-upload 请求
    "uploads": {},                 # fileId -> bytes
    "sessions": {},                # sessionId -> {fileId: token}
    "cancelled": [],
    "got_multicast_from_node": False,  # 我们听到节点的公告
    "heard_fps": {},               # 诊断：听到过的所有指纹 → 次数
}

def fake_info(with_port=False):
    d = {"alias": FAKE_ALIAS, "version": "2.1", "deviceModel": "iPhone 15 (fake)",
         "deviceType": "mobile", "fingerprint": FAKE_FP, "download": False}
    if with_port:
        d["port"] = FAKE_PORT; d["protocol"] = "https"
    return d

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _json(self, obj, code=200):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        if self.path.startswith("/api/localsend/v2/info"):
            self._json(fake_info())
        else:
            self.send_response(404); self.end_headers()
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        p = self.path
        if p == "/api/localsend/v2/register":
            fake["registered_by_node"] = True
            self._json(fake_info())
        elif p == "/api/localsend/v2/prepare-upload":
            req = json.loads(body)
            fake["prepared"].append(req)
            sid = f"fake-session-{len(fake['prepared'])}"
            fake["sessions"][sid] = {k: f"tok-{k}" for k in req["files"]}
            self._json({"sessionId": sid, "files": {k: f"tok-{k}" for k in req["files"]}})
        elif p.startswith("/api/localsend/v2/upload"):
            from urllib.parse import urlparse, parse_qs
            q = parse_qs(urlparse(p).query)
            sid, fid, tok = q["sessionId"][0], q["fileId"][0], q["token"][0]
            if fake["sessions"].get(sid, {}).get(fid) != tok:
                self.send_response(403); self.end_headers(); return
            fake["uploads"][fid] = body
            self.send_response(200); self.end_headers()
        elif p.startswith("/api/localsend/v2/cancel"):
            fake["cancelled"].append(self.path)
            self.send_response(200); self.end_headers()
        else:
            self.send_response(404); self.end_headers()

srv = ThreadingHTTPServer(("0.0.0.0", FAKE_PORT), H)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain("/tmp/fakels/cert.pem", "/tmp/fakels/key.pem")
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
threading.Thread(target=srv.serve_forever, daemon=True).start()
threading.Thread(target=announcer, daemon=True).start()

# ---------- 工具 ----------
def get(url, timeout=5):
    return urllib.request.urlopen(url, timeout=timeout, context=ssl._create_unverified_context() if url.startswith("https") else None)
def post(url, data=None, js=None, timeout=8):
    if js is not None:
        data = json.dumps(js).encode()
    return urllib.request.urlopen(urllib.request.Request(url, data=data, method="POST",
        headers={"Content-Type": "application/json"}), timeout=timeout,
        context=ssl._create_unverified_context() if url.startswith("https") else None)

def node_state():
    return json.loads(get(f"{NODE_UI}/api/ui/state").read())

RX_DIR = RX_DIR or node_state()["me"]["dir"]

# 幂等：清掉本测试历次运行残留的产物，否则防覆盖改名会让断言路径漂移（(1)/(2)/…）
import glob
for stale in glob.glob(os.path.join(RX_DIR, "e2e-测试*.bin")) + \
             glob.glob(os.path.join(RX_DIR, "sha-bad.bin")):
    try:
        os.remove(stale)
    except OSError:
        pass

def prepare_ok(prep, name="prepare-upload"):
    """免疫残留会话：409 时先 cancel 再重试一次"""
    try:
        return post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/prepare-upload", js=prep)
    except urllib.error.HTTPError as e:
        if e.code == 409:
            post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId=stale")
            time.sleep(0.3)
            return post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/prepare-upload", js=prep)
        raise

# ---------- T1: 双向发现 ----------
print("—— T1 发现 ——")
deadline = time.time() + 8
discovered = "multicast"
while time.time() < deadline:
    st = node_state()
    if any(d["fingerprint"] == FAKE_FP for d in st["devices"]): break
    time.sleep(0.5)
else:
    # 多播可能被防火墙拦（未签名 debug 二进制被拒）——退化为直接 register
    discovered = "direct-register"
    post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/register", js={**fake_info(with_port=True)})
    time.sleep(1)
    st = node_state()
check("节点设备表出现伪设备", any(d["fingerprint"] == FAKE_FP for d in st["devices"]),
      f"路径={discovered}")
check("伪设备收到节点的 register 回礼", fake["registered_by_node"])
# 回敬公告有 150ms 延迟 + 3s 限流，这里给足窗口
_t0 = time.time()
while not fake["got_multicast_from_node"] and time.time() - _t0 < 6:
    time.sleep(0.4)
check("伪设备听到节点的多播公告", fake["got_multicast_from_node"],
      f"heard={fake['heard_fps']}")

# ---------- T2: 节点 → 伪设备（面板发文本） ----------
print("—— T2 发送 ——")
sent_text = f"你好 LocalSend！from antify-rs {random.randint(0,999)}"
r = post(f"{NODE_UI}/api/ui/send-text", js={"target": FAKE_FP, "text": sent_text})
ok = r.status == 200
time.sleep(0.3)
prep = fake["prepared"][-1] if fake["prepared"] else {}
f0 = prep.get("files", {}).get("f0", {})
got = fake["uploads"].get("f0", b"")
check("面板发文本 → 伪设备收到 prepare-upload（官方格式 <uuid>.txt + preview 内嵌正文）",
      ok and len(f0.get("fileName", "")) == 40 and f0.get("fileName", "").endswith(".txt")
      and f0.get("size") == len(sent_text.encode()) and f0.get("preview") == sent_text)
check("伪设备收到上传内容一致", got.decode(errors="replace") == sent_text)
check("prepare-upload.info 带节点指纹与端口", prep.get("info", {}).get("fingerprint") not in ("", None)
      and prep.get("info", {}).get("port") == NODE_HTTPS_PORT)

# ---------- T3: 伪设备 → 节点（官方 App 语义发文件） ----------
print("—— T3 接收 ——")
payload = bytes(random.getrandbits(8) for _ in range(3 * 1024 * 1024 + 7))  # 3MB+7B
sha = hashlib.sha256(payload).hexdigest()
prep = {"info": {**fake_info(with_port=True)},
        "files": {"x1": {"id": "x1", "fileName": "e2e-测试.bin", "size": len(payload),
                          "fileType": "application/octet-stream", "sha256": sha}}}
r = prepare_ok(prep)
resp = json.loads(r.read())
sid, tok = resp["sessionId"], resp["files"]["x1"]
check("prepare-upload 返回 sessionId+token(字符串)", isinstance(sid, str) and isinstance(tok, str) and tok)
req = urllib.request.Request(
    f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/upload?sessionId={sid}&fileId=x1&token={tok}",
    data=payload, method="POST", headers={"Content-Type": "application/octet-stream"})
r2 = urllib.request.urlopen(req, context=ssl._create_unverified_context(), timeout=30)
check("upload 3MB 返回 200", r2.status == 200)
rx = os.path.join(RX_DIR, "e2e-测试.bin")
check("文件落盘且内容一致", os.path.exists(rx) and open(rx, "rb").read() == payload,
      f"{rx} {os.path.getsize(rx) if os.path.exists(rx) else -1}B")
st = node_state()
check("面板已接收列表出现该文件", any(x["name"] == "e2e-测试.bin" for x in st["received"]))

# ---------- T4: 错误路径 ----------
print("—— T4 错误路径 ——")
def expect_status(fn, want, name):
    try:
        r = fn(); got = r.status
    except urllib.error.HTTPError as e:
        got = e.code
    except Exception as e:
        got = str(e)
    check(name, got == want, f"得到 {got}")
    return got

# 4a 错 token → 403
prep2 = {"info": {**fake_info(with_port=True)},
         "files": {"x1": {"id": "x1", "fileName": "bad.bin", "size": 3}}}
r = prepare_ok(prep2)
resp2 = json.loads(r.read())
bad = urllib.request.Request(
    f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/upload?sessionId={resp2['sessionId']}&fileId=x1&token=WRONG",
    data=b"abc", method="POST")
expect_status(lambda: urllib.request.urlopen(bad, context=ssl._create_unverified_context()), 403,
              "错误 token → 403")
# 官方语义：403 后发送方应取消会话，释放接收端
post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId={resp2['sessionId']}")
# 4b 错 sha256 → 422
prep3 = {"info": {**fake_info(with_port=True)},
         "files": {"x1": {"id": "x1", "fileName": "sha-bad.bin", "size": 3,
                          "sha256": "00" * 32}}}
r = prepare_ok(prep3)
resp3 = json.loads(r.read())
badsha = urllib.request.Request(
    f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/upload?sessionId={resp3['sessionId']}&fileId=x1&token={resp3['files']['x1']}",
    data=b"abc", method="POST")
expect_status(lambda: urllib.request.urlopen(badsha, context=ssl._create_unverified_context()), 422,
              "SHA-256 不符 → 422")
# 4c 同 session 不放人（resp3 的会话还挂着，因为文件没收成——attempts<3 未标 done？此时文件失败即 422，会话保留）
prep4 = {"info": {**fake_info(with_port=True)},
         "files": {"x1": {"id": "x1", "fileName": "busy.bin", "size": 3}}}
expect_status(lambda: post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/prepare-upload", js=prep4),
              409, "会话占用时第二个 prepare → 409")
# 4d 取消会话 → 200，然后新 prepare 可用
expect_status(lambda: post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId={resp3['sessionId']}"),
              200, "cancel → 200")
r = post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/prepare-upload", js=prep4)
check("取消后可重新 prepare", r.status == 200)
post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId={json.loads(r.read())['sessionId']}")

# ---------- T5: 重名落盘不覆盖 ----------
print("—— T5 重名 ——")
for i in range(2):
    prep5 = {"info": {**fake_info(with_port=True)},
             "files": {"x1": {"id": "x1", "fileName": "e2e-测试.bin", "size": len(b"duplicate"),
                              "sha256": hashlib.sha256(b"duplicate").hexdigest()}}}
    r = prepare_ok(prep5)
    resp5 = json.loads(r.read())
    urllib.request.urlopen(urllib.request.Request(
        f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/upload?sessionId={resp5['sessionId']}&fileId=x1&token={resp5['files']['x1']}",
        data=b"duplicate", method="POST"), context=ssl._create_unverified_context())
    time.sleep(0.2)
check("同名文件自动改名（不覆盖）",
      os.path.exists(os.path.join(RX_DIR, "e2e-测试 (1).bin"))
      and open(os.path.join(RX_DIR, "e2e-测试.bin"), "rb").read() == payload)

# ---------- 汇总 ----------
fails = [n for n, ok, _ in results if not ok]
print(f"\n═══ {len(results) - len(fails)}/{len(results)} 通过 ═══")
if fails:
    print("失败项：", *fails, sep="\n  - ")
    sys.exit(1)
