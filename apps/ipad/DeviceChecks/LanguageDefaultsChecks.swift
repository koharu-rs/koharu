import XCTest
@testable import KoharuPad

final class LanguageDefaultsChecks: XCTestCase {
    func testSystemLanguageResolutionKeepsUIAndTranslationAligned() {
        let cases: [([String], InterfaceLanguage, String)] = [
            (["vi-VN"], .vietnamese, "vi-VN"),
            (["en-US"], .englishUS, "en-US"),
            (["en-GB"], .englishUK, "en-GB"),
            (["fr-CA"], .french, "fr-FR"),
            (["zh-HK"], .traditionalChinese, "zh-TW"),
            (["zh-SG"], .simplifiedChinese, "zh-CN"),
            (["th-TH", "ko-KR"], .korean, "ko-KR"),
            (["th-TH"], .englishUS, "en-US"),
            ([], .englishUS, "en-US")
        ]
        for (preferred, language, target) in cases {
            let resolved = InterfaceLanguage.preferred(for: preferred)
            XCTAssertEqual(resolved, language, "\(preferred)")
            XCTAssertEqual(resolved.translationTarget, target)
        }
        for language in InterfaceLanguage.allCases where language != .system {
            XCTAssertTrue(TargetLanguage.all.contains { $0.tag == language.translationTarget })
            XCTAssertNotNil(Bundle.main.path(forResource: language.rawValue, ofType: "lproj"))
        }
    }

    @MainActor func testFreshLibraryInheritsInterfaceAndReopenKeepsSavedTarget() throws {
        let localization = AppLocalization.shared
        let previous = localization.language
        let stored = UserDefaults.standard.object(forKey: "interfaceLanguage")
        defer {
            localization.language = previous
            if let stored { UserDefaults.standard.set(stored, forKey: "interfaceLanguage") }
            else { UserDefaults.standard.removeObject(forKey: "interfaceLanguage") }
        }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        localization.language = .french
        let disk = try LibraryDisk(root: root)
        let fresh = MangaLibrary(disk: disk)
        XCTAssertNil(fresh.error)
        XCTAssertEqual(fresh.preferences.target, "fr-FR")
        localization.language = .german
        XCTAssertEqual(Preferences().target, "de-DE")
        XCTAssertEqual(MangaLibrary(disk: disk).preferences.target, "fr-FR")
        fresh.preferences.target = "ja-JP"
        fresh.savePreferences()
        XCTAssertEqual(MangaLibrary(disk: disk).preferences.target, "ja-JP")
    }
}
