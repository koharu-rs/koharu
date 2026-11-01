import XCTest
import UIKit
@testable import KoharuPad

final class ContractTests: XCTestCase {
    func testRenderClearsEmptyTranslationAndPreservesArtworkOutsideBox() async throws {
        let source = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".png")
        defer { try? FileManager.default.removeItem(at: source) }
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        let image = UIGraphicsImageRenderer(size: CGSize(width: 100, height: 200), format: format).image { context in
            UIColor.black.setFill(); context.fill(CGRect(x: 0, y: 0, width: 100, height: 200))
        }
        try XCTUnwrap(image.pngData()).write(to: source)
        let result = try await PagePipeline().render(source, segments: [TextSegment(id: 0, source: "消す", text: "", rect: CGRect(x: 0.1, y: 0.1, width: 0.3, height: 0.2))], inpaint: false, modelRoot: source.deletingLastPathComponent())
        let cg = try XCTUnwrap(UIImage(data: result)?.cgImage)
        XCTAssertEqual(cg.width, 100); XCTAssertEqual(cg.height, 200)
        func red(_ x: Int, _ y: Int) throws -> UInt8 {
            let crop = try XCTUnwrap(cg.cropping(to: CGRect(x: x, y: y, width: 1, height: 1)))
            var pixel = [UInt8](repeating: 0, count: 4)
            try pixel.withUnsafeMutableBytes { bytes in
                let context = try XCTUnwrap(CGContext(data: bytes.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                context.draw(crop, in: CGRect(x: 0, y: 0, width: 1, height: 1))
            }
            return pixel[0]
        }
        XCTAssertGreaterThan(try red(20, 30), 245)
        XCTAssertLessThan(try red(80, 170), 10)
    }
    func testTranslationContractReordersIDsAndRejectsCorruption() throws {
        let result = try TranslationContract.decode(Data(#"{"translations":[{"id":1,"text":"hai"},{"id":0,"text":"một"}]}"#.utf8), count: 2)
        XCTAssertEqual(result, ["một", "hai"])
        for json in [
            #"{"translations":[{"id":0,"text":"a"},{"id":0,"text":"b"}]}"#,
            #"{"translations":[{"id":true,"text":"a"},{"id":1,"text":"b"}]}"#,
            #"{"translations":[{"id":0.5,"text":"a"},{"id":1,"text":"b"}]}"#,
            #"{"translations":[{"id":0,"text":"a"}]}"#,
            #"{"translations":[{"id":0,"text":"a","extra":1},{"id":1,"text":"b"}]}"#,
            #"<think>reason</think>{"translations":[]}"#
        ] { XCTAssertThrowsError(try TranslationContract.decode(Data(json.utf8), count: 2)) }
    }
    func testSourceBoundarySupportsFractionalChaptersAndRejectsOtherHosts() throws {
        let source = SourceChapter(url: "https://rawkuma.net/manga/example/chapter-38.5/", title: "Chapter", images: ["https://cdn.rawkuma.com/w/example/38.5/1.webp"])
        XCTAssertNoThrow(try source.validated())
        for url in ["http://rawkuma.net/", "https://rawkuma.net.evil.test/", "https://user@rawkuma.net/", "file:///private/test"] { XCTAssertFalse(SourceChapter.isSource(url)) }
        for url in ["http://cdn.rawkuma.com/a.jpg", "https://evil.test/a.jpg", "https://cdn.rawkuma.com:444/a.jpg"] { XCTAssertFalse(SourceChapter.isImage(url)) }
    }
    func testDiskReopenUndoAndClearCachePreserveOriginalAndManualText() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let disk = try LibraryDisk(root: root)
        var chapter = Chapter(title: "Test", sourceURL: "https://rawkuma.net/manga/example/chapter-38.5/", target: "vi-VN", pages: [MangaPage(source: "https://cdn.rawkuma.com/w/example/38.5/1.webp", original: "original.image")])
        try disk.save(chapter)
        try Data([1, 2, 3]).write(to: disk.pageURL(chapter.id, "original.image"))
        chapter.pages[0].segments = [TextSegment(id: 0, source: "こんにちは", text: "Xin chào", rect: CGRect(x: 0.1, y: 0.1, width: 0.3, height: 0.2), edited: true)]
        chapter.pages[0].rendered = "render.png"
        try Data([4]).write(to: disk.pageURL(chapter.id, "render.png"))
        try disk.save(chapter)
        let reopened = try LibraryDisk(root: root)
        XCTAssertEqual(try reopened.chapters().first?.sourceURL, chapter.sourceURL)
        let undone = try reopened.restore(chapter.id, redo: false)
        XCTAssertTrue(undone.pages[0].segments.isEmpty)
        XCTAssertEqual(try reopened.restore(chapter.id, redo: true).pages[0].segments.first?.text, "Xin chào")
        try reopened.clearDerived(chapter.id)
        let cleared = try reopened.chapters()[0]
        XCTAssertNil(cleared.pages[0].rendered)
        XCTAssertTrue(cleared.pages[0].segments[0].edited)
        XCTAssertEqual(cleared.pages[0].segments[0].text, "Xin chào")
        XCTAssertEqual(try Data(contentsOf: disk.pageURL(chapter.id, "original.image")), Data([1, 2, 3]))
    }
    func testSharedPromptIncludesTargetAndUserInputAsJSON() throws {
        var config = Preferences(); config.target = "vi-VN"; config.instructions = "Preserve honorifics."
        let (system, user) = try TranslationContract.prompts(["こんにちは！"], preferences: config)
        XCTAssertTrue(system.contains("Vietnamese"))
        XCTAssertTrue(system.contains("Copy every input ID exactly once"))
        let payload = try JSONDecoder().decode(TranslationPayload.self, from: Data(user.utf8))
        XCTAssertEqual(payload.segments.first?.text, "こんにちは！")
        XCTAssertEqual(payload.target_language, "vi-VN")
    }
}
