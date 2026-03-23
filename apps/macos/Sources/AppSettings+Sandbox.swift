import Foundation

extension AppSettings {
    func loadSandboxSettings() {
        sandboxLoading = true
        sandboxError = nil
        refreshSandboxStatus()
        loadSandboxImages()
        refreshSandboxContainersAndDiskUsage()
        loadSandboxSharedHome()
        sandboxLoading = false
    }

    func refreshSandboxStatus() {
        sandboxError = nil
        sandboxDefaultImageError = nil

        do {
            let status = try client.sandboxStatus()
            sandboxRuntimeBackend = status.backend
            sandboxRuntimeOS = status.os
            sandboxRuntimeDefaultImage = status.defaultImage
            sandboxDefaultImageDraft = status.defaultImage
        } catch {
            sandboxError = error.localizedDescription
            logSettingsError("load sandbox status", error)
        }
    }

    func loadSandboxImages() {
        sandboxImagesLoading = true
        sandboxImagesError = nil

        do {
            sandboxImages = try client.sandboxListImages()
        } catch {
            sandboxImages = []
            sandboxImagesError = error.localizedDescription
            logSettingsError("load sandbox images", error)
        }

        sandboxImagesLoading = false
    }

    func deleteSandboxImage(tag: String) {
        guard sandboxRuntimeAvailable else { return }
        sandboxImagesBusy = true
        sandboxImagesError = nil
        sandboxImagesMessage = nil

        do {
            try client.sandboxDeleteImage(tag: tag)
            sandboxImagesMessage = "Image deleted."
            loadSandboxImages()
        } catch {
            sandboxImagesError = error.localizedDescription
            logSettingsError("delete sandbox image", error)
        }

        sandboxImagesBusy = false
    }

    func pruneSandboxImages() {
        guard sandboxRuntimeAvailable else { return }
        sandboxImagesBusy = true
        sandboxImagesError = nil
        sandboxImagesMessage = nil

        do {
            let pruned = try client.sandboxPruneImages()
            sandboxImagesMessage = pruned == 1 ? "Pruned 1 image." : "Pruned \(pruned) images."
            loadSandboxImages()
        } catch {
            sandboxImagesError = error.localizedDescription
            logSettingsError("prune sandbox images", error)
        }

        sandboxImagesBusy = false
    }

    func saveSandboxDefaultImage() {
        sandboxDefaultImageSaving = true
        sandboxDefaultImageError = nil
        sandboxDefaultImageMessage = nil

        guard sandboxRuntimeAvailable else {
            sandboxDefaultImageError = "No sandbox backend available."
            sandboxDefaultImageSaving = false
            return
        }

        let trimmed = sandboxDefaultImageDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let image = try client.sandboxSetDefaultImage(image: trimmed.isEmpty ? nil : trimmed)
            sandboxRuntimeDefaultImage = image
            sandboxDefaultImageDraft = image
            sandboxDefaultImageMessage = "Saved."
        } catch {
            sandboxDefaultImageError = error.localizedDescription
            logSettingsError("save default sandbox image", error)
        }

        sandboxDefaultImageSaving = false
    }

    // swiftlint:disable function_body_length
    func buildSandboxImage() {
        guard sandboxRuntimeAvailable else { return }

        let name = sandboxBuildName.trimmingCharacters(in: .whitespacesAndNewlines)
        if name.isEmpty {
            sandboxBuildStatus = "Please specify an image name."
            return
        }

        let trimmedBase = sandboxBuildBase.trimmingCharacters(in: .whitespacesAndNewlines)
        let base = trimmedBase.isEmpty ? "ubuntu:25.10" : trimmedBase
        let requestedPackages = parseSandboxPackages(sandboxBuildPackages)
        if requestedPackages.isEmpty {
            sandboxBuildStatus = "Please specify at least one package."
            return
        }

        sandboxBuilding = true
        sandboxBuildWarning = ""
        sandboxBuildStatus = "Checking packages in base image…"
        sandboxImagesError = nil

        var packagesToInstall = requestedPackages

        do {
            let found = try client.sandboxCheckPackages(base: base, packages: requestedPackages)
            let present = requestedPackages.filter { found[$0] == true }
            let missing = requestedPackages.filter { found[$0] != true }

            if !present.isEmpty && missing.isEmpty {
                sandboxBuildWarning = "All requested packages are already present in \(base): "
                    + present.joined(separator: ", ")
                    + ". No image build needed."
                sandboxBuildStatus = ""
                sandboxBuilding = false
                return
            }

            if !present.isEmpty {
                sandboxBuildWarning = "Already in \(base): \(present.joined(separator: ", ")). "
                    + "Only installing: \(missing.joined(separator: ", "))."
            }

            packagesToInstall = missing.isEmpty ? requestedPackages : missing
        } catch {
            // Package inspection is best-effort. Build with the full package list.
            packagesToInstall = requestedPackages
        }

        do {
            let tag = try client.sandboxBuildImage(
                name: name,
                base: base,
                packages: packagesToInstall
            )
            sandboxBuildStatus = "Built: \(tag)"
            sandboxBuildName = ""
            sandboxBuildPackages = ""
            loadSandboxImages()
        } catch {
            sandboxBuildStatus = "Error: \(error.localizedDescription)"
            logSettingsError("build sandbox image", error)
        }

        sandboxBuilding = false
    }
    // swiftlint:enable function_body_length

    func refreshSandboxContainersAndDiskUsage() {
        loadSandboxContainers()
        loadSandboxDiskUsage()
    }

    func loadSandboxContainers() {
        sandboxContainersLoading = true
        sandboxContainersError = nil

        do {
            sandboxContainers = try client.sandboxListContainers()
        } catch {
            sandboxContainers = []
            sandboxContainersError = error.localizedDescription
            logSettingsError("load sandbox containers", error)
        }

        sandboxContainersLoading = false
    }

    func loadSandboxDiskUsage() {
        do {
            sandboxDiskUsage = try client.sandboxDiskUsage()
        } catch {
            sandboxDiskUsage = nil
        }
    }

    func stopSandboxContainer(name: String) {
        guard sandboxRuntimeAvailable else { return }
        sandboxContainersBusy = true
        sandboxContainersError = nil

        do {
            try client.sandboxStopContainer(name: name)
            refreshSandboxContainersAndDiskUsage()
        } catch {
            sandboxContainersError = error.localizedDescription
            logSettingsError("stop sandbox container", error)
        }

        sandboxContainersBusy = false
    }

    func removeSandboxContainer(name: String) {
        guard sandboxRuntimeAvailable else { return }
        sandboxContainersBusy = true
        sandboxContainersError = nil

        do {
            try client.sandboxRemoveContainer(name: name)
            refreshSandboxContainersAndDiskUsage()
        } catch {
            sandboxContainersError = error.localizedDescription
            logSettingsError("remove sandbox container", error)
        }

        sandboxContainersBusy = false
    }

    func cleanAllSandboxContainers() {
        guard sandboxRuntimeAvailable else { return }
        sandboxContainersBusy = true
        sandboxContainersError = nil

        do {
            _ = try client.sandboxCleanContainers()
            refreshSandboxContainersAndDiskUsage()
        } catch {
            sandboxContainersError = error.localizedDescription
            logSettingsError("clean sandbox containers", error)
        }

        sandboxContainersBusy = false
    }

    func restartSandboxDaemon() {
        guard sandboxRuntimeAvailable else { return }
        sandboxContainersBusy = true
        sandboxContainersError = nil

        do {
            try client.sandboxRestartDaemon()
            refreshSandboxContainersAndDiskUsage()
        } catch {
            sandboxContainersError = error.localizedDescription
            logSettingsError("restart sandbox daemon", error)
        }

        sandboxContainersBusy = false
    }

    func loadSandboxSharedHome() {
        sandboxSharedHomeLoading = true
        sandboxSharedHomeError = nil
        sandboxSharedHomeMessage = nil

        do {
            let config = try client.sandboxGetSharedHome()
            applySandboxSharedHome(config)
        } catch {
            sandboxSharedHomeError = error.localizedDescription
            logSettingsError("load sandbox shared home", error)
        }

        sandboxSharedHomeLoading = false
    }

    func saveSandboxSharedHome() {
        sandboxSharedHomeSaving = true
        sandboxSharedHomeError = nil
        sandboxSharedHomeMessage = nil

        let path = sandboxSharedHomePath.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let config = try client.sandboxSetSharedHome(
                enabled: sandboxSharedHomeEnabled,
                path: path.isEmpty ? nil : path
            )
            applySandboxSharedHome(config)
            sandboxSharedHomeMessage = "Saved. Restart Moltis to apply shared folder changes."
        } catch {
            sandboxSharedHomeError = error.localizedDescription
            logSettingsError("save sandbox shared home", error)
        }

        sandboxSharedHomeSaving = false
    }

    private func applySandboxSharedHome(_ config: BridgeSandboxSharedHomePayload) {
        sandboxSharedHomeEnabled = config.enabled
        sandboxSharedHomeMode = config.mode
        sandboxSharedHomePath = config.path
        sandboxSharedHomeConfiguredPath = config.configuredPath ?? ""
    }

    private func parseSandboxPackages(_ raw: String) -> [String] {
        raw
            .split(whereSeparator: { $0.isWhitespace || $0 == "," })
            .map { String($0) }
            .filter { !$0.isEmpty }
    }
}
