import Foundation
import Observation

// UI language is independent of project processing preferences/fingerprints.
enum InterfaceLanguage: String, CaseIterable, Identifiable {
    case system, englishUS = "en-US", englishUK = "en-GB", french = "fr", arabic = "ar", spanish = "es", portuguese = "pt", german = "de", korean = "ko", japanese = "ja", traditionalChinese = "zh-Hant", simplifiedChinese = "zh-Hans", vietnamese = "vi"
    var id: String { rawValue }
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
    var identifier: String { language == .system ? Bundle.main.preferredLocalizations.first ?? "en" : language.rawValue }
    var locale: Locale { Locale(identifier: identifier) }
    var isRightToLeft: Bool { locale.language.characterDirection == .rightToLeft }
    func text(_ key: String) -> String {
        guard let path = Bundle.main.path(forResource: identifier, ofType: "lproj"), let bundle = Bundle(path: path) else { return key }
        return bundle.localizedString(forKey: key, value: key, table: nil)
    }
}
