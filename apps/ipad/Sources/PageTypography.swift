import UIKit

struct PageTypography {
    static let version = 1
    let text: NSAttributedString
    let bounds: CGRect
    let font: UIFont

    init(_ string: String, size: CGSize) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.lineBreakMode = .byWordWrapping
        paragraph.lineBreakStrategy = .hangulWordPriority
        paragraph.hyphenationFactor = 0
        let source = string as NSString
        var words: [String] = []
        // Foundation's word boundaries preserve space-delimited words while
        // allowing normal breaks in Japanese/Chinese text without spaces.
        source.enumerateSubstrings(in: NSRange(location: 0, length: source.length), options: [.byWords, .substringNotRequired]) { _, _, enclosing, _ in
            let word = source.substring(with: enclosing).trimmingCharacters(in: .whitespacesAndNewlines)
            if !word.isEmpty { words.append(word) }
        }
        if words.isEmpty { words = [string.trimmingCharacters(in: .whitespacesAndNewlines)] }
        let width = max(0.1, size.width)
        let height = max(0.1, size.height)
        var lo: CGFloat = 0.1, hi = min(42, max(10, height * 0.7))
        for _ in 0..<16 {
            let candidate = UIFont.systemFont(ofSize: (lo + hi) / 2, weight: .medium)
            let attributes: [NSAttributedString.Key: Any] = [.font: candidate, .paragraphStyle: paragraph]
            let measured = source.boundingRect(with: CGSize(width: width, height: .greatestFiniteMagnitude), options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: attributes, context: nil)
            // Word wrapping otherwise falls back to character wrapping. Check
            // whole-word width as well as height, leaving room for the outline.
            let longest = words.map { ($0 as NSString).size(withAttributes: [.font: candidate]).width }.max() ?? 0
            if longest + candidate.pointSize * 0.03 <= width && measured.height <= height { lo = candidate.pointSize }
            else { hi = candidate.pointSize }
        }
        font = UIFont.systemFont(ofSize: lo, weight: .medium)
        text = NSAttributedString(string: string, attributes: [.font: font, .paragraphStyle: paragraph, .foregroundColor: UIColor.black, .strokeColor: UIColor.white, .strokeWidth: -3])
        bounds = text.boundingRect(with: CGSize(width: width, height: .greatestFiniteMagnitude), options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil)
    }
}
