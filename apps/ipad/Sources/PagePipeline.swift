import Foundation
import UIKit
import Vision
import ImageIO

actor PagePipeline {
    func recognize(_ url: URL, modelID: String? = nil, modelRoot: URL? = nil) async throws -> [TextSegment] {
        try Task.checkCancellation()
        // Document recognition exposes vertical CJK lines and their direction;
        // the legacy text request missed most speech on the same manga page.
        var request = RecognizeDocumentsRequest()
        request.textRecognitionOptions.recognitionLanguages = [Locale.Language(identifier: "ja-JP"), Locale.Language(identifier: "en-US")]
        request.textRecognitionOptions.automaticallyDetectLanguage = false
        request.textRecognitionOptions.useLanguageCorrection = false
        request.textRecognitionOptions.minimumTextHeightFraction = 0.004
        request.barcodeDetectionOptions.enabled = false
        let documents = try await request.perform(on: url)
        try Task.checkCancellation()
        var regions = documents.flatMap { $0.document.text.lines }.compactMap { item -> (CGRect, String, Bool)? in
            let text = item.transcript
            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            let r = item.boundingBox.cgRect
            return (CGRect(x: r.minX, y: 1 - r.maxY, width: r.width, height: r.height), text, item.textDirection == .topToBottom)
        }.sorted { a, b in
            if abs(a.0.minY - b.0.minY) > 0.04 { return a.0.minY < b.0.minY }
            return a.0.maxX > b.0.maxX
        }
        // Join nearby lines/columns only when they share orientation and overlap.
        var merged: [(CGRect, String, Bool)] = []
        while !regions.isEmpty {
            var region = regions.removeFirst()
            var changed = true
            while changed {
                changed = false
                for index in regions.indices {
                    let next = regions[index]
                    let vertical = region.2
                    guard vertical == next.2 else { continue }
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
        var segments = merged.enumerated().map { TextSegment(id: $0.offset, source: $0.element.1, rect: $0.element.0) }
        if let modelID, !segments.isEmpty {
            guard let modelRoot, let image = UIImage(contentsOfFile: url.path)?.cgImage else { throw AppError.message(L("Could not decode source image")) }
            let crops = try segments.map { segment -> UIImage in
                let rect = CGRect(x: segment.rect.minX * CGFloat(image.width), y: segment.rect.minY * CGFloat(image.height), width: segment.rect.width * CGFloat(image.width), height: segment.rect.height * CGFloat(image.height)).insetBy(dx: -2, dy: -2).integral.intersection(CGRect(x: 0, y: 0, width: image.width, height: image.height))
                guard let crop = image.cropping(to: rect) else { throw AppError.message(L("Could not prepare OCR crop")) }
                return UIImage(cgImage: crop)
            }
            let texts = try performVisionInference { try VisionONNX().read(crops, model: VisionModel.named(modelID), root: modelRoot) }
            for index in segments.indices { segments[index].source = texts[index] }
        }
        return segments
    }
    func render(_ source: URL, segments: [TextSegment], inpaint: Bool, modelID: String = "lama", modelRoot: URL) async throws -> Data {
        try Task.checkCancellation()
        guard let image = UIImage(contentsOfFile: source.path) else { throw AppError.message(L("Could not decode source image")) }
        let size = image.size
        let boxes = segments.filter { $0.text != nil }.map { segment in
            CGRect(x: segment.rect.minX * size.width, y: segment.rect.minY * size.height, width: segment.rect.width * size.width, height: segment.rect.height * size.height).insetBy(dx: -3, dy: -3).intersection(CGRect(origin: .zero, size: size))
        }
        let background: UIImage?
        if inpaint && !boxes.isEmpty {
            background = try performVisionInference { try inpaintImage(image, boxes: boxes, modelID: modelID, modelRoot: modelRoot) }
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
    private func performVisionInference<T>(_ operation: () throws -> T) throws -> T {
        do { return try autoreleasepool(invoking: operation) }
        catch {
            let message = error.localizedDescription.lowercased()
            let allocationFailed = message.contains("bad_alloc") || message.contains("out of memory") || message.contains("failed to allocate memory") ||
                ((error as NSError).domain == NSPOSIXErrorDomain && (error as NSError).code == Int(ENOMEM))
            guard allocationFailed else { throw error }
            // Every stage owns its sessions locally. Unwinding + the autorelease
            // pool unloads them before a fresh memory check in the one retry.
            try Task.checkCancellation()
            Thread.sleep(forTimeInterval: 0.15)
            try Task.checkCancellation()
            return try autoreleasepool(invoking: operation)
        }
    }

}
