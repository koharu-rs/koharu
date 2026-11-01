import SwiftUI
import UIKit
import ImageIO

enum ReadingMode: String, CaseIterable, Identifiable {
    case horizontal, vertical
    var id: String { rawValue }
    var name: String { self == .horizontal ? L("Horizontal swipes (default)") : L("Vertical scrolling") }
}

struct ContinuousReader: UIViewRepresentable {
    let urls: [URL]
    let index: Int
    var selectPage: (Int) -> Void
    func makeUIView(context: Context) -> ContinuousPageScrollView {
        let view = ContinuousPageScrollView()
        view.configure(urls: urls, index: index, selectPage: selectPage)
        return view
    }
    func updateUIView(_ view: ContinuousPageScrollView, context: Context) {
        view.configure(urls: urls, index: index, selectPage: selectPage)
    }
}

final class ContinuousPageScrollView: UIScrollView, UIScrollViewDelegate {
    private let canvas = UIView()
    private var urls: [URL] = []
    private var imageSizes: [CGSize] = []
    private(set) var pageFrames: [CGRect] = []
    private var pictures: [Int: (URL, UIImageView)] = [:]
    private var selectPage: ((Int) -> Void)?
    private var requestedIndex = 0
    private(set) var selectedIndex = 0
    private var layoutWidth: CGFloat = 0
    private var rebuilding = false
    var loadedPageCount: Int { pictures.count }

    override init(frame: CGRect) {
        super.init(frame: frame)
        delegate = self
        minimumZoomScale = 1; maximumZoomScale = 6
        backgroundColor = .black
        contentInsetAdjustmentBehavior = .never
        alwaysBounceVertical = true
        addSubview(canvas)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    func configure(urls: [URL], index: Int, selectPage: @escaping (Int) -> Void) {
        self.selectPage = selectPage
        if self.urls != urls {
            // Read dimensions without decoding the whole chapter. Only pictures
            // close to the viewport are retained below, including while zooming.
            let known = Dictionary(zip(self.urls, imageSizes), uniquingKeysWith: { first, _ in first })
            let sizes = urls.map { known[$0] ?? Self.imageSize($0) }
            self.urls = urls
            if sizes != imageSizes { layoutWidth = 0 }
            imageSizes = sizes
        }
        let target = max(0, min(index, urls.count - 1))
        if target != requestedIndex {
            requestedIndex = target
            if selectedIndex != target {
                selectedIndex = target
                if pageFrames.indices.contains(target), layoutWidth == bounds.width { jump(to: target) }
            }
        }
        setNeedsLayout()
        refreshPictures()
    }
    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.width > 0 else { return }
        if layoutWidth != bounds.width {
            rebuilding = true
            let target = selectedIndex
            setZoomScale(1, animated: false)
            layoutWidth = bounds.width
            var y: CGFloat = 0
            pageFrames = imageSizes.map { size in
                let frame = CGRect(x: 0, y: y, width: layoutWidth, height: layoutWidth * size.height / max(1, size.width))
                y = frame.maxY + 8
                return frame
            }
            canvas.frame = CGRect(x: 0, y: 0, width: layoutWidth, height: max(0, y - 8))
            contentSize = canvas.bounds.size
            setContentOffset(.zero, animated: false)
            jump(to: target)
            rebuilding = false
        }
        refreshPictures()
    }
    private func jump(to index: Int) {
        guard pageFrames.indices.contains(index) else { return }
        let y = min(pageFrames[index].minY * zoomScale, max(0, contentSize.height - bounds.height))
        setContentOffset(CGPoint(x: contentOffset.x, y: y), animated: false)
    }
    private func refreshPictures() {
        guard bounds.width > 0, bounds.height > 0, !pageFrames.isEmpty else { return }
        let visible = visibleCanvasRect.insetBy(dx: 0, dy: -bounds.height / zoomScale)
        let wanted = Set(pageFrames.indices.filter { pageFrames[$0].intersects(visible) })
        for index in Array(pictures.keys) where !wanted.contains(index) || !urls.indices.contains(index) {
            pictures.removeValue(forKey: index)?.1.removeFromSuperview()
        }
        for index in wanted where urls.indices.contains(index) {
            let url = urls[index]
            let picture: UIImageView
            if let existing = pictures[index] {
                picture = existing.1
                if existing.0 != url {
                    picture.image = UIImage(contentsOfFile: url.path)
                    pictures[index] = (url, picture)
                }
            } else {
                picture = UIImageView(image: UIImage(contentsOfFile: url.path))
                picture.contentMode = .scaleAspectFit
                picture.isAccessibilityElement = true
                picture.accessibilityLabel = "\(index + 1) / \(urls.count)"
                canvas.addSubview(picture)
                pictures[index] = (url, picture)
            }
            picture.frame = pageFrames[index]
        }
    }
    private var visibleCanvasRect: CGRect {
        CGRect(x: contentOffset.x / zoomScale, y: contentOffset.y / zoomScale, width: bounds.width / zoomScale, height: bounds.height / zoomScale)
    }
    func viewForZooming(in scrollView: UIScrollView) -> UIView? { canvas }
    func scrollViewDidZoom(_ scrollView: UIScrollView) { refreshPictures() }
    func scrollViewDidScroll(_ scrollView: UIScrollView) {
        refreshPictures()
        // Layout/jumps originate from SwiftUI's current selection. Publish only
        // user scrolling, otherwise a refresh can feed a jump back into itself.
        guard !rebuilding, isTracking || isDragging || isDecelerating || isZooming,
              let index = pageFrames.firstIndex(where: { $0.maxY >= visibleCanvasRect.midY }), index != selectedIndex else { return }
        selectedIndex = index
        selectPage?(index)
    }
    private static func imageSize(_ url: URL) -> CGSize {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
              let values = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = values[kCGImagePropertyPixelWidth] as? CGFloat,
              let height = values[kCGImagePropertyPixelHeight] as? CGFloat, width > 0, height > 0 else { return CGSize(width: 1, height: 1) }
        let orientation = values[kCGImagePropertyOrientation] as? Int ?? 1
        return (5...8).contains(orientation) ? CGSize(width: height, height: width) : CGSize(width: width, height: height)
    }
}
