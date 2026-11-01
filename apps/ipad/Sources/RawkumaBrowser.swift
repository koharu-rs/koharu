import SwiftUI
import WebKit
import Observation

@MainActor @Observable final class RawkumaBrowser: NSObject, WKNavigationDelegate, WKUIDelegate {
    let webView: WKWebView
    var address = "https://rawkuma.net/"
    var loading = false
    var chapter: SourceChapter?
    var error: String?
    private var inspection: Task<Void, Never>?
    override init() {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        if let file = Bundle.main.url(forResource: "rawkuma-navigation", withExtension: "js"), let script = try? String(contentsOf: file, encoding: .utf8) {
            configuration.userContentController.addUserScript(WKUserScript(source: script, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        }
        webView = WKWebView(frame: .zero, configuration: configuration)
        super.init()
        webView.navigationDelegate = self; webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
    }
    func go(_ address: String) {
        guard SourceChapter.isSource(address), let url = URL(string: address) else { error = L("Enter an HTTPS Rawkuma URL"); return }
        self.address = address; chapter = nil; error = nil
        inspection?.cancel()
        webView.load(URLRequest(url: url))
    }
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        if action.targetFrame?.isMainFrame == true || action.targetFrame == nil {
            decisionHandler(action.request.url.map { SourceChapter.isSource($0.absoluteString) } == true && action.targetFrame != nil ? .allow : .cancel)
        } else { decisionHandler(.allow) }
    }
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? { nil }
    func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) { loading = true; chapter = nil; error = nil; inspection?.cancel() }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        loading = false
        if let url = webView.url { address = url.absoluteString }
        inspection?.cancel()
        inspection = Task {
            for _ in 0..<15 {
                if Task.isCancelled { return }
                await inspect()
                if chapter != nil { return }
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { navigationFailed(error) }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { navigationFailed(error) }
    private func navigationFailed(_ error: Error) {
        // Replacing an in-flight navigation is normal when choosing another chapter.
        guard (error as NSError).code != NSURLErrorCancelled || (error as NSError).domain != NSURLErrorDomain else { return }
        loading = false; self.error = error.localizedDescription
    }
    func inspect() async {
        guard let url = webView.url, SourceChapter.isSource(url.absoluteString), !webView.isLoading else { return }
        do {
            let value = try await webView.evaluateJavaScript(#"""
            (() => {
              const reader = [...document.querySelectorAll('section[data-image-data] img')];
              const nodes = reader.length ? reader : [...document.querySelectorAll('img[alt*=" - Page "]')];
              const images = [];
              for (const image of nodes) {
                for (const source of [image.getAttribute('data-src'), image.getAttribute('data-original'), image.getAttribute('src'), image.currentSrc]) {
                  if (!source) continue;
                  try {
                    const url = new URL(source, location.href);
                    if (url.protocol === 'https:' && url.hostname === 'cdn.rawkuma.com' && !url.username && !url.password && !url.port) {
                      if (!images.includes(url.href)) images.push(url.href);
                      break;
                    }
                  } catch {}
                }
              }
              return JSON.stringify({url: location.href, title: document.title, images});
            })()
            """#)
            guard webView.url == url, let string = value as? String else { return }
            let candidate = try JSONDecoder().decode(SourceChapter.self, from: Data(string.utf8)).validated()
            guard candidate.url == url.absoluteString else { return }
            chapter = candidate; error = nil
        } catch { chapter = nil }
    }
}

struct BrowserSurface: UIViewRepresentable {
    let browser: RawkumaBrowser
    func makeUIView(context: Context) -> WKWebView { browser.webView }
    func updateUIView(_ uiView: WKWebView, context: Context) {}
}

struct BrowserScreen: View {
    @Environment(MangaLibrary.self) private var library
    @Bindable var browser: RawkumaBrowser
    @State private var revalidate = true
    @State private var newCopy = false
    var showLibrary: (UUID) -> Void
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button { browser.webView.goBack() } label: { Image(systemName: "chevron.backward") }.accessibilityLabel(L("Back"))
                Button { browser.webView.goForward() } label: { Image(systemName: "chevron.forward") }.accessibilityLabel(L("Forward"))
                Button { browser.webView.reload() } label: { Image(systemName: "arrow.clockwise") }.accessibilityLabel(L("Reload"))
                TextField(L("Rawkuma URL"), text: $browser.address).textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL).onSubmit { browser.go(browser.address) }.textFieldStyle(.roundedBorder).environment(\.layoutDirection, .leftToRight)
                Button(L("Go")) { browser.go(browser.address) }
            }.padding(10)
            HStack {
                Button {
                    guard let chapter = browser.chapter else { return }
                    library.importChapter(chapter, revalidate: revalidate, newCopy: newCopy, onReady: showLibrary)
                } label: { Label(L("Translate this chapter"), systemImage: "translate") }
                .buttonStyle(.borderedProminent).disabled(library.busy || browser.chapter == nil || browser.loading)
                Toggle(L("Check source updates"), isOn: $revalidate).toggleStyle(.button)
                Toggle(L("New copy"), isOn: $newCopy).toggleStyle(.button)
                Spacer()
                if let chapter = browser.chapter { Text("\(chapter.images.count) \(L("pages"))").font(.caption).foregroundStyle(.secondary) }
                if browser.loading { ProgressView() }
            }.padding(.horizontal, 10).padding(.bottom, 8)
            if let error = browser.error { Text(error).font(.caption).foregroundStyle(.red).padding(6) }
            BrowserSurface(browser: browser)
        }
        .navigationTitle("Rawkuma")
        .task { if browser.webView.url == nil { browser.go(browser.address) } }
    }
}
