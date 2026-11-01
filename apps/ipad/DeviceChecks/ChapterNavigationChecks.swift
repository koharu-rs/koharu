import XCTest
import UIKit
@testable import KoharuPad

final class ChapterNavigationChecks: XCTestCase {
    @MainActor func testChapterDeletionKeepsModelsSettingsAndUnselectedChapters() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let disk = try LibraryDisk(root: root), library = MangaLibrary(disk: disk)
        let settings = root.appendingPathComponent("preferences.json")
        try disk.save(library.preferences, to: settings)
        let settingsData = try Data(contentsOf: settings)
        let model = disk.modelsURL.appendingPathComponent("keep-model.gguf")
        try Data([1, 2, 3]).write(to: model)
        var ids: [UUID] = []
        for index in 1...3 {
            let chapter = Chapter(title: "Disposable chapter \(index)", sourceURL: "https://rawkuma.net/manga/check/\(index)/", target: "vi-VN", pages: [])
            try library.update(chapter)
            for name in ["original.image", "render.png", "undo.json"] {
                try Data([4, 5, 6]).write(to: disk.chapterURL(chapter.id).appendingPathComponent(name))
            }
            ids.append(chapter.id)
        }
        library.busy = true
        library.delete(ids)
        XCTAssertEqual(library.chapters.count, 3)
        library.busy = false
        library.delete([ids[0]])
        XCTAssertFalse(FileManager.default.fileExists(atPath: disk.chapterURL(ids[0]).path))
        XCTAssertEqual(Set(try disk.chapters().map(\.id)), Set(ids.dropFirst()))
        library.delete(library.chapters.map(\.id))
        XCTAssertTrue(library.chapters.isEmpty)
        XCTAssertTrue(try disk.chapters().isEmpty)
        for id in ids { XCTAssertFalse(FileManager.default.fileExists(atPath: disk.chapterURL(id).path)) }
        XCTAssertEqual(try Data(contentsOf: settings), settingsData)
        XCTAssertEqual(try Data(contentsOf: model), Data([1, 2, 3]))
        XCTAssertNil(library.error)
    }
    @MainActor func testBatchSizePersistsWithoutChangingProcessingPreferences() throws {
        let previous = UserDefaults.standard.object(forKey: "translationBatchSize")
        defer {
            if let previous { UserDefaults.standard.set(previous, forKey: "translationBatchSize") }
            else { UserDefaults.standard.removeObject(forKey: "translationBatchSize") }
        }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let disk = try LibraryDisk(root: root), library = MangaLibrary(disk: disk)
        let preferences = library.preferences
        library.translationBatchSize = 25
        XCTAssertEqual(library.preferences, preferences)
        XCTAssertEqual(MangaLibrary(disk: disk).translationBatchSize, 25)
    }
    @MainActor func testImportPublishesReadyChapterBeforeTranslation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let disk = try LibraryDisk(root: root), library = MangaLibrary(disk: disk)
        library.preferences.provider = .local // No model: fail translation after publishing the route.
        let image = UIGraphicsImageRenderer(size: CGSize(width: 100, height: 100)).image { context in
            UIColor.white.setFill(); context.fill(CGRect(x: 0, y: 0, width: 100, height: 100))
        }
        let old = Chapter(title: "Old chapter", sourceURL: "https://rawkuma.net/manga/check/chapter-1/", target: library.preferences.target, pages: [])
        try library.update(old)
        var chapter = Chapter(title: "New chapter", sourceURL: "https://rawkuma.net/manga/check/chapter-2/", target: library.preferences.target, pages: [MangaPage(source: "https://cdn.rawkuma.com/check/2/1.jpg", original: "source.png")])
        chapter.pages[0].segments = [TextSegment(id: 0, source: "こんにちは", rect: CGRect(x: 0.1, y: 0.1, width: 0.5, height: 0.5))]
        chapter.pages[0].ocrSignature = "vision-3:apple:document-2"
        try library.update(chapter)
        try XCTUnwrap(image.pngData()).write(to: disk.pageURL(chapter.id, "source.png"))
        var ready: [UUID] = []
        library.importChapter(SourceChapter(url: chapter.sourceURL, title: chapter.title, images: chapter.pages.map(\.source)), revalidate: false) { id in
            ready.append(id)
            XCTAssertTrue(library.busy)
            XCTAssertNil(library.error)
            XCTAssertEqual(library.chapters.first { $0.id == id }?.pages.count, 1)
            XCTAssertTrue(FileManager.default.fileExists(atPath: disk.pageURL(id, "source.png").path))
        }
        try await XCTUnwrap(library.activeTask).value
        XCTAssertEqual(ready, [chapter.id])
        XCTAssertNotNil(library.error, "The missing model must fail only after the new chapter is available to read")
        ready = []
        library.importChapter(SourceChapter(url: "https://example.invalid/", title: "Invalid", images: chapter.pages.map(\.source)), revalidate: false) { ready.append($0) }
        try await XCTUnwrap(library.activeTask).value
        XCTAssertTrue(ready.isEmpty, "A rejected import must not replace the reader route")
    }
}
