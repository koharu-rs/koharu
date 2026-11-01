import Foundation
import Observation

// UI language is independent of project processing preferences/fingerprints.
enum InterfaceLanguage: String, CaseIterable, Identifiable {
    case system, englishUS = "en-US", englishUK = "en-GB", french = "fr", arabic = "ar", spanish = "es", portuguese = "pt", german = "de", korean = "ko", japanese = "ja", traditionalChinese = "zh-Hant", simplifiedChinese = "zh-Hans", vietnamese = "vi"
    var id: String { rawValue }
    static func preferred(for languages: [String]) -> Self {
        let supported = allCases.filter { $0 != .system }.map(\.rawValue)
        let identifier = Bundle.preferredLocalizations(from: supported, forPreferences: languages).first ?? englishUS.rawValue
        return Self(rawValue: identifier) ?? .englishUS
    }
    var translationTarget: String {
        switch self {
        case .system: return Self.preferred(for: Locale.preferredLanguages).translationTarget
        case .englishUS: return "en-US"
        case .englishUK: return "en-GB"
        case .french: return "fr-FR"
        case .arabic: return "ar-SA"
        case .spanish: return "es-ES"
        case .portuguese: return "pt-PT"
        case .german: return "de-DE"
        case .korean: return "ko-KR"
        case .japanese: return "ja-JP"
        case .traditionalChinese: return "zh-TW"
        case .simplifiedChinese: return "zh-CN"
        case .vietnamese: return "vi-VN"
        }
    }
    var name: String {
        switch self {
        case .system: return L("System default")
        case .englishUS: return "English (US)"
        case .englishUK: return "English (UK)"
        case .french: return "Français"
        case .arabic: return "العربية"
        case .spanish: return "Español"
        case .portuguese: return "Português"
        case .german: return "Deutsch"
        case .korean: return "한국어"
        case .japanese: return "日本語"
        case .traditionalChinese: return "繁體中文"
        case .simplifiedChinese: return "简体中文"
        case .vietnamese: return "Tiếng Việt"
        }
    }
}

@Observable final class AppLocalization {
    static let shared = AppLocalization()
    var language: InterfaceLanguage {
        didSet { UserDefaults.standard.set(language.rawValue, forKey: "interfaceLanguage") }
    }
    private init() { language = InterfaceLanguage(rawValue: UserDefaults.standard.string(forKey: "interfaceLanguage") ?? "system") ?? .system }
    var resolvedLanguage: InterfaceLanguage { language == .system ? .preferred(for: Locale.preferredLanguages) : language }
    var identifier: String { resolvedLanguage.rawValue }
    var locale: Locale { Locale(identifier: identifier) }
    var isRightToLeft: Bool { locale.language.characterDirection == .rightToLeft }
    func text(_ key: String) -> String {
        guard let path = Bundle.main.path(forResource: identifier, ofType: "lproj"), let bundle = Bundle(path: path) else { return key }
        return bundle.localizedString(forKey: key, value: key, table: nil)
    }
}
