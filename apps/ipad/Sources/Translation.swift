import Foundation
import os
#if canImport(llama)
import llama
#endif

struct TranslationPayload: Codable {
    struct Segment: Codable { var id: Int; var text: String }
    var source_language: String? = "ja-JP"
    var target_language: String
    var context: [String] = []
    var segments: [Segment]
}
enum TranslationContract {
    static func prompts(_ texts: [String], preferences: Preferences) throws -> (String, String) {
        guard let resource = Bundle.main.url(forResource: "translation-system", withExtension: "txt") else { throw AppError.message(L("Translation prompt resource is missing")) }
        let target = TargetLanguage.all.first { $0.tag == preferences.target }?.name ?? preferences.target
        var system = try String(contentsOf: resource, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "{source}", with: "Japanese")
            .replacingOccurrences(of: "{target}", with: target)
        if !preferences.instructions.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { system += "\n\nAdditional instructions:\n" + preferences.instructions }
        let payload = TranslationPayload(target_language: preferences.target, segments: texts.enumerated().map { .init(id: $0.offset, text: $0.element) })
        return (system, String(decoding: try JSONEncoder().encode(payload), as: UTF8.self))
    }
    static func schema(count: Int) -> [String: Any] {
        ["type": "object", "additionalProperties": false, "required": ["translations"], "properties": ["translations": ["type": "array", "minItems": count, "maxItems": count, "items": ["type": "object", "additionalProperties": false, "required": ["id", "text"], "properties": ["id": ["type": "integer", "minimum": 0, "maximum": max(0, count - 1)], "text": ["type": "string"]]]]]]
    }
    static func decode(_ data: Data, count: Int) throws -> [String] {
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any], Set(root.keys) == ["translations"], let values = root["translations"] as? [[String: Any]], values.count == count else { throw AppError.message(L("The model returned invalid translation JSON")) }
        var result = [String?](repeating: nil, count: count)
        for value in values {
            guard Set(value.keys) == ["id", "text"], let number = value["id"] as? NSNumber,
                  CFGetTypeID(number) != CFBooleanGetTypeID(), number.doubleValue == Double(number.intValue),
                  (0..<count).contains(number.intValue), result[number.intValue] == nil, let text = value["text"] as? String else { throw AppError.message(L("The model returned missing, duplicate or invalid segment IDs")) }
            result[number.intValue] = text
        }
        return try result.map { guard let value = $0 else { throw AppError.message(L("The model omitted a translation")) }; return value }
    }
    static func grammar(count: Int) -> String {
        let items = (0..<count).map { "item-\($0)" }.joined(separator: #" ws "," ws "#)
        var lines = [#"root ::= ws "{" ws "\"translations\"" ws ":" ws "[" ws "# + items + #" ws "]" ws "}" ws"#]
        for i in 0..<count { lines.append(#"item-\#(i) ::= "{" ws "\"id\"" ws ":" ws "\#(i)" ws "," ws "\"text\"" ws ":" ws string ws "}""#) }
        return lines.joined(separator: "\n") + "\n" + #"""
    string ::= "\"" char* "\""
    char ::= [^"\\\x00-\x1F] | "\\" (["\\/bfnrt] | "u" hex hex hex hex)
    hex ::= [0-9a-fA-F]
    ws ::= [ \t\n\r]*
    """#
    }
}

actor TranslationEngine {
    let local = LocalInference()
    func translate(_ texts: [String], config: Preferences, model: URL?) async throws -> [String] {
        var result: [String] = []
        // Bound both the prompt and result on small device contexts. A failed batch
        // never publishes a partial translated page.
        for start in stride(from: 0, to: texts.count, by: 12) {
            try Task.checkCancellation()
            let batch = Array(texts[start..<min(start + 12, texts.count)])
            let (system, user) = try TranslationContract.prompts(batch, preferences: config)
            let data: Data
            switch config.provider {
            case .local:
                guard let model else { throw AppError.message(L("Download or import a GGUF model and select it in Settings")) }
                do { data = try await local.generate(model: model, context: config.contextSize, system: system, user: user) }
                catch { await local.unload(); throw error }
            case .lmStudio:
                guard !config.remoteModel.isEmpty else { throw AppError.message(L("Select an LM Studio model in Settings")) }
                let base = try LMStudio.baseURL(config.server)
                var request = URLRequest(url: base.appendingPathComponent("v1/chat/completions"))
                request.httpMethod = "POST"
                request.setValue("application/json", forHTTPHeaderField: "Content-Type")
                let token = Credentials.read(config.server)
                if !token.isEmpty { request.setValue("Bearer " + token, forHTTPHeaderField: "Authorization") }
                request.httpBody = try JSONSerialization.data(withJSONObject: [
                    "model": config.remoteModel, "messages": [["role": "system", "content": system], ["role": "user", "content": user]],
                    "temperature": 0.2, "max_tokens": 2048, "reasoning_effort": "none",
                    "response_format": ["type": "json_schema", "json_schema": ["name": "translation", "strict": true, "schema": TranslationContract.schema(count: batch.count)]]
                ])
                let session = LMStudio.session(timeout: 180); defer { session.invalidateAndCancel() }
                let (responseData, response) = try await session.data(for: request)
                guard let response = response as? HTTPURLResponse, response.statusCode == 200 else { throw AppError.message("LM Studio HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0)") }
                guard responseData.count <= 4_194_304, let root = try JSONSerialization.jsonObject(with: responseData) as? [String: Any],
                      let choices = root["choices"] as? [[String: Any]], let choice = choices.first,
                      choice["finish_reason"] as? String == "stop", let message = choice["message"] as? [String: Any], let content = message["content"] as? String, !content.isEmpty else {
                    throw AppError.message(L("LM Studio returned no final answer. Check its reasoning/content separation setting and output token limit."))
                }
                data = Data(content.utf8)
            }
            result += try TranslationContract.decode(data, count: batch.count)
        }
        return result
    }
    func unload() async { await local.unload() }
}

actor LocalInference {
    #if canImport(llama)
    private var model: OpaquePointer?
    private var context: OpaquePointer?
    private var loaded: URL?
    private var contextSize = 0
    private static let backend: Void = { llama_backend_init() }()
    deinit { if let context { llama_free(context) }; if let model { llama_model_free(model) } }
    func unload() {
        if let context { llama_free(context) }; context = nil
        if let model { llama_model_free(model) }; model = nil
        loaded = nil
    }
    private func load(_ url: URL, size: Int) throws {
        if loaded == url && contextSize == size { return }
        unload()
        let bytes = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        let estimate = UInt64(bytes) + UInt64(size) * 65536 + 384 * 1024 * 1024
        #if os(iOS)
        let available = os_proc_available_memory()
        #else
        let available = ProcessInfo.processInfo.physicalMemory
        #endif
        guard UInt64(bytes) < ProcessInfo.processInfo.physicalMemory * 65 / 100,
              estimate < available * 9 / 10 else {
            throw AppError.message(L("This model leaves too little RAM for OCR, context and rendering. Choose a smaller GGUF or use LM Studio."))
        }
        _ = Self.backend
        var parameters = llama_model_default_params()
        parameters.n_gpu_layers = 99
        guard let model = llama_model_load_from_file(url.path, parameters) else { throw AppError.message(L("Could not load this GGUF model")) }
        var config = llama_context_default_params()
        config.n_ctx = UInt32(size); config.n_batch = 128; config.n_ubatch = 128
        config.n_threads = Int32(max(1, min(6, ProcessInfo.processInfo.processorCount - 2))); config.n_threads_batch = config.n_threads
        guard let context = llama_init_from_model(model, config) else { llama_model_free(model); throw AppError.message(L("Could not allocate local model context. Reduce context size.")) }
        self.model = model; self.context = context; loaded = url; contextSize = size
    }
    func generate(model url: URL, context size: Int, system: String, user: String) throws -> Data {
        let countRequested = try JSONDecoder().decode(TranslationPayload.self, from: Data(user.utf8)).segments.count
        guard (1...12).contains(countRequested) else { throw AppError.message(L("Invalid translation request")) }
        try load(url, size: size)
        guard let model, let context, let vocab = llama_model_get_vocab(model), let template = llama_model_chat_template(model, nil) else { throw AppError.message(L("The model has no supported chat template")) }
        llama_memory_clear(llama_get_memory(context), true)
        let strings = [strdup("system"), strdup(system), strdup("user"), strdup(user)]
        defer { strings.forEach { free($0) } }
        let messages = [llama_chat_message(role: strings[0], content: strings[1]), llama_chat_message(role: strings[2], content: strings[3])]
        var buffer = [CChar](repeating: 0, count: max(4096, (system.utf8.count + user.utf8.count) * 3 + 4096))
        let length = llama_chat_apply_template(template, messages, messages.count, true, &buffer, Int32(buffer.count))
        guard length > 0, length < buffer.count else { throw AppError.message(L("Unsupported chat template or prompt too large")) }
        let prompt = String(decoding: buffer.prefix(Int(length)).map { UInt8(bitPattern: $0) }, as: UTF8.self)
        let required = -llama_tokenize(vocab, prompt, Int32(prompt.utf8.count), nil, 0, true, true)
        guard required > 0, required < size - 1024 else { throw AppError.message(L("Page text exceeds model context. Increase context size.")) }
        var tokens = [llama_token](repeating: 0, count: Int(required))
        let count = llama_tokenize(vocab, prompt, Int32(prompt.utf8.count), &tokens, required, true, true)
        guard count > 0 else { throw AppError.message(L("Model tokenization failed")) }
        for start in stride(from: 0, to: Int(count), by: 128) {
            try Task.checkCancellation()
            var chunk = Array(tokens[start..<min(start + 128, Int(count))])
            let code = chunk.withUnsafeMutableBufferPointer { llama_decode(context, llama_batch_get_one($0.baseAddress, Int32($0.count))) }
            guard code == 0 else { throw AppError.message(L("Local inference failed during prompt processing")) }
        }
        guard let chain = llama_sampler_chain_init(llama_sampler_chain_default_params()), let grammar = llama_sampler_init_grammar(vocab, TranslationContract.grammar(count: countRequested), "root") else { throw AppError.message(L("Could not create structured-output sampler")) }
        defer { llama_sampler_free(chain) }
        llama_sampler_chain_add(chain, grammar)
        llama_sampler_chain_add(chain, llama_sampler_init_greedy())
        var output = Data()
        for _ in 0..<min(2048, size - Int(count) - 1) {
            try Task.checkCancellation()
            let token = llama_sampler_sample(chain, context, -1)
            if llama_vocab_is_eog(vocab, token) { return output }
            var piece = [CChar](repeating: 0, count: 256)
            var n = llama_token_to_piece(vocab, token, &piece, Int32(piece.count), 0, false)
            if n < 0 { piece = [CChar](repeating: 0, count: Int(-n)); n = llama_token_to_piece(vocab, token, &piece, Int32(piece.count), 0, false) }
            guard n >= 0 else { throw AppError.message(L("Could not decode generated token")) }
            output.append(contentsOf: piece.prefix(Int(n)).map { UInt8(bitPattern: $0) })
            var one = [token]
            let code = one.withUnsafeMutableBufferPointer { llama_decode(context, llama_batch_get_one($0.baseAddress, 1)) }
            guard code == 0 else { throw AppError.message(L("Local inference failed")) }
        }
        throw AppError.message(L("Model reached the output limit before completing JSON"))
    }
    #else
    func generate(model: URL, context: Int, system: String, user: String) throws -> Data {
        throw AppError.message(L("Local GGUF inference requires a physical iPad/iPhone. Use LM Studio in Simulator."))
    }
    func unload() {}
    #endif
}
