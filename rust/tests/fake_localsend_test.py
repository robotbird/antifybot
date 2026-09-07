#!/usr/bin/env python3
"""端到端验证：Python 伪装 LocalSend v2 设备 ⇄ antify-rs 节点互操作
用法：python3 fake_localsend_test.py <节点面板HTTP> <节点HTTPS端口> [下载目录]
依赖节点：Tauri GUI（或 antify-rs serve --ui-port …）已在运行
可选环境变量：
  E2E_NO_MULTICAST=1  节点以 --no-multicast 启动时跳过多播互见断言
说明：T6 起为 AntifyBot 分块续传扩展（伪设备按需实现 resume-info）；
      大文件按默认 16MiB 块设计（>16MiB 即多块），ANTIFY_CHUNK=1M 亦可跑。
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
E2E_NO_MULTICAST = os.environ.get("E2E_NO_MULTICAST") == "1"
MIB = 1024 * 1024

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
    "uploads": {},                 # fileId -> bytes（整发，官方语义）
    "sessions": {},                # sessionId -> {fileId: {token,sha,size}}
    "cancelled": [],
    "got_multicast_from_node": False,  # 我们听到节点的公告
    "heard_fps": {},               # 诊断：听到过的所有指纹 → 次数
    # —— 分块续传扩展（T6 用）——
    "resume_mode": "official",     # "antify" = 实现 /api/antify/v1/resume-info
    "chunks": {},                  # sha256 -> {"data": bytearray, "size": int}（伪对端暂存）
    "chunk_posts": [],             # 每次 chunk POST：{offset,len,got,sha_ok,dropped?}
    "whole_uploads": {},           # sha256 -> bytes（无 offset 参数的整发）
    "shrink_after_probe_sha": None,  # resume-info 应答后把该 sha 暂存砍半（构造 409 失衡）
    "drop_once_at": None,          # 掐断一次该 offset 的块连接（构造块级断连重试）
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
        elif self.path.startswith("/api/antify/v1/resume-info"):
            # AntifyBot 扩展：报告该 sha256 已收到的字节数（official 模式 404 = 官方对端）
            if fake["resume_mode"] != "antify":
                self.send_response(404); self.end_headers(); return
            from urllib.parse import urlparse, parse_qs
            q = parse_qs(urlparse(self.path).query)
            m = fake["sessions"].get(q.get("sessionId", [""])[0], {}).get(q.get("fileId", [""])[0])
            if not m or m["token"] != q.get("token", [""])[0]:
                self.send_response(403); self.end_headers(); return
            if not m["sha"]:
                self._json({"offset": 0, "size": m["size"], "sha256": ""}); return
            rec = fake["chunks"].setdefault(m["sha"], {"data": bytearray(), "size": m["size"]})
            resp = {"offset": len(rec["data"]), "size": m["size"], "sha256": m["sha"]}
            # 构造 409 失衡：应答完（probe 拿到旧进度）再把暂存砍半，
            # 让随后到达的块撞上 offset 超前 → 对端应回 409 让发送端重对齐
            if fake["shrink_after_probe_sha"] == m["sha"] and len(rec["data"]) > 8 * MIB:
                del rec["data"][8 * MIB:]
                fake["shrink_after_probe_sha"] = None
            self._json(resp)
        else:
            self.send_response(404); self.end_headers()
    def do_POST(self):
        # reqwest 的流式 body 无 Content-Length（chunked TE），手动解码
        n = int(self.headers.get("Content-Length") or 0)
        te = (self.headers.get("Transfer-Encoding") or "").lower()
        if "chunked" in te:
            body = b""
            while True:
                size = int(self.rfile.readline(1024).split(b";")[0].strip(), 16)
                if size == 0:
                    self.rfile.readline(1024)  # 收尾 CRLF
                    break
                body += self.rfile.read(size)
                self.rfile.read(2)  # 块尾 CRLF
        elif n:
            body = self.rfile.read(n)
        else:
            body = b""
        p = self.path
        if p == "/api/localsend/v2/register":
            fake["registered_by_node"] = True
            self._json(fake_info())
        elif p == "/api/localsend/v2/prepare-upload":
            req = json.loads(body)
            fake["prepared"].append(req)
            sid = f"fake-session-{len(fake['prepared'])}"
            sess = {}
            for k, f in req["files"].items():
                sess[k] = {"token": f"tok-{k}", "sha": (f.get("sha256") or "").lower(),
                           "size": f.get("size", 0)}
            fake["sessions"][sid] = sess
            self._json({"sessionId": sid, "files": {k: v["token"] for k, v in sess.items()}})
        elif p.startswith("/api/localsend/v2/upload"):
            from urllib.parse import urlparse, parse_qs
            q = parse_qs(urlparse(p).query)
            sid, fid, tok = q["sessionId"][0], q["fileId"][0], q["token"][0]
            m = fake["sessions"].get(sid, {}).get(fid)
            if not isinstance(m, dict) or m["token"] != tok:
                self.send_response(403); self.end_headers(); return
            if "offset" in q and "len" in q:
                off, ln = int(q["offset"][0]), int(q["len"][0])
                rec = fake["chunks"].setdefault(m["sha"], {"data": bytearray(), "size": m["size"]})
                # 掐线注入：不回任何响应直接关连接（发送端应块级重试）
                if fake.get("drop_once_at") == off and not fake.get("dropped"):
                    fake["dropped"] = True
                    fake["chunk_posts"].append({"offset": off, "len": ln, "got": len(body),
                                                "sha_ok": False, "dropped": True})
                    self.close_connection = True
                    return
                if off > len(rec["data"]):
                    fake["chunk_posts"].append({"offset": off, "len": ln, "got": 0,
                                                "sha_ok": False, "rejected": True})
                    self._json({"error": "offset ahead", "offset": len(rec["data"])}, 409); return
                if off < len(rec["data"]):
                    del rec["data"][off:]
                rec["data"][off:off + len(body)] = body
                hdr = (self.headers.get("X-Chunk-SHA256") or "").lower()
                fake["chunk_posts"].append({"offset": off, "len": ln, "got": len(body),
                                            "sha_ok": hdr == hashlib.sha256(body).hexdigest()})
                self._json({"ok": True})
            else:
                if m["sha"]:
                    fake["whole_uploads"][m["sha"]] = body
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
             glob.glob(os.path.join(RX_DIR, "sha-bad.bin")) + \
             glob.glob(os.path.join(RX_DIR, "e2e-chunk*.bin")) + \
             glob.glob(os.path.join(RX_DIR, "e2e-badhash*.bin")):
    try:
        os.remove(stale)
    except OSError:
        pass

def prepare_ok(prep, name="prepare-upload"):
    """免疫残留会话：409 时先 cancel（不带 id，清掉上一轮测试的僵尸）再重试一次"""
    try:
        return post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/prepare-upload", js=prep)
    except urllib.error.HTTPError as e:
        if e.code == 409:
            post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel")
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
if E2E_NO_MULTICAST:
    check("多播互见断言（跳过：E2E_NO_MULTICAST）", True, "skipped")
else:
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

# ---------- T6: 节点 → 伪设备：分块发送 / 断点续传 / 409 自愈 / 断块重试 / 官方回退 ----------
print("—— T6 分块发送（节点 → 伪设备） ——")
fake["resume_mode"] = "antify"

def send_path_to_fake(path, timeout=180):
    """经面板 send-path（原生选择器同款路径）让节点发一个大文件给伪设备"""
    r = post(f"{NODE_UI}/api/ui/send-path", js={"target": FAKE_FP, "path": path}, timeout=timeout)
    v = json.loads(r.read().decode())
    return r.status == 200 and v.get("ok") is True, v.get("error", "")

def posts_since(n):
    return fake["chunk_posts"][n:]

def chunk_posts_for(sha):
    # 该 sha 的块区间是否无缝铺满 [0, size)（被 409 拒掉/被掐线的尝试不算数）
    rec = fake["chunks"][sha]
    pos = 0
    for p in sorted((x for x in fake["chunk_posts"]
                     if not (x.get("dropped") or x.get("rejected"))),
                    key=lambda x: x["offset"]):
        if p["offset"] != pos:
            return False
        pos += p["got"]
    return pos == rec["size"]

# 6a 首发分块收齐（>16MiB → 默认块大小下也 ≥2 块）
big1 = os.urandom(16 * MIB + 1234567)
sha1 = hashlib.sha256(big1).hexdigest()
open("/tmp/e2e-big1.bin", "wb").write(big1)
_n = len(fake["chunk_posts"])
ok, err = send_path_to_fake("/tmp/e2e-big1.bin")
posts1 = posts_since(_n)
check("大文件发送成功（send-path ok）", ok, err)
check("对端收到的是分块 POST（≥2 块，带 offset/len）",
      len(posts1) >= 2 and all("len" in p for p in posts1))
check("每块携带正确的 X-Chunk-SHA256",
      all(p["sha_ok"] for p in posts1 if not (p.get("dropped") or p.get("rejected"))))
check("块区间无缝铺满全文", chunk_posts_for(sha1))
check("伪设备拼装内容一致", bytes(fake["chunks"][sha1]["data"]) == big1)

# 6b 断点续传：伪设备暂存里已有前 16MiB（上一轮传了一半）→ 节点应从断点继续
big2 = os.urandom(16 * MIB + 2345678)
sha2 = hashlib.sha256(big2).hexdigest()
fake["chunks"][sha2] = {"data": bytearray(big2[:16 * MIB]), "size": len(big2)}
open("/tmp/e2e-big2.bin", "wb").write(big2)
_n = len(fake["chunk_posts"])
ok, err = send_path_to_fake("/tmp/e2e-big2.bin")
posts2 = posts_since(_n)
check("续传成功", ok, err)
check("首块从断点 16MiB 开始（不重传已收部分）",
      bool(posts2) and posts2[0]["offset"] == 16 * MIB)
check("续传后伪设备内容完整", bytes(fake["chunks"][sha2]["data"]) == big2)

# 6c 409 失衡自愈：probe 报 16MiB 后暂存被砍到 8MiB → 首块撞 409 → 重对齐到 8MiB 重发
big3 = os.urandom(16 * MIB + 3456789)
sha3 = hashlib.sha256(big3).hexdigest()
fake["chunks"][sha3] = {"data": bytearray(big3[:16 * MIB]), "size": len(big3)}
fake["shrink_after_probe_sha"] = sha3
open("/tmp/e2e-big3.bin", "wb").write(big3)
_n = len(fake["chunk_posts"])
ok, err = send_path_to_fake("/tmp/e2e-big3.bin")
posts3 = posts_since(_n)
check("409 失衡场景发送成功", ok, err)
check("首块撞 409 后按对端回报重对齐（offset 16MiB → 8MiB）",
      len(posts3) >= 2 and posts3[0]["offset"] == 16 * MIB and posts3[1]["offset"] == 8 * MIB,
      f"offsets={[p['offset'] for p in posts3[:4]]}")
check("重对齐后内容完整", bytes(fake["chunks"][sha3]["data"]) == big3)

# 6d 块级断连重试：首块连接被掐 → 发送端同块重试成功
big4 = os.urandom(16 * MIB + 456789)
sha4 = hashlib.sha256(big4).hexdigest()
fake["drop_once_at"] = 0
open("/tmp/e2e-big4.bin", "wb").write(big4)
_n = len(fake["chunk_posts"])
ok, err = send_path_to_fake("/tmp/e2e-big4.bin")
posts4 = posts_since(_n)
check("断块场景发送成功", ok, err)
check("断连的块被原样重试（同 offset 再发）",
      any(p.get("dropped") and p["offset"] == 0 for p in posts4)
      and sum(1 for p in posts4 if p["offset"] == 0 and not p.get("dropped")) >= 1)
check("断块重试后内容完整", bytes(fake["chunks"][sha4]["data"]) == big4)
fake["drop_once_at"] = None

# 6e 官方对端回退：resume-info 404 → 整文件单 POST（无 offset 参数）
fake["resume_mode"] = "official"
big5 = os.urandom(16 * MIB + 5678901)
sha5 = hashlib.sha256(big5).hexdigest()
open("/tmp/e2e-big5.bin", "wb").write(big5)
_n = len(fake["chunk_posts"])
ok, err = send_path_to_fake("/tmp/e2e-big5.bin")
check("无 resume-info（官方对端）时整发成功", ok, err)
check("走的是整文件单 POST（无分块）",
      len(posts_since(_n)) == 0 and fake["whole_uploads"].get(sha5) == big5)

# ---------- T7: 伪设备 → 节点：分块接收 / 幂等 / 跨会话续传 / 坏块熔断 ----------
print("—— T7 分块接收（伪设备 → 节点） ——")
CH = 512 * 1024
pl = os.urandom(3 * CH + 12345)   # 3 整块 + 一条尾巴
pl_sha = hashlib.sha256(pl).hexdigest()
staging = os.path.join(RX_DIR, ".antify-incoming", f"{pl_sha}-{len(pl)}")

def prepare_file(fid, name, size, sha, alias_ip_info=None):
    prep = {"info": {**fake_info(with_port=True)},
            "files": {fid: {"id": fid, "fileName": name, "size": size,
                            "fileType": "application/octet-stream", "sha256": sha}}}
    r = prepare_ok(prep)
    resp = json.loads(r.read())
    return resp["sessionId"], resp["files"][fid]

def node_chunk(sid, fid, tok, off, data, sha_hdr=None):
    """向节点 POST 一个分块；sha_hdr 显式给错可测坏块"""
    req = urllib.request.Request(
        f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/upload"
        f"?sessionId={sid}&fileId={fid}&token={tok}&offset={off}&len={len(data)}",
        data=data, method="POST",
        headers={"Content-Type": "application/octet-stream",
                 "X-Chunk-SHA256": sha_hdr or hashlib.sha256(data).hexdigest()})
    return urllib.request.urlopen(req, context=ssl._create_unverified_context(), timeout=30)

def node_resume(sid, fid, tok):
    r = get(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/antify/v1/resume-info"
            f"?sessionId={sid}&fileId={fid}&token={tok}")
    return json.loads(r.read())

# 参数校验（独立小会话，先于主会话跑完并取消）：缺 X-Chunk-SHA256 / offset 与 len 不成对 → 400
r = prepare_ok({"info": {**fake_info(with_port=True)},
                "files": {"y1": {"id": "y1", "fileName": "e2e-args.bin", "size": 4,
                                 "sha256": "ab" * 32}}})
respY = json.loads(r.read()); sidY, tokY = respY["sessionId"], respY["files"]["y1"]
expect_status(lambda: urllib.request.urlopen(urllib.request.Request(
    f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/upload?sessionId={sidY}&fileId=y1&token={tokY}&offset=0&len=4",
    data=b"abcd", method="POST", headers={"Content-Type": "application/octet-stream"}),
    context=ssl._create_unverified_context()), 400, "分块缺 X-Chunk-SHA256 → 400")
expect_status(lambda: urllib.request.urlopen(urllib.request.Request(
    f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/upload?sessionId={sidY}&fileId=y1&token={tokY}&offset=0",
    data=b"abcd", method="POST"), context=ssl._create_unverified_context()), 400,
    "offset 与 len 不成对 → 400")
post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId={sidY}")

sidA, tokA = prepare_file("x1", "e2e-chunk.bin", len(pl), pl_sha)
check("resume-info 初始 offset=0", node_resume(sidA, "x1", tokA)["offset"] == 0)

# 坏块 hash → 422；offset 超前 → 409 + 当前 offset
expect_status(lambda: node_chunk(sidA, "x1", tokA, 0, pl[:CH], sha_hdr="00" * 32), 422,
              "坏块 hash → 422")
expect_status(lambda: node_chunk(sidA, "x1", tokA, 2 * CH, pl[2 * CH:3 * CH]), 409,
              "offset 超前 → 409 并回报当前 offset")

# 好块 ×1 → resume-info 反映进度；幂等重发同块仍 200 且进度不回退
check("块 1 上传 200", node_chunk(sidA, "x1", tokA, 0, pl[:CH]).status == 200)
check("resume-info 反映已收", node_resume(sidA, "x1", tokA)["offset"] == CH)
check("幂等重发同块 → 200", node_chunk(sidA, "x1", tokA, 0, pl[:CH]).status == 200)
check("幂等重发后进度不变", node_resume(sidA, "x1", tokA)["offset"] == CH)
st = node_state()
check("面板会话 files 带进度 got", any(f.get("got") == CH for f in st["session"]["files"]))

# cancel 带 error sessionId → no-op（会话仍活）；带对 sessionId → 会话清空
post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId=NOT-MINE")
check("cancel 错误 sessionId → no-op（会话仍可用）", node_resume(sidA, "x1", tokA)["offset"] == CH)
post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId={sidA}")
expect_status(lambda: node_resume(sidA, "x1", tokA), 403, "会话取消后 resume-info → 403")

# 跨会话续传（模拟接收端重启后发送端重新协商）：新 prepare → 断点保留 → 收尾
sidB, tokB = prepare_file("x1", "e2e-chunk.bin", len(pl), pl_sha)
check("新会话断点保留（offset=CH）", node_resume(sidB, "x1", tokB)["offset"] == CH)
check("块 2 上传 200", node_chunk(sidB, "x1", tokB, CH, pl[CH:2 * CH]).status == 200)
check("尾块上传 200 并触发收尾", node_chunk(sidB, "x1", tokB, 2 * CH, pl[2 * CH:]).status == 200)
rx_chunk = os.path.join(RX_DIR, "e2e-chunk.bin")
check("分块收齐后文件落盘且内容一致",
      os.path.exists(rx_chunk) and open(rx_chunk, "rb").read() == pl)
check("暂存目录已清理", not os.path.exists(staging), staging)

# 连续 3 个坏块 → 熔断（文件置 done，之后好块也 403）
pb = os.urandom(CH + 99)
pb_sha = hashlib.sha256(pb).hexdigest()
sidC, tokC = prepare_file("x1", "e2e-badhash.bin", len(pb), pb_sha)
for _i in range(3):
    expect_status(lambda: node_chunk(sidC, "x1", tokC, 0, pb[:CH], sha_hdr="11" * 32), 422,
                  f"坏块 #{_i + 1} → 422")
expect_status(lambda: node_chunk(sidC, "x1", tokC, 0, pb[:CH]), 403,
              "连续 3 坏块熔断后好块 → 403")
post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/cancel?sessionId={sidC}")

# ---------- T8: 接收取消 / 拒收窗口 / 取消发送标志 ----------
print("—— T8 取消 ——")
expect_status(lambda: post(f"{NODE_UI}/api/ui/cancel-rx"), 404, "无会话时 cancel-rx → 404")
sidD, tokD = prepare_file("x1", "e2e-declined.bin", 10, hashlib.sha256(b"0123456789").hexdigest())
check("cancel-rx 清会话 → 200", post(f"{NODE_UI}/api/ui/cancel-rx").status == 200)
expect_status(lambda: post(f"https://127.0.0.1:{NODE_HTTPS_PORT}/api/localsend/v2/prepare-upload",
             js={"info": {**fake_info(with_port=True)},
                 "files": {"x1": {"id": "x1", "fileName": "e2e-declined.bin", "size": 10}}}), 204,
              "拒收窗口内 prepare → 204（官方拒收语义）")
check("cancel-send 置标志 → 200", post(f"{NODE_UI}/api/ui/cancel-send").status == 200)
fails = [n for n, ok, _ in results if not ok]
print(f"\n═══ {len(results) - len(fails)}/{len(results)} 通过 ═══")
if fails:
    print("失败项：", *fails, sep="\n  - ")
    sys.exit(1)
