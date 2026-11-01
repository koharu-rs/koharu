import XCTest
import UIKit
@testable import KoharuPad

final class ReaderGestureChecks: XCTestCase {
    @MainActor func testReaderReservesBackSwipesAndRestoresTheirOriginalState() throws {
        let navigation = UINavigationController(rootViewController: UIViewController())
        navigation.loadViewIfNeeded()
        let edge = try XCTUnwrap(navigation.interactivePopGestureRecognizer)
        let content = try XCTUnwrap(navigation.interactiveContentPopGestureRecognizer)
        let reader = ReaderNavigationController()
        navigation.pushViewController(reader, animated: false)
        for edgeEnabled in [true, false] {
            edge.isEnabled = edgeEnabled; content.isEnabled = true
            reader.viewWillAppear(false)
            XCTAssertFalse(edge.isEnabled)
            XCTAssertFalse(content.isEnabled)
            reader.viewDidAppear(false)
            reader.viewWillDisappear(false)
            XCTAssertEqual(edge.isEnabled, edgeEnabled)
            XCTAssertTrue(content.isEnabled)
            reader.restoreNavigationGestures()
            XCTAssertEqual(edge.isEnabled, edgeEnabled)
        }
    }
}
