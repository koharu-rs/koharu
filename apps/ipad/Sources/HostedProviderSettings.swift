import SwiftUI

struct HostedProviderSettings: View {
    @Environment(MangaLibrary.self) private var library
    let provider: TranslationProvider
    @State private var token = ""
    @State private var models: [HostedModel] = []
    @State private var checking = false
    @State private var saved = false
    @State private var status = ""
    @State private var failed = false
    @State private var revision = UUID()
    @State private var validation: Task<Void, Never>?
    @State private var service = HostedTranslation()
    private var configuration: HostedConfiguration { library.preferences.hosted?[provider.rawValue] ?? HostedConfiguration() }
    private var endpoint: HostedEndpoint? { try? HostedEndpoint(provider: provider, configuration: configuration) }
    private func binding(_ key: WritableKeyPath<HostedConfiguration, String>) -> Binding<String> {
        Binding(get: { configuration[keyPath: key] }, set: { value in
            var updated = configuration; updated[keyPath: key] = value
            if library.preferences.hosted == nil { library.preferences.hosted = [:] }
            library.preferences.hosted?[provider.rawValue] = updated
        })
    }
    var body: some View {
        Section(provider.name) {
            if provider == .qwen || provider == .miniMax {
                Picker(L("API region"), selection: binding(\.region)) {
                    Text(L("International") + (provider == .qwen ? " · " + (AppLocalization.shared.locale.localizedString(forRegionCode: "SG") ?? "SG") : "")).tag("international")
                    Text(provider == .qwen ? L("China (Beijing)") : (AppLocalization.shared.locale.localizedString(forRegionCode: "CN") ?? "CN")).tag("china")
                    if provider == .qwen {
                        Text(L("United States (Virginia)")).tag("us")
                        Text(L("Hong Kong")).tag("hongkong")
                        Text(L("Japan (Tokyo)")).tag("japan")
                        Text(L("Germany (Frankfurt)")).tag("germany")
                    }
                }
                if provider == .qwen {
                    TextField(L("Workspace ID"), text: binding(\.workspace)).textInputAutocapitalization(.never).autocorrectionDisabled().environment(\.layoutDirection, .leftToRight)
                }
                Text(L("Use an API key created in the selected region. Qwen also requires your Model Studio workspace ID.")).font(.caption).foregroundStyle(.secondary)
            }
            SecureField(L("API key"), text: $token).textInputAutocapitalization(.never).autocorrectionDisabled().privacySensitive().environment(\.layoutDirection, .leftToRight)
            Button { validate() } label: { Label(L("Validate key and load models"), systemImage: "arrow.clockwise") }
                .disabled(checking || token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || endpoint == nil)
            if checking { ProgressView(L("Checking API key…")) }
            if !status.isEmpty {
                Label(status, systemImage: failed ? "exclamationmark.circle" : "checkmark.circle")
                    .font(.caption).foregroundStyle(failed ? Color.orange : Color.green)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("provider-validation-status")
            }
            if !models.isEmpty {
                Picker(L("Model"), selection: binding(\.model)) {
                    Text(L("Select a model")).tag("")
                    ForEach(models) { Text($0.displayName).tag($0.id) }
                }
                Text(L("Key verified. Model access and available credit are checked again when translating.")).font(.caption).foregroundStyle(.secondary)
            } else {
                LabeledContent(L("Model"), value: L("Validate the API key first")).foregroundStyle(.secondary)
            }
            if saved {
                Button(L("Remove saved API key"), role: .destructive) {
                    guard let endpoint else { return }
                    do {
                        invalidate()
                        try Credentials.save("", endpoint.credentialAccount)
                        token = ""; saved = false
                        binding(\.model).wrappedValue = ""
                    } catch { library.error = error.localizedDescription }
                }
            }
            if provider == .miniMax {
                Text(L("Only MiniMax-Text-01 currently supports the required native JSON schema. Other MiniMax models are excluded.")).font(.caption).foregroundStyle(.secondary)
            }
            if provider == .qwen {
                Text(L("Only Qwen models with native JSON Schema support are listed.")).font(.caption).foregroundStyle(.secondary)
            }
            if provider == .deepL {
                Text(L("DeepL uses translation modes instead of LLM model names. Free or Pro is detected from the key; supported languages are checked before translation. Additional instructions are limited to 300 characters.")).font(.caption).foregroundStyle(.secondary)
            }
            Text(L("Validation saves this provider's key in Keychain on this device. Keys are separate for each provider and region and are excluded from chapter exports.")).font(.caption).foregroundStyle(.secondary)
        }
        .task(id: endpoint?.credentialAccount) {
            invalidate()
            token = endpoint.map { Credentials.read($0.credentialAccount) } ?? ""
            saved = !token.isEmpty
        }
        .onChange(of: token) { _, _ in invalidate() }
        .onChange(of: configuration.region) { _, _ in invalidate() }
        .onChange(of: configuration.workspace) { _, _ in invalidate() }
        .onDisappear { validation?.cancel() }
    }
    private func invalidate() {
        revision = UUID(); validation?.cancel(); validation = nil
        models = []; checking = false; status = ""; failed = false
    }
    private func validate() {
        guard let endpoint else { return }
        invalidate()
        let current = revision
        let key = token.trimmingCharacters(in: .whitespacesAndNewlines)
        checking = true
        validation = Task { @MainActor in
            do {
                let available = try await service.models(endpoint: endpoint, key: key, refresh: true)
                try Task.checkCancellation()
                guard current == revision, self.endpoint == endpoint else { return }
                try Credentials.save(key, endpoint.credentialAccount)
                models = available; saved = true
                if !available.contains(where: { $0.id == configuration.model }) { binding(\.model).wrappedValue = "" }
                status = L("API key verified and saved")
            } catch {
                guard !Task.isCancelled, current == revision else { return }
                status = error.localizedDescription; failed = true
            }
            if current == revision { checking = false }
        }
    }
}
