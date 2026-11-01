import XCTest
@testable import KoharuPad

final class LocalRuntimeChecks: XCTestCase {
    func testVerticalMangaOCRKeepsMultipleSpeechRegions() async throws {
        let source = try XCTUnwrap(FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first)
            .appendingPathComponent("OCRFixture.image")
        guard FileManager.default.fileExists(atPath: source.path) else {
            throw XCTSkip("Copy the optional vertical Japanese manga fixture into Documents first")
        }
        XCTAssertEqual(digest(try Data(contentsOf: source)), "3dade0142057ce7db8cf170dd14fba9614f755909c7facfb65ac0576dcbb7ec0")
        let regions = try await PagePipeline().recognize(source)
        XCTAssertGreaterThan(regions.count, 4)
        XCTAssertGreaterThan(regions.map(\.source).joined().count, 50)
        XCTAssertTrue(regions.allSatisfy { !$0.rect.isEmpty && $0.rect.minX >= 0 && $0.rect.minY >= 0 && $0.rect.maxX <= 1.001 && $0.rect.maxY <= 1.001 })
        print("Vertical manga OCR: \(regions.count) speech regions, \(regions.map(\.source).joined().count) characters")
    }
    func testMetalGGUFReturnsStructuredSegmentsOnDevice() async throws {
        // The same input is used by the Mac runtime check. Weights are supplied
        // through Files/devicectl, never bundled into the app or repository.
        let model = try XCTUnwrap(FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first)
            .appendingPathComponent("Qwen3.5-0.8B-Q4_K_M.gguf")
        guard FileManager.default.fileExists(atPath: model.path) else {
            throw XCTSkip("Copy the optional Qwen 3.5 0.8B Q4_K_M fixture into Koharu's Documents directory first")
        }
        var config = Preferences(); config.provider = .local; config.target = "vi-VN"
        let input = ["こんにちは！", "今日はいい天気ですね。"]
        let (system, user) = try TranslationContract.prompts(input, preferences: config)
        let runtime = LocalInference()
        let started = Date()
        let data: Data
        do { data = try await runtime.generate(model: model, context: 4096, system: system, user: user) }
        catch { await runtime.unload(); throw error }
        await runtime.unload()
        let result = try TranslationContract.decode(data, count: input.count)
        XCTAssertEqual(result.count, input.count)
        XCTAssertTrue(result.allSatisfy { !$0.isEmpty })
        print("Physical-device GGUF check: \(result.count) segments; \(Date().timeIntervalSince(started)) seconds including model load. Output: \(result)")
    }
}
