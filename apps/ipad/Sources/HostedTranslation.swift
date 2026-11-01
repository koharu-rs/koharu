import Foundation
import CryptoKit

struct HostedModel: Identifiable, Equatable {
    let id: String
    let name: String
    var displayName: String {
        switch id {
        case "quality_optimized": return L("Prioritize quality")
        case "latency_optimized": return L("Prioritize speed")
        case "prefer_quality_optimized": return L("Prefer quality, with speed fallback")
        default: return name == id ? id : "\(name) · \(id)"
        }
    }
}

// Only provider-owned HTTPS origins can receive credentials. Workspace and region
// participate in the Keychain account, so switching either cannot reuse a key.
struct HostedEndpoint: Equatable {
    let provider: TranslationProvider
    let root: URL
    var credentialAccount: String { "hosted:\(provider.rawValue):\(root.absoluteString)" }
    init(provider: TranslationProvider, configuration: HostedConfiguration) throws {
        self.provider = provider
        let address: String
        switch provider {
        case .openAI: address = "https://api.openai.com/v1"
        case .anthropic: address = "https://api.anthropic.com/v1"
        case .deepSeek: address = "https://api.deepseek.com"
        case .gemini: address = "https://generativelanguage.googleapis.com/v1beta"
        case .grok: address = "https://api.x.ai/v1"
        case .openRouter: address = "https://openrouter.ai/api/v1"
        case .deepL: address = "https://api.deepl.com/v2"
        case .miniMax:
            guard ["international", "china"].contains(configuration.region) else { throw Self.invalidSettings }
            address = configuration.region == "china" ? "https://api.minimaxi.com/v1" : "https://api.minimax.io/v1"
        case .qwen:
            let regions = ["international": "ap-southeast-1", "china": "cn-beijing", "us": "us-east-1", "hongkong": "cn-hongkong", "japan": "ap-northeast-1", "germany": "eu-central-1"]
            let workspace = configuration.workspace.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let region = regions[configuration.region], !workspace.isEmpty, workspace.utf8.count <= 100,
                  workspace.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }) else { throw Self.invalidSettings }
            address = "https://\(workspace).\(region).maas.aliyuncs.com/compatible-mode/v1"
        default: throw Self.invalidSettings
        }
        root = URL(string: address)!
    }
    static var invalidSettings: AppError { .message(L("Check the provider region and workspace ID")) }
    func base(key: String) -> URL {
        provider == .deepL && key.hasSuffix(":fx") ? URL(string: "https://api-free.deepl.com/v2")! : root
    }
}

actor HostedTranslation {
    private struct Catalog { let date: Date; let models: [HostedModel] }
    private var catalogs: [String: Catalog] = [:]
    private let session: URLSession
    init(session: URLSession? = nil) {
        if let session { self.session = session }
        else {
            let config = URLSessionConfiguration.ephemeral
            config.timeoutIntervalForRequest = 180
            config.timeoutIntervalForResource = 240
            config.httpCookieStorage = nil
            config.urlCache = nil
            self.session = URLSession(configuration: config, delegate: RedirectBoundary { _ in false }, delegateQueue: nil)
        }
    }
    deinit { session.invalidateAndCancel() }
    private func cacheID(_ endpoint: HostedEndpoint, _ key: String) -> String {
        SHA256.hash(data: Data((endpoint.credentialAccount + "\u{0}" + key).utf8)).map { String(format: "%02x", $0) }.joined()
    }
    private func request(_ endpoint: HostedEndpoint, key: String, path: String, query: [URLQueryItem] = [], body: [String: Any]? = nil) throws -> URLRequest {
        guard !key.isEmpty, key.utf8.count <= 4096, !key.contains(where: { $0.isNewline || $0.asciiValue == 0 }) else { throw AppError.message(L("Enter an API key")) }
        var components = URLComponents(url: endpoint.base(key: key).appendingPathComponent(path), resolvingAgainstBaseURL: false)!
        if !query.isEmpty { components.queryItems = query }
        var request = URLRequest(url: components.url!)
        request.httpMethod = body == nil ? "GET" : "POST"
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        switch endpoint.provider {
        case .anthropic:
            request.setValue(key, forHTTPHeaderField: "x-api-key")
            request.setValue("2023-06-01", forHTTPHeaderField: "anthropic-version")
        case .gemini: request.setValue(key, forHTTPHeaderField: "x-goog-api-key")
        case .deepL: request.setValue("DeepL-Auth-Key " + key, forHTTPHeaderField: "Authorization")
        default: request.setValue("Bearer " + key, forHTTPHeaderField: "Authorization")
        }
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        return request
    }
    private func send(_ request: URLRequest, provider: TranslationProvider) async throws -> Any {
        let (data, response) = try await session.data(for: request)
        try Task.checkCancellation()
        guard let response = response as? HTTPURLResponse else { throw invalidResponse }
        switch response.statusCode {
        case 200..<300: break
        case 401, 403: throw AppError.message(provider.name + ": " + L("Authentication rejected. Check the key, region and API permissions."))
        case 402, 456: throw AppError.message(provider.name + ": " + L("API quota or credit limit reached"))
        case 429: throw AppError.message(provider.name + ": " + L("API rate or quota limit reached. Try again later."))
        default: throw AppError.message(provider.name + " HTTP \(response.statusCode). " + L("Check model access, supported settings and service status."))
        }
        guard data.count <= 8_388_608, let value = try? JSONSerialization.jsonObject(with: data) else { throw invalidResponse }
        if let root = value as? [String: Any], let status = (root["base_resp"] as? [String: Any])?["status_code"] as? Int, status != 0 {
            // MiniMax can return an application error with HTTP 200. Never echo a
            // server-supplied message, which may contain a submitted credential.
            throw AppError.message(provider.name + " API \(status). " + L("Check the key, API credit and model access."))
        }
        return value
    }
    private var invalidResponse: AppError { .message(L("The provider returned an invalid or incomplete response")) }

    func models(endpoint: HostedEndpoint, key: String, refresh: Bool = false) async throws -> [HostedModel] {
        let key = key.trimmingCharacters(in: .whitespacesAndNewlines)
        let id = cacheID(endpoint, key)
        if !refresh, let cached = catalogs[id], Date().timeIntervalSince(cached.date) < 300 { return cached.models }
        // A forced validation failure must revoke any earlier successful cache.
        catalogs.removeValue(forKey: id)
        var models: [HostedModel] = []
        switch endpoint.provider {
        case .deepL:
            guard let usage = try await send(request(endpoint, key: key, path: "usage"), provider: endpoint.provider) as? [String: Any],
                  usage["character_count"] != nil || usage["products"] != nil else { throw invalidResponse }
            // DeepL publishes translation modes, not an LLM model-list endpoint.
            models = ["quality_optimized", "latency_optimized", "prefer_quality_optimized"].map { HostedModel(id: $0, name: $0) }
        case .anthropic, .gemini:
            var cursor: String?; var seen = Set<String>()
            for page in 0..<30 {
                var query = [URLQueryItem(name: endpoint.provider == .gemini ? "pageSize" : "limit", value: "1000")]
                if let cursor { query.append(URLQueryItem(name: endpoint.provider == .gemini ? "pageToken" : "after_id", value: cursor)) }
                guard let root = try await send(request(endpoint, key: key, path: "models", query: query), provider: endpoint.provider) as? [String: Any],
                      let items = root[endpoint.provider == .gemini ? "models" : "data"] as? [[String: Any]] else { throw invalidResponse }
                for item in items {
                    if endpoint.provider == .gemini {
                        guard let name = item["name"] as? String, name.hasPrefix("models/"),
                              (item["supportedGenerationMethods"] as? [String])?.contains("generateContent") == true else { continue }
                        let model = String(name.dropFirst(7))
                        if Self.supportsGemini(model) { models.append(.init(id: model, name: item["displayName"] as? String ?? model)) }
                    } else if let model = item["id"] as? String { models.append(.init(id: model, name: item["display_name"] as? String ?? model)) }
                }
                cursor = endpoint.provider == .gemini ? root["nextPageToken"] as? String : (root["has_more"] as? Bool == true ? root["last_id"] as? String : nil)
                if cursor?.isEmpty != false {
                    guard root["has_more"] as? Bool != true else { throw invalidResponse }
                    break
                }
                guard seen.insert(cursor!).inserted, page < 29 else { throw invalidResponse }
            }
        case .qwen:
            // Model Studio's authenticated catalog includes capability metadata.
            var components = URLComponents(url: endpoint.root, resolvingAgainstBaseURL: false)!
            components.path = "/api/v1/models"
            for page in 1...30 {
                var req = try request(endpoint, key: key, path: "models")
                components.queryItems = [URLQueryItem(name: "providers", value: "qwen"), URLQueryItem(name: "capabilities", value: "TG"), URLQueryItem(name: "page_size", value: "100"), URLQueryItem(name: "page_no", value: String(page))]
                req.url = components.url
                guard let root = try await send(req, provider: .qwen) as? [String: Any], let output = root["output"] as? [String: Any],
                      let items = output["models"] as? [[String: Any]], let total = output["total"] as? Int, (0...3000).contains(total) else { throw invalidResponse }
                for item in items {
                    guard let model = item["model"] as? String, Self.supportsQwen(model), (item["features"] as? [String])?.contains("structured-outputs") == true else { continue }
                    models.append(.init(id: model, name: item["name"] as? String ?? model))
                }
                if page * 100 >= total { break }
                guard !items.isEmpty, page < 30 else { throw invalidResponse }
            }
        default:
            if endpoint.provider == .openRouter {
                // /models is public and cannot establish whether a key is valid.
                guard let root = try await send(request(endpoint, key: key, path: "key"), provider: .openRouter) as? [String: Any], root["data"] is [String: Any] else { throw invalidResponse }
            }
            let path = endpoint.provider == .grok ? "language-models" : "models"
            guard let root = try await send(request(endpoint, key: key, path: path), provider: endpoint.provider) as? [String: Any],
                  let items = root[endpoint.provider == .grok ? "models" : "data"] as? [[String: Any]], items.count <= 10000 else { throw invalidResponse }
            for item in items {
                guard let model = item["id"] as? String else { continue }
                switch endpoint.provider {
                case .openAI: guard Self.supportsOpenAI(model) else { continue }
                case .miniMax: guard model == "MiniMax-Text-01" else { continue }
                case .openRouter:
                    guard (item["supported_parameters"] as? [String])?.contains("structured_outputs") == true,
                          ((item["architecture"] as? [String: Any])?["output_modalities"] as? [String])?.contains("text") == true else { continue }
                default: break
                }
                models.append(.init(id: model, name: item["name"] as? String ?? model))
            }
        }
        var seen = Set<String>()
        models = models.filter { !$0.id.isEmpty && $0.id.utf8.count <= 300 && $0.id.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || "-_.:/".contains($0)) }) && seen.insert($0.id).inserted }.sorted { $0.id < $1.id }
        guard !models.isEmpty else { throw AppError.message(L("Key accepted, but no supported translation models are available")) }
        catalogs[id] = Catalog(date: Date(), models: models)
        return models
    }
    static func supportsOpenAI(_ id: String) -> Bool {
        let supported = ["gpt-4o", "gpt-4.1", "gpt-5", "gpt-6", "o3", "o4"].contains { id.hasPrefix($0) }
        return supported && !["audio", "codex", "deep-research", "image", "realtime", "search", "transcribe", "tts", "chat-latest"].contains { id.contains($0) }
    }
    static func supportsGemini(_ id: String) -> Bool {
        id.hasPrefix("gemini-") && !["1.0", "1.5", "2.0", "computer-use", "deep-research", "embedding", "image", "live", "audio", "omni", "robotics", "tts"].contains { id.contains($0) }
    }
    static func supportsQwen(_ id: String) -> Bool {
        // The current official JSON Schema support list. JSON-only models are not
        // silently substituted for native schema enforcement.
        ["qwen3.7-plus", "qwen3.7-flash", "qwen3.7-max", "qwen3.8-max", "qwen3.8-flash"].contains { id == $0 || id.hasPrefix($0 + "-") }
    }

    func translate(endpoint: HostedEndpoint, key: String, model: String, system: String, user: String, count: Int, instructions: String = "") async throws -> Data {
        let key = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard (1...12).contains(count) else { throw invalidResponse }
        let available = try await models(endpoint: endpoint, key: key)
        guard available.contains(where: { $0.id == model }) else { throw AppError.message(L("Validate the API key and select a supported model in Settings")) }
        if endpoint.provider == .deepL { return try await deepL(endpoint, key: key, model: model, user: user, instructions: instructions, count: count) }
        let schema = TranslationContract.hostedSchema(count: count)
        let format: [String: Any] = ["type": "json_schema", "json_schema": ["name": "translation", "strict": true, "schema": schema]]
        let messages = [["role": "system", "content": system], ["role": "user", "content": user]]
        var path = "chat/completions"
        var body: [String: Any] = ["model": model, "messages": messages, "response_format": format, "max_tokens": 4096]
        switch endpoint.provider {
        case .openAI, .grok:
            path = "responses"
            body = ["model": model, "instructions": system, "input": user, "store": false, "max_output_tokens": 8192,
                    "text": ["format": ["type": "json_schema", "name": "translation", "strict": true, "schema": schema]]]
        case .anthropic:
            path = "messages"
            body = ["model": model, "system": system, "messages": [["role": "user", "content": user]], "max_tokens": 4096,
                    "output_config": ["format": ["type": "json_schema", "schema": schema]]]
        case .deepSeek:
            path = "beta/chat/completions"
            body = ["model": model, "messages": messages, "max_tokens": 4096, "thinking": ["type": "disabled"],
                    "tools": [["type": "function", "function": ["name": "translation", "description": "Return the translated segments.", "strict": true, "parameters": schema]]],
                    "tool_choice": ["type": "function", "function": ["name": "translation"]]]
        case .qwen:
            // Newer Qwen 3.7/3.8 models always think; unsupported disabling flags
            // must not be sent to them. Budget includes reasoning and final text.
            body["max_tokens"] = nil; body["max_completion_tokens"] = 8192
        case .miniMax:
            path = "text/chatcompletion_v2"
            body["max_tokens"] = nil; body["max_completion_tokens"] = 4096
        case .gemini:
            path = "models/\(model):generateContent"
            body = ["systemInstruction": ["parts": [["text": system]]], "contents": [["role": "user", "parts": [["text": user]]]],
                    "generationConfig": ["responseMimeType": "application/json", "responseJsonSchema": schema, "maxOutputTokens": 8192]]
        case .openRouter:
            body["provider"] = ["require_parameters": true]
        default: throw HostedEndpoint.invalidSettings
        }
        guard let root = try await send(request(endpoint, key: key, path: path, body: body), provider: endpoint.provider) as? [String: Any] else { throw invalidResponse }
        let text: String
        switch endpoint.provider {
        case .openAI, .grok:
            guard root["status"] as? String == "completed", let output = root["output"] as? [[String: Any]] else { throw invalidResponse }
            let messages = output.filter { $0["type"] as? String == "message" }
            guard messages.count == 1, let content = messages.first?["content"] as? [[String: Any]], content.count == 1,
                  content[0]["type"] as? String == "output_text", let answer = content[0]["text"] as? String else { throw invalidResponse }
            text = answer
        case .anthropic:
            guard root["stop_reason"] as? String == "end_turn", let content = root["content"] as? [[String: Any]],
                  content.count == 1, content[0]["type"] as? String == "text", let answer = content[0]["text"] as? String else { throw invalidResponse }
            text = answer
        case .gemini:
            guard let candidates = root["candidates"] as? [[String: Any]], candidates.count == 1,
                  candidates[0]["finishReason"] as? String == "STOP", let content = candidates[0]["content"] as? [String: Any],
                  let parts = content["parts"] as? [[String: Any]] else { throw invalidResponse }
            let final = parts.filter { $0["thought"] as? Bool != true }
            guard final.count == 1, let answer = final[0]["text"] as? String else { throw invalidResponse }
            text = answer
        default:
            guard let choices = root["choices"] as? [[String: Any]], choices.count == 1, let message = choices[0]["message"] as? [String: Any] else { throw invalidResponse }
            if endpoint.provider == .deepSeek {
                guard choices[0]["finish_reason"] as? String == "tool_calls", let calls = message["tool_calls"] as? [[String: Any]], calls.count == 1,
                      let function = calls[0]["function"] as? [String: Any], function["name"] as? String == "translation", let answer = function["arguments"] as? String else { throw invalidResponse }
                text = answer
            } else {
                guard choices[0]["finish_reason"] as? String == "stop", let answer = message["content"] as? String else { throw invalidResponse }
                text = answer
            }
        }
        let data = Data(text.utf8)
        _ = try TranslationContract.decode(data, count: count)
        return data
    }
    private func deepL(_ endpoint: HostedEndpoint, key: String, model: String, user: String, instructions: String, count: Int) async throws -> Data {
        let payload = try JSONDecoder().decode(TranslationPayload.self, from: Data(user.utf8))
        guard let target = Self.deepLTarget(payload.target_language) else { throw AppError.message(L("DeepL does not support the selected target language")) }
        guard let languages = try await send(request(endpoint, key: key, path: "languages", query: [URLQueryItem(name: "type", value: "target")]), provider: .deepL) as? [[String: Any]],
              languages.contains(where: { ($0["language"] as? String)?.uppercased() == target }) else { throw AppError.message(L("DeepL does not support the selected target language")) }
        var body: [String: Any] = ["text": payload.segments.map(\.text), "source_lang": "JA", "target_lang": target, "model_type": model]
        // Machine translation uses its native typed text array. It never receives
        // the LLM JSON prompt as text to translate.
        if !instructions.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            guard instructions.count <= 300 else { throw AppError.message(L("DeepL additional instructions must be at most 300 characters")) }
            body["custom_instructions"] = [instructions]
        }
        guard let root = try await send(request(endpoint, key: key, path: "translate", body: body), provider: .deepL) as? [String: Any],
              let items = root["translations"] as? [[String: Any]], items.count == count else { throw invalidResponse }
        let output = try items.enumerated().map { index, item -> [String: Any] in
            guard let text = item["text"] as? String else { throw invalidResponse }
            return ["id": payload.segments[index].id, "text": text]
        }
        return try JSONSerialization.data(withJSONObject: ["translations": output])
    }
    static func deepLTarget(_ tag: String) -> String? {
        switch tag {
        case "en-US": return "EN-US"
        case "en-GB": return "EN-GB"
        case "pt-BR": return "PT-BR"
        case "pt-PT": return "PT-PT"
        case "zh-CN": return "ZH-HANS"
        case "zh-TW": return "ZH-HANT"
        case "yue-HK", "fil-PH", "km-KH", "my-MM", "fa-IR", "bo-CN", "mn-MN", "ug-CN": return nil
        default: return tag.split(separator: "-").first.map { $0.uppercased() }
        }
    }
}
