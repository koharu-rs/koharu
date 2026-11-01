import XCTest
@testable import KoharuPad

private final class ProviderHTTP: URLProtocol, @unchecked Sendable {
    struct Reply { let status: Int; let body: Any }
    static let lock = NSLock()
    static var handler: ((URLRequest) throws -> Reply)?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            Self.lock.lock(); let handler = Self.handler; Self.lock.unlock()
            let reply = try XCTUnwrap(handler)(request)
            let data = try JSONSerialization.data(withJSONObject: reply.body)
            let response = HTTPURLResponse(url: request.url!, statusCode: reply.status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
    static func setup(_ handler: @escaping (URLRequest) throws -> Reply) -> HostedTranslation {
        replaceHandler(handler)
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [Self.self]
        return HostedTranslation(session: URLSession(configuration: config))
    }
    static func replaceHandler(_ handler: @escaping (URLRequest) throws -> Reply) {
        lock.withLock { self.handler = handler }
    }
}

final class HostedProviderTests: XCTestCase {
    private let output = #"{"translations":[{"id":1,"text":"two"},{"id":0,"text":"one"}]}"#
    func testAllNineProviderContractsAndAuthentication() async throws {
        for provider in TranslationProvider.allCases.filter(\.hosted) {
            var config = HostedConfiguration(); config.workspace = "workspace-42"
            let endpoint = try HostedEndpoint(provider: provider, configuration: config)
            let model: String
            switch provider {
            case .openAI: model = "gpt-4.1-mini"
            case .anthropic: model = "claude-sonnet-4-6"
            case .deepSeek: model = "deepseek-flash"
            case .qwen: model = "qwen3.8-max"
            case .miniMax: model = "MiniMax-Text-01"
            case .gemini: model = "gemini-3-flash"
            case .grok: model = "grok-4"
            case .openRouter: model = "vendor/model"
            case .deepL: model = "quality_optimized"
            default: fatalError()
            }
            let answer = output
            let service = ProviderHTTP.setup { request in
                XCTAssertEqual(request.url?.scheme, "https")
                XCTAssertNil(URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "key" })
                switch provider {
                case .anthropic:
                    XCTAssertEqual(request.value(forHTTPHeaderField: "x-api-key"), "test-key")
                    XCTAssertEqual(request.value(forHTTPHeaderField: "anthropic-version"), "2023-06-01")
                case .gemini: XCTAssertEqual(request.value(forHTTPHeaderField: "x-goog-api-key"), "test-key")
                case .deepL: XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "DeepL-Auth-Key test-key")
                default: XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test-key")
                }
                let path = request.url!.path
                if request.httpMethod == "GET" {
                    if path.hasSuffix("/key") { return .init(status: 200, body: ["data": ["label": "test"]]) }
                    if path.hasSuffix("/usage") { return .init(status: 200, body: ["character_count": 0]) }
                    if path.hasSuffix("/languages") { return .init(status: 200, body: [["language": "EN-GB"]]) }
                    switch provider {
                    case .qwen:
                        XCTAssertEqual(path, "/api/v1/models")
                        return .init(status: 200, body: ["output": ["total": 2, "models": [["model": model, "features": ["structured-outputs"]], ["model": "qwen-turbo", "features": ["structured-outputs"]]]]])
                    case .gemini: return .init(status: 200, body: ["models": [["name": "models/" + model, "supportedGenerationMethods": ["generateContent"]], ["name": "models/gemini-image", "supportedGenerationMethods": ["generateContent"]]]])
                    case .grok: return .init(status: 200, body: ["models": [["id": model]]])
                    case .openRouter: return .init(status: 200, body: ["data": [["id": model, "supported_parameters": ["structured_outputs"], "architecture": ["output_modalities": ["text"]]], ["id": "text-only-no-schema"]]])
                    default: return .init(status: 200, body: ["data": [["id": model], ["id": "image-model"]]])
                    }
                }
                let body = try Self.jsonBody(request)
                switch provider {
                case .openAI, .grok:
                    XCTAssertTrue(path.hasSuffix("/responses")); XCTAssertEqual(body["store"] as? Bool, false)
                    let schema = try XCTUnwrap((body["text"] as? [String: Any])?["format"] as? [String: Any])
                    XCTAssertEqual(schema["type"] as? String, "json_schema"); XCTAssertEqual(schema["strict"] as? Bool, true)
                    return .init(status: 200, body: ["status": "completed", "output": [["type": "message", "content": [["type": "output_text", "text": answer]]]]])
                case .anthropic:
                    XCTAssertTrue(path.hasSuffix("/messages")); XCTAssertEqual((body["output_config"] as? [String: Any])?["format"] is [String: Any], true)
                    return .init(status: 200, body: ["stop_reason": "end_turn", "content": [["type": "text", "text": answer]]])
                case .deepSeek:
                    XCTAssertEqual(path, "/beta/chat/completions")
                    let tools = try XCTUnwrap(body["tools"] as? [[String: Any]])
                    XCTAssertEqual((tools[0]["function"] as? [String: Any])?["strict"] as? Bool, true)
                    return .init(status: 200, body: ["choices": [["finish_reason": "tool_calls", "message": ["tool_calls": [["function": ["name": "translation", "arguments": answer]]]]]]])
                case .gemini:
                    XCTAssertTrue(path.hasSuffix(":generateContent")); XCTAssertEqual((body["generationConfig"] as? [String: Any])?["responseJsonSchema"] is [String: Any], true)
                    return .init(status: 200, body: ["candidates": [["finishReason": "STOP", "content": ["parts": [["text": answer]]]]]])
                case .deepL:
                    XCTAssertEqual(body["target_lang"] as? String, "EN-GB")
                    XCTAssertEqual(body["text"] as? [String], ["一", "二"])
                    XCTAssertEqual(body["custom_instructions"] as? [String], ["Use casual speech."])
                    return .init(status: 200, body: ["translations": [["text": "one"], ["text": "two"]]])
                default:
                    let format = try XCTUnwrap(body["response_format"] as? [String: Any]); XCTAssertEqual(format["type"] as? String, "json_schema")
                    if provider == .openRouter { XCTAssertEqual((body["provider"] as? [String: Any])?["require_parameters"] as? Bool, true) }
                    return .init(status: 200, body: ["choices": [["finish_reason": "stop", "message": ["content": answer]]]])
                }
            }
            let payload = TranslationPayload(target_language: "en-GB", segments: [.init(id: 0, text: "一"), .init(id: 1, text: "二")])
            let result = try await service.translate(endpoint: endpoint, key: "test-key", model: model, system: "shared system", user: String(decoding: JSONEncoder().encode(payload), as: UTF8.self), count: 2, instructions: "Use casual speech.")
            XCTAssertEqual(try TranslationContract.decode(result, count: 2), ["one", "two"], provider.name)
        }
    }
    func testOpenRouterRequiresAuthenticatedKeyBeforePublicCatalog() async throws {
        let service = ProviderHTTP.setup { request in
            XCTAssertTrue(request.url!.path.hasSuffix("/key"))
            return .init(status: 401, body: ["error": ["message": "Never echo test-key"]])
        }
        do { _ = try await service.models(endpoint: HostedEndpoint(provider: .openRouter, configuration: .init()), key: "test-key"); XCTFail() }
        catch { XCTAssertFalse(error.localizedDescription.contains("test-key")) }
    }
    func testPaginationAndRevokedKeyInvalidateCatalog() async throws {
        let endpoint = try HostedEndpoint(provider: .anthropic, configuration: .init())
        var requests = 0
        let service = ProviderHTTP.setup { request in
            requests += 1
            let after = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "after_id" }?.value
            return .init(status: 200, body: after == nil ? ["data": [["id": "claude-a"]], "has_more": true, "last_id": "claude-a"] : ["data": [["id": "claude-b"]], "has_more": false])
        }
        let models = try await service.models(endpoint: endpoint, key: "test-key")
        XCTAssertEqual(models.map(\.id), ["claude-a", "claude-b"]); XCTAssertEqual(requests, 2)
        ProviderHTTP.replaceHandler { _ in .init(status: 403, body: ["error": "revoked"]) }
        do { _ = try await service.models(endpoint: endpoint, key: "test-key", refresh: true); XCTFail() } catch {}
        do { _ = try await service.models(endpoint: endpoint, key: "test-key"); XCTFail("Revoked key reused cached validation") } catch {}
    }
    func testIncompleteOrMalformedTranslationNeverPublishes() async throws {
        for (finish, answer) in [("length", output), ("stop", #"{"translations":[{"id":0,"text":"one"},{"id":0,"text":"two"}]}"#)] {
            let service = ProviderHTTP.setup { request in
                if request.httpMethod == "GET" { return .init(status: 200, body: ["data": [["id": "MiniMax-Text-01"]]]) }
                return .init(status: 200, body: ["choices": [["finish_reason": finish, "message": ["content": answer]]]])
            }
            do { _ = try await service.translate(endpoint: HostedEndpoint(provider: .miniMax, configuration: .init()), key: "test-key", model: "MiniMax-Text-01", system: "shared", user: "{}", count: 2); XCTFail() } catch {}
        }
    }
    func testCredentialBoundariesAndDeepLFreeEndpoint() async throws {
        var config = HostedConfiguration(); config.workspace = "my-workspace"
        let singapore = try HostedEndpoint(provider: .qwen, configuration: config)
        config.region = "china"
        let china = try HostedEndpoint(provider: .qwen, configuration: config)
        XCTAssertNotEqual(singapore.credentialAccount, china.credentialAccount)
        config.workspace = "attacker.example/path"
        XCTAssertThrowsError(try HostedEndpoint(provider: .qwen, configuration: config))
        let deepL = try HostedEndpoint(provider: .deepL, configuration: .init())
        XCTAssertEqual(deepL.base(key: "test:fx").host, "api-free.deepl.com")
        XCTAssertEqual(HostedTranslation.deepLTarget("zh-TW"), "ZH-HANT")
        let service = ProviderHTTP.setup { _ in .init(status: 200, body: ["base_resp": ["status_code": 1008, "status_msg": "test-key"]]) }
        do { _ = try await service.models(endpoint: HostedEndpoint(provider: .miniMax, configuration: .init()), key: "test-key"); XCTFail() }
        catch { XCTAssertFalse(error.localizedDescription.contains("test-key")) }
    }
    private static func jsonBody(_ request: URLRequest) throws -> [String: Any] {
        var data = request.httpBody ?? Data()
        if let stream = request.httpBodyStream {
            stream.open(); defer { stream.close() }; var buffer = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable { let count = stream.read(&buffer, maxLength: buffer.count); if count <= 0 { break }; data.append(contentsOf: buffer.prefix(count)) }
        }
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }
}
