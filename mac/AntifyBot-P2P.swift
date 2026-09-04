// AntifyBot P2P · macOS 壳
// WKWebView 加载随包分发的 antify-p2p.html —— 零外部依赖，swiftc 直接编译。
// 壳层只做三件浏览器免费、WKWebView 不免费的事：
//   1) <input type=file> 打开 NSOpenPanel（选文件发送）
//   2) 收到的文件 blob:<a download> 转 WKDownload + NSSavePanel（保存落盘）
//   3) 标准窗口生命周期
//
// 自检：ANTIFY_SELFCHECK=1 运行可验证各 delegate 方法的 ObjC 选择器已挂上
//（@objc 可选协议方法签名不匹配不会报编译错，只能运行时验证）。
import Cocoa
import WebKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, WKUIDelegate, WKNavigationDelegate, WKDownloadDelegate {
    var window: NSWindow!
    var web: WKWebView!
    let autotest = ProcessInfo.processInfo.environment["ANTIFY_AUTOTEST"] == "1"   // 页面加载后自动点发起方，验证 WebRTC 全链路

    func applicationDidFinishLaunching(_ notification: Notification) {
        let frame = NSRect(x: 0, y: 0, width: 480, height: 840)

        web = WKWebView(frame: frame, configuration: WKWebViewConfiguration())
        web.autoresizingMask = [.width, .height]
        web.uiDelegate = self
        web.navigationDelegate = self

        if let url = Bundle.main.url(forResource: "antify-p2p", withExtension: "html") {
            web.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
        }

        window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered, defer: false)
        window.title = "AntifyBot P2P · 蚁间直传"
        window.minSize = NSSize(width: 400, height: 560)
        window.contentView = web
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    // ---- 自动测试：加载后点「生成配对码」，读回配对码与 ICE 状态写文件 ----
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard autotest else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
            webView.evaluateJavaScript("""
                document.getElementById('name').value = 'Mac 自检蚂蚁';
                document.getElementById('btn-offer').click();
                setTimeout(() => {
                    const pc = window.__antify.pc;
                    window.__antifyResult = {
                        code: document.getElementById('code-out').value,
                        cands: pc && pc.localDescription ? (pc.localDescription.sdp.match(/a=candidate/g) || []).length : -1,
                        ice: pc ? pc.iceGatheringState : 'no-pc',
                        log: window.__antify.log
                    };
                }, 5000);
            """, completionHandler: nil)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 6.6) {
            webView.evaluateJavaScript("JSON.stringify(window.__antifyResult || {error: 'no-result'})") { result, _ in
                let out = (result as? String) ?? "{\"error\":\"evaluate-failed\"}"
                try? out.write(toFile: "/tmp/antify-autotest.json", atomically: true, encoding: .utf8)
                exit(0)
            }
        }
    }

    // ---- ① 选择文件发送：<input type=file> 在 WKWebView 里不会自己弹面板 ----
    func webView(_ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters,
                 initiatedByFrame frame: WKFrameInfo,
                 completionHandler: @escaping @MainActor @Sendable ([URL]?) -> Void) {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        panel.message = "选择要发送的文件（可多选）"
        completionHandler(panel.runModal() == .OK ? panel.urls : nil)
    }

    // ---- ② 保存收到的文件：blob: 下载导航 → WKDownload → NSSavePanel ----
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void) {
        // 页面收完文件用 blob: + download 属性触发保存 —— 这类导航转成下载
        if navigationAction.request.url?.scheme == "blob" {
            decisionHandler(.download)
        } else {
            decisionHandler(.allow)
        }
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationResponse: WKNavigationResponse,
                 decisionHandler: @escaping @MainActor @Sendable (WKNavigationResponsePolicy) -> Void) {
        decisionHandler(navigationResponse.canShowMIMEType ? .allow : .download)
    }

    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) {
        download.delegate = self
    }

    // 本 SDK（macOS 26）的 WKDownloadDelegate 选择器已去掉 webView: 前缀，Swift 名为 download(_:…)
    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse,
                  suggestedFilename: String,
                  completionHandler: @escaping @MainActor @Sendable (URL?) -> Void) {
        let panel = NSSavePanel()
        panel.canCreateDirectories = true
        panel.nameFieldStringValue = suggestedFilename.isEmpty ? "antify-file" : suggestedFilename
        completionHandler(panel.runModal() == .OK ? panel.url : nil)
    }

    func downloadDidFinish(_ download: WKDownload) {}
    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {}
}

// ---- 引导（Swift 6：顶层代码非 MainActor，用 assumeIsolated 进入） ----
MainActor.assumeIsolated {
    if ProcessInfo.processInfo.environment["ANTIFY_SELFCHECK"] == "1" {
        let delegate = AppDelegate()
        let selectors = [
            "webView:decidePolicyForNavigationAction:decisionHandler:",
            "webView:decidePolicyForNavigationResponse:decisionHandler:",
            "webView:navigationResponse:didBecomeDownload:",
            "webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:",
            "download:decideDestinationUsingResponse:suggestedFilename:completionHandler:",
            "downloadDidFinish:",
            "download:didFailWithError:resumeData:",
        ]
        var all = true
        for s in selectors {
            let ok = delegate.responds(to: Selector(s))
            all = all && ok
            print((ok ? "✓ " : "✗ ") + s)
        }
        exit(all ? 0 : 1)
    }

    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.setActivationPolicy(.regular)
    app.run()
}
