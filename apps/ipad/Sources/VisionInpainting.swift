import Foundation
import UIKit
import OnnxRuntimeBindings

struct InpaintTile: Equatable {
    let x: Int
    let y: Int
    static let edge = 512
    var rect: CGRect { CGRect(x: x, y: y, width: Self.edge, height: Self.edge) }
    static func covering(_ boxes: [CGRect], size: CGSize) -> [InpaintTile] {
        var result: [InpaintTile] = []
        for box in boxes {
            for y in stride(from: Int(box.minY), to: max(Int(box.minY) + 1, Int(ceil(box.maxY))), by: 384) {
                for x in stride(from: Int(box.minX), to: max(Int(box.minX) + 1, Int(ceil(box.maxX))), by: 384) {
                    let part = CGRect(x: CGFloat(x), y: CGFloat(y), width: min(384, box.maxX - CGFloat(x)), height: min(384, box.maxY - CGFloat(y)))
                    if result.contains(where: { $0.rect.insetBy(dx: 32, dy: 32).contains(part) }) { continue }
                    let left = max(0, min(max(0, Int(size.width) - edge), Int(part.midX) - edge / 2))
                    let top = max(0, min(max(0, Int(size.height) - edge), Int(part.midY) - edge / 2))
                    let tile = InpaintTile(x: left, y: top)
                    if !result.contains(tile) { result.append(tile) }
                }
            }
        }
        return result
    }
}

extension PagePipeline {
    func inpaintImage(_ image: UIImage, boxes: [CGRect], modelID: String, modelRoot: URL) throws -> UIImage {
        let model = try VisionModel.named(modelID)
        guard model.kind == "inpaint", model.ready(modelRoot) else { throw AppError.message(L("Download the selected model in Settings first")) }
        try model.checkMemory()
        let directory = model.directory(modelRoot)
        let onnx = try VisionONNX()
        let session = ["lama", "aot"].contains(modelID) ? try onnx.session(directory.appendingPathComponent("model.onnx"), coreMLCache: modelID == "aot" ? directory.appendingPathComponent("coreml-" + model.files[0].sha256) : nil) : nil
        var error = [CChar](repeating: 0, count: 4096)
        let diffusion = session == nil ? koharu_diffusion_open(directory.path, modelID == "flux" ? 1 : 0, &error, error.count) : nil
        if session == nil && diffusion == nil { throw AppError.message(L(String(cString: error))) }
        defer { if let diffusion { koharu_diffusion_close(diffusion) } }
        let source = try VisionRaster(image)
        var result = source
        let edge = InpaintTile.edge, count = edge * edge
        for tile in InpaintTile.covering(boxes, size: image.size) {
            try Task.checkCancellation()
            var pixels = [UInt8](repeating: 255, count: count * 3), mask = [UInt8](repeating: 0, count: count)
            for y in 0..<edge where tile.y + y < source.height { for x in 0..<edge where tile.x + x < source.width {
                let src = ((tile.y + y) * source.width + tile.x + x) * 3, dst = (y * edge + x) * 3
                for c in 0..<3 { pixels[dst + c] = source.rgb[src + c] }
            } }
            for box in boxes {
                let intersection = box.intersection(tile.rect).integral
                guard !intersection.isNull && !intersection.isEmpty else { continue }
                for y in max(0, Int(intersection.minY) - tile.y)..<min(edge, Int(intersection.maxY) - tile.y) {
                    for x in max(0, Int(intersection.minX) - tile.x)..<min(edge, Int(intersection.maxX) - tile.x) { mask[y * edge + x] = 255 }
                }
            }
            let output: [UInt8]
            if let session {
                var values = VisionRaster(width: edge, height: edge, rgb: pixels).normalized(mean: modelID == "aot" ? [0.5,0.5,0.5] : [0,0,0], std: modelID == "aot" ? [0.5,0.5,0.5] : [1,1,1])
                let maskValues = mask.map { Float($0) / 255 }
                if modelID == "aot" { for i in 0..<count where mask[i] != 0 { for c in 0..<3 { values[c * count + i] = 0 } } }
                let prediction = try VisionONNX.output("output", from: VisionONNX.run(session, [modelID == "aot" ? "img" : "image": VisionONNX.floats(values, [1,3,edge,edge]), "mask": VisionONNX.floats(maskValues, [1,1,edge,edge])]))
                let data = try prediction.tensorData()
                guard data.length == count * 3 * 4 else { throw AppError.message(L("Unexpected cleanup model output")) }
                let floats = data.bytes.assumingMemoryBound(to: Float.self)
                var rgb = [UInt8](repeating: 0, count: count * 3)
                for i in 0..<count { for c in 0..<3 {
                    let value = floats[c * count + i]
                    guard value.isFinite else { throw AppError.message(L("Unexpected cleanup model output")) }
                    let normalized = modelID == "aot" ? (value + 1) / 2 : value
                    rgb[i * 3 + c] = UInt8(clamping: Int((normalized * 255).rounded()))
                } }
                output = rgb
            } else {
                var rgb = [UInt8](repeating: 0, count: count * 3)
                guard koharu_diffusion_run(diffusion!, pixels, mask, Int32(edge), &rgb, &error, error.count) != 0 else { throw AppError.message(L(String(cString: error))) }
                output = rgb
            }
            try Task.checkCancellation()
            // Composite only replaced pixels at native resolution. No whole-page
            // downscale/upscale or VAE roundtrip can alter the original artwork.
            for y in 0..<edge where tile.y + y < source.height { for x in 0..<edge where tile.x + x < source.width && mask[y * edge + x] != 0 {
                let src = (y * edge + x) * 3, dst = ((tile.y + y) * source.width + tile.x + x) * 3
                for c in 0..<3 { result.rgb[dst + c] = output[src + c] }
            } }
        }
        return try result.image()
    }
}
