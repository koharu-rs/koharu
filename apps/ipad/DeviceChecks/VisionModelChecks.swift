import XCTest
import UIKit
import os
@testable import KoharuPad

final class VisionModelChecks: XCTestCase {
    private func inputs() throws -> (LibraryDisk, URL) {
        let disk = try LibraryDisk()
        let documents = try XCTUnwrap(FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first)
        let fixtures = documents.appendingPathComponent("VisionChecks")
        guard FileManager.default.fileExists(atPath: fixtures.appendingPathComponent("title.png").path) else { throw XCTSkip("Copy the optional VisionChecks fixtures first") }
        return (disk, fixtures)
    }
    func testOnnxOCRMatchesPinnedPythonReference() throws {
        let (disk, fixtures) = try inputs()
        let image = try XCTUnwrap(UIImage(contentsOfFile: fixtures.appendingPathComponent("title.png").path))
        let reference = try JSONDecoder().decode([String: [String: String]].self, from: Data(contentsOf: fixtures.appendingPathComponent("ocr-text.json")))
        for id in ["manga", "baberu", "hayai"] {
            let model = try VisionModel.named(id)
            guard model.ready(disk.modelsURL) else { throw XCTSkip("Copy the pinned \(id) model files first") }
            let started = Date()
            let output = try VisionONNX().read([image], model: model, root: disk.modelsURL)
            XCTAssertEqual(output, [try XCTUnwrap(reference[id]?["raw"])], id)
            print("OCR \(id): \(output) elapsed \(Date().timeIntervalSince(started)) s; available RAM \(os_proc_available_memory())")
        }
    }
    func testOnnxInpaintingMatchesReferenceAndPreservesUnmaskedPixels() async throws {
        let (disk, fixtures) = try inputs()
        let tile = try XCTUnwrap(UIImage(contentsOfFile: fixtures.appendingPathComponent("inpaint.png").path))
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        let image = UIGraphicsImageRenderer(size: CGSize(width: 720, height: 1200), format: format).image { canvas in
            UIColor.white.setFill(); canvas.fill(CGRect(x: 0, y: 0, width: 720, height: 1200)); tile.draw(at: .zero)
        }
        let original = try VisionRaster(image)
        for id in ["lama", "aot"] {
            guard try VisionModel.named(id).ready(disk.modelsURL) else { throw XCTSkip("Copy the pinned \(id) model files first") }
            let started = Date()
            let output = try await PagePipeline().inpaintImage(image, boxes: [CGRect(x: 76, y: 156, width: 288, height: 61)], modelID: id, modelRoot: disk.modelsURL)
            let pixels = try VisionRaster(output)
            let expected = try VisionRaster(XCTUnwrap(UIImage(contentsOfFile: fixtures.appendingPathComponent(id + "-reference.png").path)))
            XCTAssertEqual(pixels.width, original.width); XCTAssertEqual(pixels.height, original.height)
            var difference = 0, count = 0, changedOutside = 0
            for y in 0..<original.height { for x in 0..<original.width { for c in 0..<3 {
                let i = (y * original.width + x) * 3 + c
                if x >= 76 && x < 364 && y >= 156 && y < 217 { difference += abs(Int(pixels.rgb[i]) - Int(expected.rgb[(y * expected.width + x) * 3 + c])); count += 1 }
                else if pixels.rgb[i] != original.rgb[i] { changedOutside += 1 }
            } } }
            let mae = Double(difference) / Double(count)
            XCTAssertLessThan(mae, 2, id); XCTAssertEqual(changedOutside, 0, id)
            try output.pngData()?.write(to: fixtures.appendingPathComponent(id + "-device.png"))
            print("Inpaint \(id): masked MAE \(mae); changed outside \(changedOutside); \(Date().timeIntervalSince(started)) s")
        }
    }
    func testPaddleOCRRunsOnDevice() throws {
        let (disk, fixtures) = try inputs()
        let model = try VisionModel.named("paddle")
        guard model.ready(disk.modelsURL) else { throw XCTSkip("Copy pinned PaddleOCR files first") }
        do { try model.checkMemory() } catch { throw XCTSkip(error.localizedDescription) }
        let image = try XCTUnwrap(UIImage(contentsOfFile: fixtures.appendingPathComponent("title.png").path))
        let output = try VisionONNX().read([image], model: model, root: disk.modelsURL)
        XCTAssertEqual(output, ["対策委員会です！"], "Must match the same pinned GGUF/runtime on Mac")
        print("PaddleOCR output: \(output)")
    }
    func testRoremInpaintingRunsOnDevice() async throws { try await checkDiffusion("rorem") }
    func testFluxInpaintingRunsOnDevice() async throws { try await checkDiffusion("flux") }
    private func checkDiffusion(_ id: String) async throws {
        let (disk, fixtures) = try inputs()
        let model = try VisionModel.named(id)
        guard model.ready(disk.modelsURL) else { throw XCTSkip("Copy pinned \(id) files first") }
        do { try model.checkMemory() } catch { throw XCTSkip(error.localizedDescription) }
        let image = try XCTUnwrap(UIImage(contentsOfFile: fixtures.appendingPathComponent("inpaint.png").path))
        let output = try await PagePipeline().inpaintImage(image, boxes: [CGRect(x: 76, y: 156, width: 288, height: 61)], modelID: id, modelRoot: disk.modelsURL)
        XCTAssertEqual(output.size, image.size)
        try output.pngData()?.write(to: fixtures.appendingPathComponent(id + "-device.png"))
        print("Inpaint \(id) finished on device")
    }
}
