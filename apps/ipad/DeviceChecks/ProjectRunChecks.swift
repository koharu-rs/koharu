import XCTest
import UIKit
@testable import KoharuPad

final class ProjectRunChecks: XCTestCase {
    @MainActor func testEntireProjectAdvancesAcrossFluxPages() async throws {
        let installed = try LibraryDisk()
        let settings = try installed.load(Preferences.self, installed.root.appendingPathComponent("preferences.json"))
        guard settings.provider == .lmStudio else { throw XCTSkip("This check requires the configured LAN LM Studio provider") }
        let flux = try VisionModel.named("flux")
        guard flux.ready(installed.modelsURL) else { throw XCTSkip("Download FLUX before this optional device check") }
        try flux.checkMemory()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ProjectRun-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let disk = try LibraryDisk(root: root)
        // Reuse immutable installed weights without altering the reader's
        // preferences, chapter manifests or downloaded models.
        try FileManager.default.createSymbolicLink(at: disk.modelsURL.appendingPathComponent("vision"), withDestinationURL: installed.modelsURL.appendingPathComponent("vision"))
        let library = MangaLibrary(disk: disk)
        library.preferences = settings
        library.preferences.ocrModel = nil
        library.preferences.inpaint = true
        library.preferences.cleanupModel = "flux"
        var chapter = Chapter(title: "Project run device check", sourceURL: "https://rawkuma.net/", target: settings.target, pages: [])
        try FileManager.default.createDirectory(at: disk.chapterURL(chapter.id), withIntermediateDirectories: true)
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        for (index, text) in ["こんにちは", "ありがとう"].enumerated() {
            let image = UIGraphicsImageRenderer(size: CGSize(width: 512, height: 512), format: format).image { context in
                UIColor.white.setFill(); context.fill(CGRect(x: 0, y: 0, width: 512, height: 512))
                (text as NSString).draw(at: CGPoint(x: 100, y: 220), withAttributes: [.font: UIFont.systemFont(ofSize: 40), .foregroundColor: UIColor.black])
            }
            let filename = "fixture-\(index).png"
            try XCTUnwrap(image.pngData()).write(to: disk.pageURL(chapter.id, filename))
            chapter.pages.append(MangaPage(source: "https://cdn.rawkuma.com/check/\(index).jpg", original: filename))
        }
        try library.update(chapter)
        let started = Date()
        library.run(chapter.id, force: true)
        try await XCTUnwrap(library.activeTask).value
        XCTAssertNil(library.error, library.error ?? "")
        XCTAssertFalse(library.busy)
        let completed = try XCTUnwrap(library.chapters.first)
        XCTAssertEqual(completed.pages.count, 2)
        for page in completed.pages {
            XCTAssertFalse(page.segments.isEmpty, "The fixture must reach actual OCR and LM Studio")
            XCTAssertTrue(page.segments.allSatisfy { $0.text != nil })
            let rendered = try XCTUnwrap(page.rendered)
            XCTAssertTrue(FileManager.default.fileExists(atPath: disk.pageURL(chapter.id, rendered).path))
        }
        XCTAssertEqual(try disk.chapters().first?.pages.filter { $0.rendered != nil }.count, 2)
        print("PROJECT_RUN completed 2/2 pages with LM Studio + FLUX; elapsed \(Date().timeIntervalSince(started)) s")
    }
    @MainActor func testFluxCompletesMultipleTilesOnSamePage() async throws {
        let idleTimer = UIApplication.shared.isIdleTimerDisabled
        UIApplication.shared.isIdleTimerDisabled = true
        defer { UIApplication.shared.isIdleTimerDisabled = idleTimer }
        let disk = try LibraryDisk()
        guard try VisionModel.named("flux").ready(disk.modelsURL) else { throw XCTSkip("Download FLUX before this optional check") }
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".png")
        defer { try? FileManager.default.removeItem(at: file) }
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        let image = UIGraphicsImageRenderer(size: CGSize(width: 512, height: 896), format: format).image { context in
            UIColor.white.setFill(); context.fill(CGRect(x: 0, y: 0, width: 512, height: 896))
            for y in [100, 720] { ("こんにちは" as NSString).draw(at: CGPoint(x: 100, y: y), withAttributes: [.font: UIFont.systemFont(ofSize: 32), .foregroundColor: UIColor.black]) }
        }
        try XCTUnwrap(image.pngData()).write(to: file)
        let segments = [100, 720].enumerated().map { index, y in
            TextSegment(id: index, source: "こんにちは", text: "Hello", rect: CGRect(x: 100.0/512, y: Double(y)/896, width: 200.0/512, height: 40.0/896))
        }
        let recorder = CleanupRecorder()
        let started = Date()
        let data = try await PagePipeline().render(file, segments: segments, inpaint: true, modelID: "flux", modelRoot: disk.modelsURL) { recorder.record($0) }
        XCTAssertEqual(UIImage(data: data)?.size, image.size)
        XCTAssertEqual(recorder.counts(), [0, 1, 2])
        print("MULTITILE completed 2/2 FLUX tiles in one context; elapsed \(Date().timeIntervalSince(started)) s")
    }

}

private final class CleanupRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var completed: [Int] = []
    func record(_ progress: CleanupProgress) {
        if case .tiles(let count, _) = progress { lock.withLock { completed.append(count) } }
    }
    func counts() -> [Int] { lock.withLock { completed } }
}
