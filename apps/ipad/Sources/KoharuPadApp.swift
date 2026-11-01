import SwiftUI
import UIKit

@main struct KoharuPadApp: App {
    @State private var library: MangaLibrary?
    private let startupError: String?
    init() {
        do { _library = State(initialValue: MangaLibrary(disk: try LibraryDisk())); startupError = nil }
        catch { _library = State(initialValue: nil); startupError = error.localizedDescription }
    }
    var body: some Scene {
        WindowGroup {
            if let library { RootScreen().environment(library).tint(.pink) }
            else { ContentUnavailableView(L("Could not open library"), systemImage: "exclamationmark.triangle", description: Text(startupError ?? "")) }
        }
    }
}

enum MainTab: Hashable { case library, browser, settings }
struct RootScreen: View {
    @Environment(MangaLibrary.self) private var library
    @State private var browser = RawkumaBrowser()
    @State private var selection = MainTab.library
    var body: some View {
        @Bindable var library = library
        TabView(selection: $selection) {
            Tab(L("Library"), systemImage: "books.vertical", value: .library) {
                NavigationStack { LibraryScreen { url in browser.go(url); selection = .browser } browse: { selection = .browser } }
            }
            Tab("Rawkuma", systemImage: "globe", value: .browser) {
                NavigationStack { BrowserScreen(browser: browser) { selection = .library } }
            }
            Tab(L("Settings"), systemImage: "gear", value: .settings) { NavigationStack { SettingsScreen() } }
        }
        .tabViewStyle(.sidebarAdaptable)
        .safeAreaInset(edge: .bottom) {
            if library.busy {
                HStack {
                    ProgressView(value: library.progress).frame(maxWidth: 180)
                    Text(library.status).font(.caption)
                    Button(L("Stop"), role: .destructive) { library.activeTask?.cancel() }.buttonStyle(.bordered)
                }.padding(10).frame(maxWidth: .infinity).background(.regularMaterial)
            }
        }
        .alert(L("Koharu"), isPresented: Binding(get: { library.error != nil }, set: { if !$0 { library.error = nil } })) {
            Button(L("OK")) { library.error = nil }
        } message: { Text(library.error ?? "") }
        .onChange(of: library.preferences) { _, _ in library.savePreferences() }
        .onReceive(NotificationCenter.default.publisher(for: UIApplication.didReceiveMemoryWarningNotification)) { _ in
            if !library.busy { Task { await library.translator.unload() } }
        }
    }
}

struct LibraryScreen: View {
    @Environment(MangaLibrary.self) private var library
    var openSource: (String) -> Void
    var browse: () -> Void
    @State private var deleting: Chapter?
    var body: some View {
        List {
            if library.chapters.isEmpty {
                ContentUnavailableView {
                    Label(L("Your manga library"), systemImage: "books.vertical")
                } description: { Text(L("Browse Rawkuma and translate a chapter. Saved chapters stay on this device.")) } actions: { Button(L("Browse Rawkuma"), action: browse).buttonStyle(.borderedProminent) }
            }
            ForEach(library.chapters) { chapter in
                NavigationLink {
                    ReaderScreen(chapterID: chapter.id, openSource: openSource)
                } label: {
                    VStack(alignment: .leading, spacing: 7) {
                        Text(chapter.title).font(.headline).lineLimit(2)
                        Text("\(chapter.pages.count) \(L("pages")) · \(Locale.current.localizedString(forIdentifier: chapter.target) ?? chapter.target)").font(.caption).foregroundStyle(.secondary)
                        ProgressView(value: Double(chapter.pages.filter { $0.rendered != nil }.count), total: Double(max(1, chapter.pages.count))).tint(.pink)
                    }.padding(.vertical, 8)
                }
                .swipeActions { Button(L("Delete"), role: .destructive) { deleting = chapter }.disabled(library.busy) }
                .contextMenu {
                    Button(L("Original chapter"), systemImage: "globe") { openSource(chapter.sourceURL) }
                    Button(L("Retranslate"), systemImage: "arrow.clockwise") { library.run(chapter.id, force: true) }.disabled(library.busy)
                }
            }
        }
        .navigationTitle(L("Library"))
        .toolbar { Button(L("Browse Rawkuma"), systemImage: "globe", action: browse) }
        .confirmationDialog(L("Delete this saved chapter?"), isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible) {
            Button(L("Delete"), role: .destructive) { if let deleting { library.delete(deleting.id) }; deleting = nil }
        }
    }
}

struct ReaderScreen: View {
    @Environment(MangaLibrary.self) private var library
    let chapterID: UUID
    var openSource: (String) -> Void
    @State private var index = 0
    @State private var original = false
    @State private var entireProject = true
    @State private var editing = false
    @State private var clear = false
    @State private var share: URL?
    var chapter: Chapter? { library.chapters.first { $0.id == chapterID } }
    var body: some View {
        Group {
            if let chapter, !chapter.pages.isEmpty {
                let page = chapter.pages[min(index, chapter.pages.count - 1)]
                VStack(spacing: 0) {
                    HStack {
                        Button { index = max(0, index - 1) } label: { Image(systemName: "chevron.left") }.disabled(index == 0).accessibilityLabel(L("Previous page"))
                        Text("\(index + 1) / \(chapter.pages.count)").monospacedDigit()
                        Button { index = min(chapter.pages.count - 1, index + 1) } label: { Image(systemName: "chevron.right") }.disabled(index == chapter.pages.count - 1).accessibilityLabel(L("Next page"))
                        Slider(value: Binding(get: { Double(index) }, set: { index = Int($0.rounded()) }), in: 0...Double(max(1, chapter.pages.count - 1)), step: 1).disabled(chapter.pages.count == 1)
                        Toggle(L("Original"), isOn: $original).toggleStyle(.button)
                        Button(L("Edit text"), systemImage: "text.cursor") { editing = true }.disabled(library.busy || page.segments.isEmpty)
                    }.padding(10).background(.bar)
                    ZoomPage(url: pageURL(page)) { offset in
                        index = min(chapter.pages.count - 1, max(0, index + offset))
                    }.id(page.id.uuidString + (original ? "original" : page.rendered ?? "source"))
                }
                .sheet(isPresented: $editing) { TextEditorScreen(chapterID: chapterID, page: page) }
                .toolbar {
                    ToolbarItemGroup(placement: .primaryAction) {
                        Button(L("Original chapter"), systemImage: "globe") { openSource(chapter.sourceURL) }
                        Menu {
                            Toggle(L("Entire project"), isOn: $entireProject)
                            Button(L("Run"), systemImage: "play") { library.run(chapterID, force: false, page: entireProject ? nil : page.id) }.disabled(library.busy)
                            Button(L("Retranslate"), systemImage: "arrow.clockwise") { library.run(chapterID, force: true, page: entireProject ? nil : page.id) }.disabled(library.busy)
                            Button(L("Undo"), systemImage: "arrow.uturn.backward") { library.restore(chapterID, redo: false) }.disabled(library.busy || !FileManager.default.fileExists(atPath: library.disk.chapterURL(chapterID).appendingPathComponent("undo.json").path))
                            Button(L("Redo"), systemImage: "arrow.uturn.forward") { library.restore(chapterID, redo: true) }.disabled(library.busy || !FileManager.default.fileExists(atPath: library.disk.chapterURL(chapterID).appendingPathComponent("redo.json").path))
                            Button(L("Share page"), systemImage: "square.and.arrow.up") { share = pageURL(page) }
                            Button(L("Export PDF"), systemImage: "doc") { exportPDF(chapter) }.disabled(library.busy)
                            Button(L("Clear cache"), systemImage: "trash") { clear = true }.disabled(library.busy)
                        } label: { Image(systemName: "ellipsis.circle") }
                    }
                }
            } else { ContentUnavailableView(L("Chapter not found"), systemImage: "book.closed") }
        }
        .background(ReaderNavigationGuard().allowsHitTesting(false))
        .navigationTitle(chapter?.title ?? L("Reader"))
        .navigationBarTitleDisplayMode(.inline)
        .task { if let chapter { index = min(chapter.readPage, max(0, chapter.pages.count - 1)) } }
        .onChange(of: index) { _, value in
            guard var chapter else { return }; chapter.readPage = value
            do { try library.update(chapter, recordUndo: false) } catch { library.error = error.localizedDescription }
        }
        .sheet(item: Binding(get: { share.map { ShareDocument(url: $0) } }, set: { share = $0?.url })) { ActivityShare(items: [$0.url]) }
        .confirmationDialog(L("Clear rendered cache? Originals, OCR and edited text are kept."), isPresented: $clear, titleVisibility: .visible) {
            Button(L("Clear cache"), role: .destructive) { library.clearCache(chapterID) }
        }
    }
    private func pageURL(_ page: MangaPage) -> URL {
        if !original, let rendered = page.rendered {
            let url = library.disk.pageURL(chapterID, rendered)
            if FileManager.default.fileExists(atPath: url.path) { return url }
        }
        return library.disk.pageURL(chapterID, page.original)
    }
    private func exportPDF(_ chapter: Chapter) {
        let disk = library.disk
        Task {
            do {
                share = try await Task.detached {
                    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".pdf")
                    guard let context = CGContext(url as CFURL, mediaBox: nil, nil) else { throw AppError.message(L("Could not create PDF")) }
                    defer { context.closePDF() }
                    for page in chapter.pages {
                        try Task.checkCancellation()
                        try autoreleasepool {
                            let rendered = page.rendered.map { disk.pageURL(chapter.id, $0) }
                            let path = rendered.flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil } ?? disk.pageURL(chapter.id, page.original)
                            guard let image = UIImage(contentsOfFile: path.path)?.cgImage else { throw AppError.message(L("Could not decode source image")) }
                            var rect = CGRect(x: 0, y: 0, width: image.width, height: image.height)
                            let box = Data(bytes: &rect, count: MemoryLayout<CGRect>.size)
                            context.beginPDFPage([kCGPDFContextMediaBox as String: box] as CFDictionary)
                            context.draw(image, in: rect); context.endPDFPage()
                        }
                    }
                    return url
                }.value
            } catch { library.error = error.localizedDescription }
        }
    }
}

struct ReaderNavigationGuard: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> ReaderNavigationController { ReaderNavigationController() }
    func updateUIViewController(_ controller: ReaderNavigationController, context: Context) {}
    static func dismantleUIViewController(_ controller: ReaderNavigationController, coordinator: ()) { controller.restoreNavigationGestures() }
}

final class ReaderNavigationController: UIViewController {
    private var savedGestures: [(UIGestureRecognizer, Bool)] = []
    override func loadView() { view = UIView(); view.backgroundColor = .clear }
    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        reserveReaderGestures()
    }
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        reserveReaderGestures()
    }
    override func viewWillDisappear(_ animated: Bool) {
        restoreNavigationGestures()
        super.viewWillDisappear(animated)
    }
    private func reserveReaderGestures() {
        guard let navigationController else { return }
        if savedGestures.isEmpty {
            // iPadOS 26+ also recognizes back swipes across the content area.
            // The reader owns horizontal gestures until it leaves the screen.
            savedGestures = [navigationController.interactivePopGestureRecognizer,
                             navigationController.interactiveContentPopGestureRecognizer]
                .compactMap { $0 }.map { ($0, $0.isEnabled) }
        }
        for (gesture, _) in savedGestures { gesture.isEnabled = false }
    }
    func restoreNavigationGestures() {
        for (gesture, enabled) in savedGestures { gesture.isEnabled = enabled }
        savedGestures.removeAll()
    }
}

struct ShareDocument: Identifiable { let url: URL; var id: String { url.absoluteString } }
struct ActivityShare: UIViewControllerRepresentable {
    var items: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController { UIActivityViewController(activityItems: items, applicationActivities: nil) }
    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
struct ZoomPage: UIViewRepresentable {
    let url: URL
    var turnPage: (Int) -> Void
    func makeUIView(context: Context) -> PageScrollView { let view = PageScrollView(); view.turnPage = turnPage; view.show(url); return view }
    func updateUIView(_ view: PageScrollView, context: Context) { view.turnPage = turnPage; view.show(url) }
}
final class PageScrollView: UIScrollView, UIScrollViewDelegate, UIGestureRecognizerDelegate {
    var turnPage: ((Int) -> Void)?
    private let picture = UIImageView()
    private var url: URL?
    private var previousSize: CGSize = .zero
    override init(frame: CGRect) {
        super.init(frame: frame)
        delegate = self; maximumZoomScale = 6; backgroundColor = .black; addSubview(picture)
        for direction: UISwipeGestureRecognizer.Direction in [.left, .right] {
            let swipe = UISwipeGestureRecognizer(target: self, action: #selector(turnPageWithSwipe(_:)))
            swipe.direction = direction; swipe.delegate = self
            addGestureRecognizer(swipe)
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
    func show(_ url: URL) {
        guard self.url != url else { return }
        self.url = url; picture.image = UIImage(contentsOfFile: url.path)
        picture.frame = CGRect(origin: .zero, size: picture.image?.size ?? CGSize(width: 1, height: 1))
        previousSize = .zero; setNeedsLayout()
    }
    override func layoutSubviews() {
        super.layoutSubviews()
        guard let image = picture.image, bounds.width > 0, bounds.height > 0 else { return }
        if bounds.size != previousSize {
            minimumZoomScale = min(bounds.width / image.size.width, bounds.height / image.size.height)
            zoomScale = minimumZoomScale; previousSize = bounds.size
        }
        contentInset = UIEdgeInsets(top: max(0, (bounds.height - picture.frame.height) / 2), left: max(0, (bounds.width - picture.frame.width) / 2), bottom: 0, right: 0)
        contentSize = picture.frame.size
    }
    func viewForZooming(in scrollView: UIScrollView) -> UIView? { picture }
    override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard gestureRecognizer is UISwipeGestureRecognizer else { return super.gestureRecognizerShouldBegin(gestureRecognizer) }
        // A zoomed page owns horizontal drags for panning, including when a
        // second finger is just beginning a pinch.
        return zoomScale <= minimumZoomScale * 1.01 && !isZooming && (pinchGestureRecognizer?.numberOfTouches ?? 0) < 2
    }
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool {
        otherGestureRecognizer === panGestureRecognizer
    }
    @objc private func turnPageWithSwipe(_ swipe: UISwipeGestureRecognizer) {
        guard swipe.state == .ended, gestureRecognizerShouldBegin(swipe) else { return }
        turnPage?(swipe.direction == .left ? 1 : -1)
    }
}

struct TextEditorScreen: View {
    @Environment(MangaLibrary.self) private var library
    @Environment(\.dismiss) private var dismiss
    let chapterID: UUID
    let page: MangaPage
    @State private var texts: [Int: String] = [:]
    @State private var saving = false
    var body: some View {
        NavigationStack {
            List(page.segments) { segment in
                VStack(alignment: .leading) {
                    Text(segment.source).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                    TextField(L("Translation"), text: Binding(get: { texts[segment.id] ?? segment.text ?? "" }, set: { texts[segment.id] = $0 }), axis: .vertical)
                }.padding(.vertical, 5)
            }
            .navigationTitle(L("Edit text"))
            .toolbar {
                Button(L("Cancel")) { dismiss() }.disabled(saving)
                Button(L("Save")) {
                    saving = true
                    Task {
                        if await library.edit(chapterID, page: page.id, texts: texts) { dismiss() }
                        saving = false
                    }
                }.disabled(saving || library.busy)
            }
        }
    }
}
