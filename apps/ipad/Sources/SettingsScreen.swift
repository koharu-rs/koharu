import SwiftUI
import UniformTypeIdentifiers

struct SettingsScreen: View {
    struct CatalogModel: Decodable, Identifiable {
        var name: String; var url: String; var id: String { url }
        static var all: [Self] {
            guard let url = Bundle.main.url(forResource: "local-models", withExtension: "json"), let data = try? Data(contentsOf: url) else { return [] }
            return (try? JSONDecoder().decode([Self].self, from: data)) ?? []
        }
    }
    @Environment(MangaLibrary.self) private var library
    @State private var remoteModels: [String] = []
    @State private var servers: [LMStudio.Server] = []
    @State private var probing = false
    @State private var token = ""
    @State private var downloadURL = ""
    @State private var importing = false
    @State private var deleting: LocalModel?
    var body: some View {
        @Bindable var library = library
        @Bindable var localization = AppLocalization.shared
        Form {
            Section(L("Interface")) {
                Picker(L("App language"), selection: $localization.language) {
                    ForEach(InterfaceLanguage.allCases) { Text($0.name).tag($0) }
                }
                Text(L("Interface language changes menus and controls. The translation target is selected separately.")).font(.caption).foregroundStyle(.secondary)
            }
            Section(L("Translation")) {
                Picker(L("Provider"), selection: $library.preferences.provider) {
                    ForEach(TranslationProvider.allCases) { Text($0.name).tag($0) }
                }
                Picker(L("Target language"), selection: $library.preferences.target) {
                    ForEach(TargetLanguage.all) { Text($0.displayName).tag($0.tag) }
                }
                TextField(L("Additional instructions"), text: $library.preferences.instructions, axis: .vertical).lineLimit(3...6)
            }
            if library.preferences.provider == .lmStudio {
                Section("LM Studio") {
                    TextField(L("Server URL"), text: $library.preferences.server).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled().environment(\.layoutDirection, .leftToRight)
                    Text(L("Use the server root URL, without /v1. Enable LAN serving in LM Studio.")).font(.caption).foregroundStyle(.secondary)
                    SecureField(L("API token (optional)"), text: $token).textInputAutocapitalization(.never).autocorrectionDisabled().environment(\.layoutDirection, .leftToRight)
                    Button(L("Save credential")) {
                        do { try Credentials.save(token, library.preferences.server) } catch { library.error = error.localizedDescription }
                    }
                    Button {
                        Task { await refreshModels() }
                    } label: { Label(L("Refresh models"), systemImage: "arrow.clockwise") }.disabled(probing || library.busy)
                    Picker(L("Model"), selection: $library.preferences.remoteModel) {
                        if !remoteModels.contains(library.preferences.remoteModel) { Text(library.preferences.remoteModel.isEmpty ? L("Select a model") : library.preferences.remoteModel).tag(library.preferences.remoteModel) }
                        ForEach(remoteModels, id: \.self) { Text($0).tag($0) }
                    }
                    Button {
                        probing = true
                        Task {
                            defer { probing = false }
                            do {
                                let base = try LMStudio.baseURL(library.preferences.server)
                                servers = try await LMStudio.discover(port: base.port ?? (base.scheme == "https" ? 443 : 80))
                                if servers.isEmpty { library.error = L("No LM Studio server found. Check Wi-Fi, LAN serving and firewall.") }
                            } catch { library.error = error.localizedDescription }
                        }
                    } label: { Label(L("Find server on /24 LAN"), systemImage: "network") }.disabled(probing || library.busy)
                    if probing { ProgressView() }
                    ForEach(servers) { server in
                        Button {
                            library.preferences.server = server.url
                            remoteModels = server.models
                            library.preferences.remoteModel = server.models.first ?? ""
                            token = Credentials.read(server.url)
                        } label: { VStack(alignment: .leading) { Text(server.url); Text("\(server.models.count) \(L("models"))").font(.caption).foregroundStyle(.secondary) } }
                    }
                }
            } else if library.preferences.provider == .local {
                Section(L("Local AI")) {
                    Picker(L("Model"), selection: $library.preferences.localModel) {
                        Text(L("Select a model")).tag(UUID?.none)
                        ForEach(library.models) { Text($0.name).tag(Optional($0.id)) }
                    }
                    Picker(L("Context tokens"), selection: $library.preferences.contextSize) {
                        ForEach([2048, 4096, 8192, 16384], id: \.self) { Text(String($0)).tag($0) }
                    }
                    Text("\(L("Device RAM")): \(ByteCountFormatter.string(fromByteCount: Int64(ProcessInfo.processInfo.physicalMemory), countStyle: .memory))").font(.caption)
                    Text(L("Choose GGUF weights that leave RAM for OCR, context and rendering. Large 27B models may not fit even on an M5 iPad.")).font(.caption).foregroundStyle(.secondary)
                    #if targetEnvironment(simulator)
                    Text(L("Local GGUF inference requires a physical iPad/iPhone. Use LM Studio in Simulator.")).font(.caption).foregroundStyle(.orange)
                    #endif
                    TextField(L("HTTPS GGUF download URL"), text: $downloadURL).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled().environment(\.layoutDirection, .leftToRight)
                    Menu(L("Choose a GGUF download")) {
                        ForEach(CatalogModel.all) { model in Button(model.name) { downloadURL = model.url } }
                    }
                    Button(L("Download GGUF"), systemImage: "arrow.down.circle") { library.downloadModel(downloadURL) }.disabled(downloadURL.isEmpty || library.downloadingModel || library.busy)
                    Button(L("Import GGUF from Files"), systemImage: "folder") { importing = true }.disabled(library.downloadingModel || library.busy)
                    Button(L("Unload local model")) { Task { await library.translator.unload() } }.disabled(library.busy)
                    ForEach(library.models) { model in
                        HStack {
                            VStack(alignment: .leading) { Text(model.name); Text(ByteCountFormatter.string(fromByteCount: model.bytes, countStyle: .file)).font(.caption).foregroundStyle(.secondary) }
                            Spacer()
                            Button(role: .destructive) { deleting = model } label: { Image(systemName: "trash") }.disabled(library.busy || library.downloadingModel)
                        }
                    }
                }
            }
            if library.preferences.provider.hosted {
                HostedProviderSettings(provider: library.preferences.provider).id(library.preferences.provider)
            }
            if VisionModel.all.isEmpty { Text(L("Invalid vision model manifest")).foregroundStyle(.red) }
            Section(L("OCR model")) {
                Picker(L("Model"), selection: $library.preferences.ocrModel) {
                    Text(L("Apple Vision (built in)")).tag(String?.none)
                    ForEach(VisionModel.all.filter { $0.kind == "ocr" }) { Text($0.name).tag(Optional($0.id)) }
                }
                Text(L("Apple Vision detects text regions. The selected OCR model reads each crop locally; models are downloaded separately.")).font(.caption).foregroundStyle(.secondary)
                if let id = library.preferences.ocrModel { VisionModelDownload(modelID: id) }
            }
            Section(L("Image cleanup")) {
                Toggle(L("AI inpainting"), isOn: $library.preferences.inpaint)
                Picker(L("Model"), selection: Binding(get: { library.preferences.cleanupModel ?? "lama" }, set: { library.preferences.cleanupModel = $0 })) {
                    ForEach(VisionModel.all.filter { $0.kind == "inpaint" }) { Text($0.name).tag($0.id) }
                }
                Text(L("Inpainting runs on this device on native-resolution crops. FLUX and RORem need more RAM and time. Without cleanup, translated text uses white speech cards.")).font(.caption).foregroundStyle(.secondary)
                VisionModelDownload(modelID: library.preferences.cleanupModel ?? "lama")
            }
            if library.downloadingModel {
                Section {
                    HStack { ProgressView(); Text(library.status) }
                    Button(L("Cancel download"), role: .cancel) { library.modelTask?.cancel() }
                }
            }
            Section(L("About")) {
                Text("Koharu iPad 0.3.2")
                Text(L("Projects and source URLs stay on this device. Remote providers receive OCR text and translation instructions. Local mode translates on this device. API keys are stored in Keychain.")).font(.caption).foregroundStyle(.secondary)
            }
        }
        .navigationTitle(L("Settings"))
        .disabled(library.busy)
        .task { token = Credentials.read(library.preferences.server) }
        .onChange(of: library.preferences.server) { _, value in token = Credentials.read(value) }
        .fileImporter(isPresented: $importing, allowedContentTypes: [UTType(filenameExtension: "gguf") ?? .data]) { result in
            switch result { case .success(let url): library.importModel(url); case .failure(let error): library.error = error.localizedDescription }
        }
        .confirmationDialog(L("Delete this downloaded model?"), isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible) {
            Button(L("Delete"), role: .destructive) { if let deleting { Task { await library.removeModel(deleting) } }; deleting = nil }
        }
    }
    private func refreshModels() async {
        probing = true; defer { probing = false }
        do {
            remoteModels = try await LMStudio.models(library.preferences.server, token: Credentials.read(library.preferences.server))
            if !remoteModels.contains(library.preferences.remoteModel) { library.preferences.remoteModel = remoteModels.first ?? "" }
        } catch { library.error = error.localizedDescription }
    }
}

struct VisionModelDownload: View {
    @Environment(MangaLibrary.self) private var library
    let modelID: String
    @State private var removing = false
    var body: some View {
        if let model = VisionModel.all.first(where: { $0.id == modelID }) {
            Text("\(ByteCountFormatter.string(fromByteCount: model.bytes, countStyle: .file)) · \(L("Estimated free RAM required")): \(model.minimumFreeMB) MB").font(.caption).foregroundStyle(.secondary)
            if VisionModel.availableMemory < model.minimumFreeMB * 1024 * 1024 {
                Label(L("Not enough available RAM"), systemImage: "memorychip").font(.caption).foregroundStyle(.orange)
            }
            if model.ready(library.disk.modelsURL) {
                HStack {
                    Label(L("Model ready"), systemImage: "checkmark.circle").foregroundStyle(.green)
                    Spacer()
                    Button(role: .destructive) { removing = true } label: { Image(systemName: "trash") }
                        .accessibilityLabel(L("Delete this downloaded model?"))
                        .disabled(library.busy || library.downloadingModel)
                }
                .confirmationDialog(L("Delete this downloaded model?"), isPresented: $removing, titleVisibility: .visible) {
                    Button(L("Delete"), role: .destructive) {
                        library.removeVisionModel(model)
                    }
                }
            } else {
                Button(L("Download selected model"), systemImage: "arrow.down.circle") { library.downloadVisionModel(model) }.disabled(library.downloadingModel || library.busy)
            }
            Link(L("Model source"), destination: model.source)
        }
    }
}
