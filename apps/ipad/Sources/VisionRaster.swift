import UIKit

// Top-left RGB pixels are shared by OCR, inpainting and the native C runtimes.
struct VisionRaster {
    let width: Int
    let height: Int
    var rgb: [UInt8]
    init(_ image: UIImage) throws {
        guard let cg = image.cgImage else { throw AppError.message(L("Could not decode source image")) }
        width = cg.width; height = cg.height
        var rgba = [UInt8](repeating: 255, count: width * height * 4)
        guard let context = CGContext(data: &rgba, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw AppError.message(L("Could not allocate cleanup pixels")) }
        context.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
        rgb = [UInt8](repeating: 0, count: width * height * 3)
        for i in 0..<(width * height) { for c in 0..<3 { rgb[3 * i + c] = rgba[4 * i + c] } }
    }
    init(width: Int, height: Int, rgb: [UInt8]) { self.width = width; self.height = height; self.rgb = rgb }
    func image() throws -> UIImage {
        guard rgb.count == width * height * 3,
              let provider = CGDataProvider(data: Data(rgb) as CFData),
              let image = CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 24, bytesPerRow: width * 3, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: [], provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent) else { throw AppError.message(L("Could not render cleanup result")) }
        return UIImage(cgImage: image)
    }
    func resized(_ w: Int, _ h: Int, bicubic: Bool = false) -> VisionRaster {
        let floats = Self.resize(rgb.map(Float.init), width: width, height: height, channels: 3, toWidth: w, toHeight: h, bicubic: bicubic, quantized: true)
        return VisionRaster(width: w, height: h, rgb: floats.map { UInt8(clamping: Int($0.rounded())) })
    }
    func normalized(mean: [Float], std: [Float]) -> [Float] {
        let count = width * height
        var output = [Float](repeating: 0, count: count * 3)
        for i in 0..<count { for c in 0..<3 { output[c * count + i] = (Float(rgb[3 * i + c]) / 255 - mean[c]) / std[c] } }
        return output
    }
    func grayscale() -> VisionRaster {
        var result = self
        for i in 0..<(width * height) {
            let v = (19595 * Int(rgb[i * 3]) + 38470 * Int(rgb[i * 3 + 1]) + 7471 * Int(rgb[i * 3 + 2]) + 32768) >> 16
            for c in 0..<3 { result.rgb[i * 3 + c] = UInt8(v) }
        }
        return result
    }
    // Pillow-compatible separable bilinear/Catmull-Rom convolution, including
    // widened filter support when reducing. Float grids retain their precision.
    static func resize(_ pixels: [Float], width: Int, height: Int, channels: Int, toWidth w: Int, toHeight h: Int, bicubic: Bool = false, quantized: Bool = false) -> [Float] {
        func weights(_ source: Int, _ target: Int) -> [[(Int, Float)]] {
            let scale = Double(source) / Double(target), factor = max(1, scale), support = (bicubic ? 2.0 : 1.0) * factor
            return (0..<target).map { x in
                let center = (Double(x) + 0.5) * scale
                let lo = max(0, Int(center - support + 0.5)), hi = min(source, Int(center + support + 0.5))
                var entries: [(Int, Float)] = []
                for i in lo..<max(lo, hi) {
                    let t = abs((Double(i) + 0.5 - center) / factor)
                    let weight: Double
                    if bicubic { weight = t < 1 ? (1.5 * t - 2.5) * t * t + 1 : (t < 2 ? ((-0.5 * t + 2.5) * t - 4) * t + 2 : 0) }
                    else { weight = max(0, 1 - t) }
                    entries.append((i, Float(weight)))
                }
                let sum = entries.reduce(Float(0)) { $0 + $1.1 }
                return entries.map { ($0.0, $0.1 / sum) }
            }
        }
        let xs = weights(width, w), ys = weights(height, h)
        var temp = [Float](repeating: 0, count: height * w * channels)
        for y in 0..<height { for x in 0..<w { for c in 0..<channels {
            var value: Float = 0; for (i, weight) in xs[x] { value += pixels[(y * width + i) * channels + c] * weight }
            temp[(y * w + x) * channels + c] = quantized ? min(255, max(0, value.rounded())) : value
        } } }
        var result = [Float](repeating: 0, count: h * w * channels)
        for y in 0..<h { for x in 0..<w { for c in 0..<channels {
            var value: Float = 0; for (i, weight) in ys[y] { value += temp[(i * w + x) * channels + c] * weight }
            result[(y * w + x) * channels + c] = quantized ? min(255, max(0, value.rounded())) : value
        } } }
        return result
    }
}
