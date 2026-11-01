import Foundation
import UIKit
import Vision
import CoreML
import ImageIO

actor PagePipeline {
    private var cleanup: MLModel?
    func recognize(_ url: URL) throws -> [TextSegment] {
        try Task.checkCancellation()
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        let supported = try request.supportedRecognitionLanguages()
        request.recognitionLanguages = ["ja-JP", "en-US"].filter { supported.contains($0) }
        request.automaticallyDetectsLanguage = true
        request.usesLanguageCorrection = false
        request.minimumTextHeight = 0.004
        let handler = VNImageRequestHandler(url: url, options: [:])
        try handler.perform([request])
        try Task.checkCancellation()
        var regions = (request.results ?? []).compactMap { item -> (CGRect, String)? in
            guard let text = item.topCandidates(1).first?.string, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            let r = item.boundingBox
            return (CGRect(x: r.minX, y: 1 - r.maxY, width: r.width, height: r.height), text)
        }.sorted { a, b in
            if abs(a.0.minY - b.0.minY) > 0.04 { return a.0.minY < b.0.minY }
            return a.0.maxX > b.0.maxX
        }
        // Join nearby lines/columns only when they share orientation and overlap.
        var merged: [(CGRect, String)] = []
        while !regions.isEmpty {
            var region = regions.removeFirst()
            var changed = true
            while changed {
                changed = false
                for index in regions.indices {
                    let next = regions[index]
                    let vertical = region.0.height > region.0.width * 1.5
                    guard vertical == (next.0.height > next.0.width * 1.5) else { continue }
                    let overlap = region.0.intersection(next.0)
                    let along = vertical ? min(region.0.maxY, next.0.maxY) - max(region.0.minY, next.0.minY) : min(region.0.maxX, next.0.maxX) - max(region.0.minX, next.0.minX)
                    let length = vertical ? min(region.0.height, next.0.height) : min(region.0.width, next.0.width)
                    let gap = vertical ? max(region.0.minX, next.0.minX) - min(region.0.maxX, next.0.maxX) : max(region.0.minY, next.0.minY) - min(region.0.maxY, next.0.maxY)
                    let thickness = vertical ? min(region.0.width, next.0.width) : min(region.0.height, next.0.height)
                    if (overlap.isNull || overlap.isEmpty) && along > length * 0.6 && gap >= 0 && gap < thickness * 0.8 {
                        region.1 = (vertical && next.0.maxX > region.0.maxX) ? next.1 + region.1 : region.1 + (vertical ? "" : " ") + next.1
                        region.0 = region.0.union(next.0)
                        regions.remove(at: index); changed = true; break
                    }
                }
            }
            merged.append(region)
        }
        return merged.enumerated().map { TextSegment(id: $0.offset, source: $0.element.1, rect: $0.element.0) }
    }
    func render(_ source: URL, segments: [TextSegment], inpaint: Bool, modelRoot: URL) async throws -> Data {
        try Task.checkCancellation()
        guard let image = UIImage(contentsOfFile: source.path) else { throw AppError.message(L("Could not decode source image")) }
        let size = image.size
        let boxes = segments.filter { $0.text != nil }.map { segment in
            CGRect(x: segment.rect.minX * size.width, y: segment.rect.minY * size.height, width: segment.rect.width * size.width, height: segment.rect.height * size.height).insetBy(dx: -3, dy: -3).intersection(CGRect(origin: .zero, size: size))
        }
        let background: UIImage?
        if inpaint && !boxes.isEmpty {
            do { background = try await inpaintImage(image, boxes: boxes, modelRoot: modelRoot) }
            catch { cleanup = nil; throw error }
        } else { background = nil }
        try Task.checkCancellation()
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        let rendered = UIGraphicsImageRenderer(size: size, format: format).image { context in
            image.draw(in: CGRect(origin: .zero, size: size))
            if let background {
                context.cgContext.saveGState()
                for box in boxes { context.cgContext.addRect(box) }
                context.cgContext.clip()
                background.draw(in: CGRect(origin: .zero, size: size))
                context.cgContext.restoreGState()
            }
            for segment in segments {
                guard let text = segment.text else { continue }
                let rect = CGRect(x: segment.rect.minX * size.width, y: segment.rect.minY * size.height, width: segment.rect.width * size.width, height: segment.rect.height * size.height).insetBy(dx: -2, dy: -2).intersection(CGRect(origin: .zero, size: size))
                guard rect.width > 1 && rect.height > 1 else { continue }
                if background == nil { UIColor.white.setFill(); UIBezierPath(roundedRect: rect, cornerRadius: 2).fill() }
                guard !text.isEmpty else { continue }
                let paragraph = NSMutableParagraphStyle(); paragraph.alignment = .center; paragraph.lineBreakMode = .byWordWrapping
                var lo: CGFloat = 3, hi = min(42, max(10, rect.height * 0.7))
                for _ in 0..<12 {
                    let font = UIFont.systemFont(ofSize: (lo + hi) / 2, weight: .medium)
                    let measured = (text as NSString).boundingRect(with: CGSize(width: max(1, rect.width - 4), height: .greatestFiniteMagnitude), options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: [.font: font, .paragraphStyle: paragraph], context: nil)
                    if measured.height <= rect.height - 2 { lo = (lo + hi) / 2 } else { hi = (lo + hi) / 2 }
                }
                let attributes: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: lo, weight: .medium), .foregroundColor: UIColor.black, .strokeColor: UIColor.white, .strokeWidth: -3, .paragraphStyle: paragraph]
                let measured = (text as NSString).boundingRect(with: CGSize(width: max(1, rect.width - 4), height: .greatestFiniteMagnitude), options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: attributes, context: nil)
                (text as NSString).draw(in: CGRect(x: rect.minX + 2, y: rect.midY - measured.height / 2, width: max(1, rect.width - 4), height: measured.height), withAttributes: attributes)
            }
        }
        guard let data = rendered.pngData() else { throw AppError.message(L("Could not render translated page")) }
        return data
    }
    private func inpaintImage(_ image: UIImage, boxes: [CGRect], modelRoot: URL) async throws -> UIImage {
        if cleanup == nil {
            let file = modelRoot.appendingPathComponent("LaMa.mlmodelc")
            guard FileManager.default.fileExists(atPath: file.path) else { throw AppError.message(L("Download the cleanup model in Settings or disable AI cleanup")) }
            let config = MLModelConfiguration(); config.computeUnits = .all
            cleanup = try await MLModel.load(contentsOf: file, configuration: config)
        }
        guard let cleanup else { throw AppError.message(L("Cleanup model unavailable")) }
        let edge = 512
        let format = UIGraphicsImageRendererFormat(); format.scale = 1; format.opaque = true
        let small = UIGraphicsImageRenderer(size: CGSize(width: edge, height: edge), format: format).image { _ in image.draw(in: CGRect(x: 0, y: 0, width: edge, height: edge)) }
        guard let cg = small.cgImage else { throw AppError.message(L("Could not prepare cleanup image")) }
        var pixels = [UInt8](repeating: 0, count: edge * edge * 4)
        try pixels.withUnsafeMutableBytes { buffer in
            guard let canvas = CGContext(data: buffer.baseAddress, width: edge, height: edge, bitsPerComponent: 8, bytesPerRow: edge * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw AppError.message(L("Could not allocate cleanup pixels")) }
            canvas.draw(cg, in: CGRect(x: 0, y: 0, width: edge, height: edge))
        }
        let rgb = try MLMultiArray(shape: [1, 3, 512, 512], dataType: .float32)
        let mask = try MLMultiArray(shape: [1, 1, 512, 512], dataType: .float32)
        let rgbData = rgb.dataPointer.assumingMemoryBound(to: Float.self)
        let maskData = mask.dataPointer.assumingMemoryBound(to: Float.self)
        let count = edge * edge
        for i in 0..<count { for c in 0..<3 { rgbData[c * count + i] = Float(pixels[4 * i + c]) / 255 }; maskData[i] = 0 }
        for box in boxes {
            let r = CGRect(x: box.minX / image.size.width * 512, y: box.minY / image.size.height * 512, width: box.width / image.size.width * 512, height: box.height / image.size.height * 512).integral.intersection(CGRect(x: 0, y: 0, width: 512, height: 512))
            for y in max(0, Int(r.minY))..<min(512, Int(r.maxY)) { for x in max(0, Int(r.minX))..<min(512, Int(r.maxX)) { maskData[y * 512 + x] = 1 } }
        }
        let input = try MLDictionaryFeatureProvider(dictionary: ["image": MLFeatureValue(multiArray: rgb), "mask": MLFeatureValue(multiArray: mask)])
        let prediction = try await cleanup.prediction(from: input)
        try Task.checkCancellation()
        guard let output = prediction.featureValue(for: "inpainted_image")?.multiArrayValue, output.count == count * 3, output.dataType == .float32 else { throw AppError.message(L("Unexpected cleanup model output")) }
        let values = output.dataPointer.assumingMemoryBound(to: Float.self)
        for i in 0..<count { for c in 0..<3 { pixels[4 * i + c] = UInt8(max(0, min(255, values[c * count + i] * 255))) }; pixels[4 * i + 3] = 255 }
        guard let provider = CGDataProvider(data: Data(pixels) as CFData), let result = CGImage(width: edge, height: edge, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: edge * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue), provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent) else { throw AppError.message(L("Could not render cleanup result")) }
        return UIImage(cgImage: result)
    }
}
