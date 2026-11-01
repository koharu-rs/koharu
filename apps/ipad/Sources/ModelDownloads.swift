import Foundation

extension MangaLibrary {
    func importModel(_ source: URL) {
        guard !downloadingModel else { return }
        downloadingModel = true
        modelTask = Task {
            defer { downloadingModel = false; modelTask = nil }
            let granted = source.startAccessingSecurityScopedResource()
            defer { if granted { source.stopAccessingSecurityScopedResource() } }
            do {
                let id = UUID(); let destination = disk.modelsURL.appendingPathComponent(id.uuidString + ".gguf")
                let size = try await Task.detached {
                    try Self.validateGGUF(source)
                    try FileManager.default.copyItem(at: source, to: destination)
                    return Int64(try destination.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
                }.value
                models.append(LocalModel(id: id, name: source.deletingPathExtension().lastPathComponent, bytes: size, source: nil))
                preferences.localModel = id
                try disk.save(models, to: disk.root.appendingPathComponent("models.json")); savePreferences()
            } catch { self.error = error.localizedDescription }
        }
    }
    func downloadModel(_ address: String) {
        guard !downloadingModel else { return }
        guard let url = URL(string: address), url.scheme == "https", url.host != nil, url.user == nil, url.password == nil else { error = L("Enter an HTTPS GGUF download URL"); return }
        downloadingModel = true
        modelTask = Task {
            defer { downloadingModel = false; modelTask = nil }
            do {
                status = L("Downloading model")
                let config = URLSessionConfiguration.default
                config.timeoutIntervalForRequest = 60; config.timeoutIntervalForResource = 86_400
                let session = URLSession(configuration: config, delegate: RedirectBoundary { $0.scheme == "https" && $0.user == nil && $0.password == nil }, delegateQueue: nil); defer { session.invalidateAndCancel() }
                let (file, response) = try await session.download(from: url)
                defer { try? FileManager.default.removeItem(at: file) }
                guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw AppError.message(L("Model download failed")) }
                try Task.checkCancellation(); try Self.validateGGUF(file)
                let id = UUID(); let destination = disk.modelsURL.appendingPathComponent(id.uuidString + ".gguf")
                try FileManager.default.moveItem(at: file, to: destination)
                var excluded = destination; var attributes = URLResourceValues(); attributes.isExcludedFromBackup = true; try excluded.setResourceValues(attributes)
                let size = Int64(try destination.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
                models.append(LocalModel(id: id, name: url.deletingPathExtension().lastPathComponent, bytes: size, source: url.absoluteString))
                preferences.localModel = id
                try disk.save(models, to: disk.root.appendingPathComponent("models.json")); savePreferences()
                status = L("Model ready")
            } catch is CancellationError { status = L("Stopped") }
            catch { self.error = error.localizedDescription }
        }
    }
    nonisolated private static func validateGGUF(_ url: URL) throws {
        let file = try FileHandle(forReadingFrom: url); defer { try? file.close() }
        guard try file.read(upToCount: 4) == Data("GGUF".utf8) else { throw AppError.message(L("This file is not a GGUF model")) }
    }
    func removeModel(_ model: LocalModel) async {
        guard !busy && !downloadingModel else { return }
        await translator.unload()
        do {
            try FileManager.default.removeItem(at: disk.modelsURL.appendingPathComponent(model.filename))
            models.removeAll { $0.id == model.id }
            if preferences.localModel == model.id { preferences.localModel = nil }
            try disk.save(models, to: disk.root.appendingPathComponent("models.json")); savePreferences()
        } catch { self.error = error.localizedDescription }
    }
}
