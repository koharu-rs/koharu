import Foundation
import UIKit
import OnnxRuntimeBindings

// Pre/postprocessing follows the checkpoint-pinned publisher implementations:
// Baberu onnx_infer.py; Hayai inference_onnx.py; Manga OCR's ViT/TrOCR +
// manga_ocr postprocessor. Exact artifact revisions/hashes live in vision-models.json.
final class VisionONNX {
    let environment: ORTEnv
    init() throws { environment = try ORTEnv(loggingLevel: .error) }
    func session(_ url: URL, coreMLCache: URL? = nil) throws -> ORTSession {
        let options = try ORTSessionOptions()
        try options.setIntraOpNumThreads(4)
        if let coreMLCache {
            try FileManager.default.createDirectory(at: coreMLCache, withIntermediateDirectories: true)
            try options.appendCoreMLExecutionProvider(withOptionsV2: ["ModelFormat": "MLProgram", "MLComputeUnits": "ALL", "ModelCacheDirectory": coreMLCache.path])
        }
        return try ORTSession(env: environment, modelPath: url.path, sessionOptions: options)
    }
    static func floats(_ values: [Float], _ shape: [Int]) throws -> ORTValue {
        let data = values.withUnsafeBytes { NSMutableData(bytes: $0.baseAddress!, length: $0.count) }
        return try ORTValue(tensorData: data, elementType: .float, shape: shape.map(NSNumber.init(value:)))
    }
    static func integers(_ values: [Int64], _ shape: [Int]) throws -> ORTValue {
        let data = values.withUnsafeBytes { NSMutableData(bytes: $0.baseAddress!, length: $0.count) }
        return try ORTValue(tensorData: data, elementType: .int64, shape: shape.map(NSNumber.init(value:)))
    }
    static func run(_ session: ORTSession, _ inputs: [String: ORTValue]) throws -> [String: ORTValue] {
        try Task.checkCancellation()
        return try session.run(withInputs: inputs, outputNames: Set(session.outputNames()), runOptions: nil)
    }
    static func output(_ name: String, from outputs: [String: ORTValue]) throws -> ORTValue {
        guard let value = outputs[name] else { throw AppError.message(L("Unexpected OCR model output")) }
        return value
    }
    static func argmax(_ value: ORTValue, vocabulary: Int, seen: [Int] = [], penalty: Float = 1, suppress: Int? = nil) throws -> Int {
        let data = try value.tensorData()
        guard data.length >= vocabulary * 4, data.length % (vocabulary * 4) == 0 else { throw AppError.message(L("Unexpected OCR model output")) }
        let offset = data.length / 4 - vocabulary, values = data.bytes.assumingMemoryBound(to: Float.self)
        let repeated = Set(seen)
        var best = -Float.infinity, chosen = 0
        for i in 0..<vocabulary where i != suppress {
            var score = values[offset + i]
            guard score.isFinite else { throw AppError.message(L("Unexpected OCR model output")) }
            if repeated.contains(i) { score = score < 0 ? score * penalty : score / penalty }
            if score > best { best = score; chosen = i }
        }
        return chosen
    }
    func read(_ crops: [UIImage], model: VisionModel, root: URL) throws -> [String] {
        try model.checkMemory()
        guard model.ready(root) else { throw AppError.message(L("Download the selected model in Settings first")) }
        let directory = model.directory(root)
        if model.id == "paddle" {
            var error = [CChar](repeating: 0, count: 8192)
            guard let handle = koharu_paddle_open(directory.path, &error, error.count) else { throw AppError.message(L(String(cString: error))) }
            defer { koharu_paddle_close(handle) }
            return try crops.map { image in
                try Task.checkCancellation()
                let pixels = try VisionRaster(image)
                guard koharu_paddle_read(handle, pixels.rgb, Int32(pixels.width), Int32(pixels.height), &error, error.count) != 0 else { throw AppError.message(L(String(cString: error))) }
                return String(cString: error)
            }
        }
        let encoder = try session(directory.appendingPathComponent("encoder.onnx"))
        let decoder = try session(directory.appendingPathComponent(model.id == "baberu" ? "prefill.onnx" : "decoder.onnx"))
        let step = model.id == "baberu" ? try session(directory.appendingPathComponent("step.onnx")) : nil
        let vocab: [String]
        if model.id == "manga" { vocab = try String(contentsOf: directory.appendingPathComponent("vocab.txt"), encoding: .utf8).split(whereSeparator: \.isNewline).map(String.init) }
        else if model.id == "baberu" { vocab = ["", "", "", ""] + (try JSONDecoder().decode([String].self, from: Data(contentsOf: directory.appendingPathComponent("vocab.json")))) }
        else { vocab = [] }
        guard model.id != "manga" || vocab.count == 6144, model.id != "baberu" || vocab.count == 14630 else { throw AppError.message(L("Invalid OCR vocabulary")) }
        var hayaiVocabulary: [String: Int] = [:]
        var hayaiIDs: [Int: String] = [:]
        var base: [Float] = []
        if model.id == "hayai" {
            guard let json = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("tokenizer.json"))) as? [String: Any],
                  let tokenizer = json["model"] as? [String: Any], let vocabulary = tokenizer["vocab"] as? [String: Int], let added = json["added_tokens"] as? [[String: Any]] else { throw AppError.message(L("Invalid OCR vocabulary")) }
            hayaiVocabulary = vocabulary
            for entry in added { if let text = entry["content"] as? String, let id = entry["id"] as? Int { hayaiVocabulary[text] = id } }
            for (text, id) in hayaiVocabulary {
                guard hayaiIDs.updateValue(text, forKey: id) == nil else { throw AppError.message(L("Invalid OCR vocabulary")) }
            }
            let data = try Data(contentsOf: directory.appendingPathComponent("position_base.npy"))
            guard data.count > 10, Array(data.prefix(6)) == [147, 78, 85, 77, 80, 89], data[6] == 1 else { throw AppError.message(L("Invalid OCR position grid")) }
            let start = 10 + Int(data[8]) + (Int(data[9]) << 8)
            guard data.count - start == 16 * 16 * 768 * 4 else { throw AppError.message(L("Invalid OCR position grid")) }
            base = data.subdata(in: start..<data.count).withUnsafeBytes { Array($0.bindMemory(to: Float.self)) }
        }
        return try crops.map { image in
            let raster = try VisionRaster(image)
            switch model.id {
            case "manga":
                let pixels = raster.grayscale().resized(224, 224).normalized(mean: [0.5,0.5,0.5], std: [0.5,0.5,0.5])
                let encoded = try Self.output("last_hidden_state", from: Self.run(encoder, ["pixel_values": Self.floats(pixels, [1,3,224,224])]))
                var ids: [Int] = [2]
                for _ in 0..<299 {
                    let output = try Self.output("logits", from: Self.run(decoder, ["input_ids": Self.integers(ids.map(Int64.init), [1,ids.count]), "encoder_hidden_states": encoded]))
                    let next = try Self.argmax(output, vocabulary: 6144)
                    if next == 3 { return Self.mangaText(ids.dropFirst().map { vocab[$0] }.joined()) }
                    ids.append(next)
                }
                throw AppError.message(L("OCR reached its output limit without finishing"))
            case "baberu":
                let pixels = raster.resized(224, 224, bicubic: true).normalized(mean: [0.485,0.456,0.406], std: [0.229,0.224,0.225])
                let encoded = try Self.output("vision_embeds", from: Self.run(encoder, ["pixel_values": Self.floats(pixels, [1,3,224,224])]))
                var outputs = try Self.run(decoder, ["vision_embeds": encoded, "input_ids": Self.integers([1], [1,1])])
                var ids = [1], position = 257
                for _ in 0..<256 {
                    let last = ids[ids.count - 1], text = vocab[last]
                    let content = text.unicodeScalars.count == 1 && text.unicodeScalars.allSatisfy { CharacterSet.letters.union(.decimalDigits).contains($0) } && !["ー","ｰ","〜","~"].contains(text)
                    let run = ids.reversed().prefix(while: { $0 == last }).count
                    let suppressed = last > 3 && run >= (content ? 12 : 16) ? last : nil
                    let next = try Self.argmax(try Self.output("logits", from: outputs), vocabulary: vocab.count, seen: ids, penalty: 1.2, suppress: suppressed)
                    if next == 2 { return ids.dropFirst().map { vocab[$0] }.joined() }
                    ids.append(next)
                    var inputs = ["input_ids": try Self.integers([Int64(next)], [1,1]), "position_ids": try Self.integers([Int64(position)], [1,1])]
                    for kind in ["k", "v"] { for i in 0..<6 { inputs["past_\(kind)\(i)"] = try Self.output("present_\(kind)\(i)", from: outputs) } }
                    guard let step else { throw AppError.message(L("Vision model is unavailable")) }
                    outputs = try Self.run(step, inputs); position += 1
                }
                throw AppError.message(L("OCR reached its output limit without finishing"))
            case "hayai": return try self.hayai(raster, encoder: encoder, decoder: decoder, vocabulary: hayaiVocabulary, ids: hayaiIDs, base: base)
            default: throw AppError.message(L("Vision model is unavailable"))
            }
        }
    }
    private static func mangaText(_ text: String) -> String {
        let cleaned = text.replacingOccurrences(of: "##", with: "").replacingOccurrences(of: "[UNK]", with: "").replacingOccurrences(of: "[PAD]", with: "").replacingOccurrences(of: "[CLS]", with: "").replacingOccurrences(of: "[MASK]", with: "")
            .filter { !$0.isWhitespace }.replacingOccurrences(of: "…", with: "...")
        let mutable = NSMutableString(string: cleaned)
        if let regex = try? NSRegularExpression(pattern: "[.・]{2,}") {
            for match in regex.matches(in: cleaned, range: NSRange(location: 0, length: mutable.length)).reversed() {
                mutable.replaceCharacters(in: match.range, with: String(repeating: ".", count: match.range.length))
            }
        }
        let dots = mutable as String
        return dots.applyingTransform(.fullwidthToHalfwidth, reverse: true) ?? dots
    }
    private func hayai(_ raster: VisionRaster, encoder: ORTSession, decoder: ORTSession, vocabulary: [String: Int], ids: [Int: String], base: [Float]) throws -> String {
        guard let bos = vocabulary["<bos>"], let eos = vocabulary["<eos>"], let pad = vocabulary["<pad>"] else { throw AppError.message(L("Invalid OCR vocabulary")) }
        var lo = 0.000001, hi = 100.0
        func edge(_ scale: Double, _ size: Int) -> Int { max(1, Int(ceil(Double(size) * scale / 16))) * 16 }
        while hi - lo >= 0.00001 {
            let mid = (lo + hi) / 2
            if edge(mid, raster.width) * edge(mid, raster.height) / 256 <= 256 { lo = mid } else { hi = mid }
        }
        let width = edge(lo, raster.width), height = edge(lo, raster.height), ph = height / 16, pw = width / 16, count = ph * pw
        let scaled = raster.resized(width, height)
        var patches = [Float](repeating: 0, count: 256 * 768), active = [Int64](repeating: 0, count: 256)
        for py in 0..<ph { for px in 0..<pw {
            let patch = py * pw + px; active[patch] = 1
            for y in 0..<16 { for x in 0..<16 { for c in 0..<3 {
                patches[patch * 768 + (y * 16 + x) * 3 + c] = Float(scaled.rgb[((py * 16 + y) * width + px * 16 + x) * 3 + c]) / 127.5 - 1
            } } }
        } }
        let resized = VisionRaster.resize(base, width: 16, height: 16, channels: 768, toWidth: pw, toHeight: ph)
        var position = [Float](repeating: 0, count: 256 * 768)
        for i in 0..<256 { for c in 0..<768 { position[i * 768 + c] = resized[(i < count ? i : 0) * 768 + c] } }
        let visual = try Self.output("visual_features", from: Self.run(encoder, ["pixel_values": Self.floats(patches, [1,256,768]), "attention_mask": Self.integers(active, [1,256]), "pos_embeds": Self.floats(position, [1,256,768])]))
        var generated = [bos]
        for _ in 0..<63 {
            let length = 256 + generated.count
            var cosines = [Float](repeating: 1, count: length * 32), sines = [Float](repeating: 0, count: length * 32)
            for i in 0..<length {
                guard i < count || i >= 256 else { continue }
                for d in 0..<32 {
                    let coordinate = i < 256 ? (d < 16 ? i / pw : i % pw) : i - 256
                    let angle = Float(coordinate) / pow(Float(10000), Float((d % 16) * 2) / 32)
                    cosines[i * 32 + d] = cos(angle); sines[i * 32 + d] = sin(angle)
                }
            }
            var mask = [Float](repeating: -1e9, count: length * length)
            for y in 0..<length { for x in 0..<length where x < 256 || (y >= 256 && x <= y) { mask[y * length + x] = 0 } }
            let output = try Self.output("logits", from: Self.run(decoder, ["visual_features": visual, "text_token_ids": Self.integers(generated.map(Int64.init), [1,generated.count]), "cos": Self.floats(cosines, [1,length,32]), "sin": Self.floats(sines, [1,length,32]), "mask": Self.floats(mask, [1,1,length,length])]))
            let next = try Self.argmax(output, vocabulary: 16004)
            if next == eos || next == pad { return try Self.byteLevelDecode(generated.dropFirst().map { ids[$0] ?? "<invalid>" }.joined()) }
            generated.append(next)
        }
        throw AppError.message(L("OCR reached its output limit without finishing"))
    }
    private static func byteLevelDecode(_ text: String) throws -> String {
        var visible = Array(33...126) + Array(161...172) + Array(174...255)
        var unicode = visible, next = 256
        for i in 0...255 where !visible.contains(i) { visible.append(i); unicode.append(next); next += 1 }
        let map = Dictionary(uniqueKeysWithValues: zip(unicode, visible).map { (UInt32($0.0), UInt8($0.1)) })
        let bytes = text.unicodeScalars.compactMap { map[$0.value] }
        guard bytes.count == text.unicodeScalars.count, let decoded = String(bytes: bytes, encoding: .utf8) else { throw AppError.message(L("Invalid OCR text encoding")) }
        return decoded
    }
}
