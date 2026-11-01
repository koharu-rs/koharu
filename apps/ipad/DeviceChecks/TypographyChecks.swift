import XCTest
import UIKit
@testable import KoharuPad

final class TypographyChecks: XCTestCase {
    func testWholeWordsFitNarrowBubbles() throws {
        for value in ["Không ngờ mình lại có tài năng đến thế này!", "extraordinary internationalisation!", "L’extraordinaire aventure continue.", "مرحبا بالعالم الجميل", "안녕하세요 반갑습니다"] {
            for width: CGFloat in [14, 30, 70, 140] {
                let layout = PageTypography(value, size: CGSize(width: width, height: 260))
                XCTAssertEqual(layout.text.string, value)
                XCTAssertLessThanOrEqual(layout.bounds.height, 260.1)
                let storage = NSTextStorage(attributedString: layout.text)
                let manager = NSLayoutManager(); storage.addLayoutManager(manager)
                let container = NSTextContainer(size: CGSize(width: width, height: 2000)); container.lineFragmentPadding = 0
                manager.addTextContainer(container); manager.ensureLayout(for: container)
                var boundaries: [Int] = []
                manager.enumerateLineFragments(forGlyphRange: manager.glyphRange(for: container)) { _, used, _, glyphs, _ in
                    XCTAssertLessThanOrEqual(used.width, width + 0.1)
                    boundaries.append(NSMaxRange(manager.characterRange(forGlyphRange: glyphs, actualGlyphRange: nil)))
                }
                let source = value as NSString
                source.enumerateSubstrings(in: NSRange(location: 0, length: source.length), options: [.byWords, .substringNotRequired]) { _, word, _, _ in
                    XCTAssertFalse(boundaries.contains { $0 > word.location && $0 < NSMaxRange(word) }, "A line split a word: \(value), width \(width)")
                }
            }
        }
        for value in ["これは長い日本語の文章です。次の行にも自然に折り返します。", "這是一段沒有空格的中文文字，應該自然換行。"] {
            let layout = PageTypography(value, size: CGSize(width: 120, height: 240))
            XCTAssertGreaterThan(layout.font.pointSize, 8)
            XCTAssertLessThanOrEqual(layout.bounds.height, 240.1)
        }
    }
    @MainActor func testLayoutRefreshReusesCachedTextWithoutAI() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let disk = try LibraryDisk(root: root), library = MangaLibrary(disk: disk)
        library.preferences.provider = .local // No weights: an unintended AI request must fail.
        var chapter = Chapter(title: "Typography cache check", sourceURL: "https://rawkuma.net/", target: library.preferences.target, pages: [MangaPage(source: "https://cdn.rawkuma.com/check/1.jpg", original: "source.png")])
        try FileManager.default.createDirectory(at: disk.chapterURL(chapter.id), withIntermediateDirectories: true)
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        let blank = UIGraphicsImageRenderer(size: CGSize(width: 200, height: 300), format: format).image { context in
            UIColor.white.setFill(); context.fill(CGRect(x: 0, y: 0, width: 200, height: 300))
        }
        try XCTUnwrap(blank.pngData()).write(to: disk.pageURL(chapter.id, "source.png"))
        try library.update(chapter)
        library.run(chapter.id, force: false)
        try await XCTUnwrap(library.activeTask).value
        XCTAssertNil(library.error)
        chapter = try XCTUnwrap(library.chapters.first)
        XCTAssertTrue(chapter.pages[0].segments.isEmpty)
        let cached = "Không ngờ mình lại có tài năng!"
        chapter.pages[0].segments = [TextSegment(id: 0, source: "こんにちは", text: cached, rect: CGRect(x: 0.2, y: 0.1, width: 0.15, height: 0.8))]
        chapter.pages[0].textLayoutVersion = nil
        let previous = chapter.pages[0].rendered
        try library.update(chapter)
        library.run(chapter.id, force: false)
        try await XCTUnwrap(library.activeTask).value
        XCTAssertNil(library.error, library.error ?? "")
        let page = try XCTUnwrap(library.chapters.first?.pages.first)
        XCTAssertEqual(page.segments[0].text, cached)
        XCTAssertEqual(page.textLayoutVersion, PageTypography.version)
        XCTAssertNotEqual(page.rendered, previous)
        XCTAssertNotNil(UIImage(contentsOfFile: disk.pageURL(chapter.id, try XCTUnwrap(page.rendered)).path))
    }
    func testRenderPreview() async throws {
        let root = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("TypographyChecks")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        let blank = UIGraphicsImageRenderer(size: CGSize(width: 900, height: 600), format: format).image { context in
            UIColor(white: 0.9, alpha: 1).setFill(); context.fill(CGRect(x: 0, y: 0, width: 900, height: 600))
        }
        let input = root.appendingPathComponent("input.png"); try XCTUnwrap(blank.pngData()).write(to: input)
        let values = ["Không ngờ mình lại có tài năng đến thế này!", "Chỉ có một vài người chơi cảm thấy không hài lòng.", "extraordinary internationalisation!", "مرحبا بالعالم الجميل"]
        let segments = values.enumerated().map { TextSegment(id: $0.offset, source: "", text: $0.element, rect: CGRect(x: 0.04 + Double($0.offset) * 0.24, y: 0.05, width: 0.18, height: 0.9)) }
        let data = try await PagePipeline().render(input, segments: segments, inpaint: false, modelRoot: root)
        try data.write(to: root.appendingPathComponent("preview.png"))
        XCTAssertNotNil(UIImage(data: data))
    }
}
