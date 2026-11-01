import Foundation
import CoreGraphics
import CryptoKit

struct TargetLanguage: Codable, Identifiable {
    var tag: String
    var name: String
    var id: String { tag }
    var displayName: String {
        let language = String(tag.split(separator: "-")[0])
        let identifier: String
        if tag == "zh-CN" { identifier = "zh-Hans" }
        else if tag == "zh-TW" { identifier = "zh-Hant" }
        else if Self.all.filter({ $0.tag.split(separator: "-").first == Substring(language) }).count > 1 { identifier = tag }
        else { identifier = language }
        return Locale.current.localizedString(forIdentifier: identifier) ?? name
    }
    static let all: [TargetLanguage] = {
        guard let url = Bundle.main.url(forResource: "languages", withExtension: "json"),
              let data = try? Data(contentsOf: url), let result = try? JSONDecoder().decode([Self].self, from: data) else { return [] }
        return result
    }()
}

enum TranslationProvider: String, Codable, CaseIterable { case lmStudio, local }
struct Preferences: Codable, Equatable {
    var provider: TranslationProvider = .lmStudio
    var server = "http://localhost:1234"
    var remoteModel = ""
    var localModel: UUID?
    var target = "vi-VN"
    var instructions = ""
    var contextSize = 4096
    var inpaint = false
    var ocrModel: String?
    var cleanupModel: String?
}
struct LocalModel: Codable, Identifiable {
    var id: UUID
    var name: String
    var bytes: Int64
    var source: String?
    var filename: String { id.uuidString + ".gguf" }
}
struct TextSegment: Codable, Identifiable {
    var id: Int
    var source: String
    var text: String?
    var rect: CGRect
    var edited = false
}
struct MangaPage: Codable, Identifiable {
    var id: UUID = UUID()
    var source: String
    var original: String
    var etag: String?
    var modified: String?
    var segments: [TextSegment] = []
    var ocrSignature: String?
    var rendered: String?
    var fingerprint: String?
}
struct Chapter: Codable, Identifiable {
    var id: UUID = UUID()
    var title: String
    var sourceURL: String
    var created = Date()
    var target: String
    var pages: [MangaPage]
    var readPage = 0
}
struct SourceChapter: Codable {
    var url: String
    var title: String
    var images: [String]
    func validated() throws -> Self {
        guard Self.isSource(url), !images.isEmpty, images.count <= 500,
              Set(images).count == images.count, images.allSatisfy(Self.isImage) else {
            throw AppError.message(L("Open a Rawkuma chapter containing manga pages."))
        }
        return self
    }
    static func isSource(_ value: String) -> Bool {
        guard let url = URL(string: value) else { return false }
        return url.scheme == "https" && ["rawkuma.net", "www.rawkuma.net"].contains(url.host ?? "") && url.port == nil && url.user == nil && url.password == nil
    }
    static func isImage(_ value: String) -> Bool {
        guard let url = URL(string: value) else { return false }
        return url.scheme == "https" && url.host == "cdn.rawkuma.com" && url.port == nil && url.user == nil && url.password == nil && url.path != "/"
    }
}
enum AppError: LocalizedError {
    case message(String)
    var errorDescription: String? { switch self { case .message(let value): return value } }
}
func L(_ key: String) -> String { NSLocalizedString(key, comment: "") }
func digest(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }

struct LibraryDisk {
    let root: URL
    init(root: URL? = nil) throws {
        self.root = try root ?? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true).appendingPathComponent("Koharu", isDirectory: true)
        for directory in ["chapters", "models", "downloads"] {
            try FileManager.default.createDirectory(at: self.root.appendingPathComponent(directory), withIntermediateDirectories: true)
        }
    }
    var modelsURL: URL { root.appendingPathComponent("models", isDirectory: true) }
    func chapterURL(_ id: UUID) -> URL { root.appendingPathComponent("chapters/" + id.uuidString, isDirectory: true) }
    func pageURL(_ chapter: UUID, _ file: String) -> URL {
        chapterURL(chapter).appendingPathComponent(URL(fileURLWithPath: file).lastPathComponent)
    }
    func save<T: Encodable>(_ value: T, to url: URL) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try JSONEncoder().encode(value).write(to: url, options: .atomic)
    }
    func load<T: Decodable>(_ type: T.Type, _ url: URL) throws -> T { try JSONDecoder().decode(type, from: Data(contentsOf: url)) }
    func chapters() throws -> [Chapter] {
        try FileManager.default.contentsOfDirectory(at: root.appendingPathComponent("chapters"), includingPropertiesForKeys: nil)
            .filter { FileManager.default.fileExists(atPath: $0.appendingPathComponent("chapter.json").path) }
            .map { try load(Chapter.self, $0.appendingPathComponent("chapter.json")) }
            .sorted { $0.created > $1.created }
    }
    func save(_ chapter: Chapter, recordUndo: Bool = true) throws {
        let file = chapterURL(chapter.id).appendingPathComponent("chapter.json")
        if recordUndo && FileManager.default.fileExists(atPath: file.path) {
            try Data(contentsOf: file).write(to: chapterURL(chapter.id).appendingPathComponent("undo.json"), options: .atomic)
            try? FileManager.default.removeItem(at: chapterURL(chapter.id).appendingPathComponent("redo.json"))
        }
        try save(chapter, to: file)
    }
    func restore(_ id: UUID, redo: Bool) throws -> Chapter {
        let folder = chapterURL(id)
        let from = folder.appendingPathComponent(redo ? "redo.json" : "undo.json")
        let to = folder.appendingPathComponent(redo ? "undo.json" : "redo.json")
        let chapter = try load(Chapter.self, from)
        guard chapter.id == id else { throw AppError.message(L("Invalid chapter history")) }
        try Data(contentsOf: folder.appendingPathComponent("chapter.json")).write(to: to, options: .atomic)
        try save(chapter, to: folder.appendingPathComponent("chapter.json"))
        try? FileManager.default.removeItem(at: from)
        return chapter
    }
    func clearDerived(_ id: UUID) throws {
        var chapter = try load(Chapter.self, chapterURL(id).appendingPathComponent("chapter.json"))
        for file in try FileManager.default.contentsOfDirectory(at: chapterURL(id), includingPropertiesForKeys: nil) where file.pathExtension == "png" {
            try FileManager.default.removeItem(at: file)
        }
        for i in chapter.pages.indices {
            chapter.pages[i].rendered = nil
            chapter.pages[i].fingerprint = nil
        }
        try save(chapter)
    }
}
