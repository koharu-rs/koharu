import XCTest
import UIKit
@testable import KoharuPad

final class ContinuousReaderChecks: XCTestCase {
    @MainActor func testContinuousPagesStayLazyAndPreservePositionWhenImageChanges() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let format = UIGraphicsImageRendererFormat(); format.scale = 1
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: 400, height: 800), format: format)
        let first = root.appendingPathComponent("original.png"), second = root.appendingPathComponent("translated.png")
        for (url, color) in [(first, UIColor.white), (second, UIColor.lightGray)] {
            let image = renderer.image { context in color.setFill(); context.fill(CGRect(x: 0, y: 0, width: 400, height: 800)) }
            try XCTUnwrap(image.pngData()).write(to: url)
        }
        let view = ContinuousPageScrollView(frame: CGRect(x: 0, y: 0, width: 400, height: 600))
        var urls = Array(repeating: first, count: 100)
        view.configure(urls: urls, index: 25) { _ in XCTFail("A layout or programmatic jump must not publish user scrolling") }
        view.layoutIfNeeded()
        XCTAssertEqual(view.pageFrames.count, 100)
        XCTAssertEqual(view.pageFrames[0].height, 800, accuracy: 0.1)
        XCTAssertEqual(view.contentOffset.y, view.pageFrames[25].minY, accuracy: 0.1)
        XCTAssertEqual(view.selectedIndex, 25)
        XCTAssertLessThan(view.loadedPageCount, 6, "A 100-page chapter must not decode all pages")
        view.setZoomScale(2, animated: false)
        view.setContentOffset(CGPoint(x: 120, y: view.pageFrames[25].minY * 2 + 100), animated: false)
        let offset = view.contentOffset
        urls[25] = second
        view.configure(urls: urls, index: 25) { _ in XCTFail("A rendered-image refresh must not change selection") }
        view.layoutIfNeeded()
        XCTAssertEqual(view.zoomScale, 2, accuracy: 0.01)
        XCTAssertEqual(view.contentOffset.y, offset.y, accuracy: 0.1)
        XCTAssertEqual(view.contentOffset.x, offset.x, accuracy: 0.1)
        view.frame.size.width = 600
        view.setNeedsLayout(); view.layoutIfNeeded()
        XCTAssertEqual(view.pageFrames[0].height, 1200, accuracy: 0.1)
        XCTAssertEqual(view.contentOffset.x, 0, accuracy: 0.1)
        XCTAssertEqual(view.contentOffset.y, view.pageFrames[25].minY, accuracy: 0.1)
        view.configure(urls: urls, index: 70) { _ in XCTFail("Slider navigation already owns its selection") }
        view.layoutIfNeeded()
        XCTAssertEqual(view.selectedIndex, 70)
        XCTAssertEqual(view.contentOffset.y, view.pageFrames[70].minY, accuracy: 0.1)
        XCTAssertLessThan(view.loadedPageCount, 6)
    }
    @MainActor func testReadingModeDefaultsAndPersistsOutsideProcessingPreferences() throws {
        let previous = UserDefaults.standard.object(forKey: "readingMode")
        defer {
            if let previous { UserDefaults.standard.set(previous, forKey: "readingMode") }
            else { UserDefaults.standard.removeObject(forKey: "readingMode") }
        }
        UserDefaults.standard.removeObject(forKey: "readingMode")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let disk = try LibraryDisk(root: root), library = MangaLibrary(disk: disk)
        XCTAssertEqual(library.readingMode, .horizontal)
        let preferences = library.preferences
        library.readingMode = .vertical
        XCTAssertEqual(MangaLibrary(disk: disk).readingMode, .vertical)
        XCTAssertEqual(library.preferences, preferences)
    }
}
