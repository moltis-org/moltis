// swiftlint:disable file_length type_body_length
import SwiftUI

struct SandboxesPane: View {
    @ObservedObject var settings: AppSettings

    var body: some View {
        Group {
            Section {
                if settings.sandboxLoading {
                    ProgressView("Loading sandbox settings…")
                } else {
                    LabeledContent("Container backend") {
                        Text(backendLabel)
                            .foregroundStyle(backendColor)
                    }
                    if let recommendation {
                        Text(recommendation.text)
                            .font(.caption)
                            .foregroundStyle(recommendation.level == .warning ? .orange : .secondary)
                    }
                    if !settings.sandboxRuntimeAvailable {
                        Text(sandboxDisabledHint)
                            .font(.caption)
                            .foregroundStyle(.orange)
                    }
                    if let error = settings.sandboxError {
                        Text(error)
                            .font(.caption)
                            .foregroundStyle(.red)
                    }
                    Button("Refresh status") {
                        settings.loadSandboxSettings()
                    }
                }
            } header: {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Overview")
                        .textCase(nil)
                    Text(
                        "Container images cached by moltis for sandbox execution. "
                            + "You can delete individual images or prune all. "
                            + "Build custom images from a base with apt packages."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textCase(nil)
                }
            }

            Section("Default image") {
                Text(
                    "Base image used for new sessions and projects unless overridden. "
                        + "Leave empty to use the built-in default (ubuntu:25.10)."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                TextField("ubuntu:25.10", text: $settings.sandboxDefaultImageDraft)
                    .font(.system(.body, design: .monospaced))

                HStack(spacing: 8) {
                    Button(settings.sandboxDefaultImageSaving ? "Saving…" : "Save") {
                        settings.saveSandboxDefaultImage()
                    }
                    .disabled(settings.sandboxDefaultImageSaving || !settings.sandboxRuntimeAvailable)

                    if let message = settings.sandboxDefaultImageMessage {
                        Text(message)
                            .font(.caption)
                            .foregroundStyle(.green)
                    }
                }

                if let error = settings.sandboxDefaultImageError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }

            Section("Shared home folder") {
                let modeLabel = settings.sandboxSharedHomeMode == "shared"
                    ? "enabled"
                    : "disabled (\(settings.sandboxSharedHomeMode))"
                LabeledContent("Status") {
                    Text(modeLabel)
                        .foregroundStyle(settings.sandboxSharedHomeMode == "shared" ? .orange : .secondary)
                }

                if settings.sandboxSharedHomeLoading {
                    ProgressView("Loading shared home settings…")
                } else {
                    Toggle("Enable shared home folder", isOn: $settings.sandboxSharedHomeEnabled)
                    TextField("Shared folder location", text: $settings.sandboxSharedHomePath)
                        .font(.system(.body, design: .monospaced))
                    LabeledContent("Configured path") {
                        Text(
                            settings.sandboxSharedHomeConfiguredPath.isEmpty
                                ? "default"
                                : settings.sandboxSharedHomeConfiguredPath
                        )
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                    }

                    HStack(spacing: 8) {
                        Button(settings.sandboxSharedHomeSaving ? "Saving…" : "Save") {
                            settings.saveSandboxSharedHome()
                        }
                        .disabled(settings.sandboxSharedHomeSaving)

                        if let message = settings.sandboxSharedHomeMessage {
                            Text(message)
                                .font(.caption)
                                .foregroundStyle(.green)
                        }
                    }

                    if let error = settings.sandboxSharedHomeError {
                        Text(error)
                            .font(.caption)
                            .foregroundStyle(.red)
                    }
                }
            }

            Section("Running Containers") {
                HStack(spacing: 8) {
                    Button("Restart") {
                        settings.restartSandboxDaemon()
                    }
                    .disabled(settings.sandboxContainersBusy || !settings.sandboxRuntimeAvailable)

                    Button("Refresh") {
                        settings.refreshSandboxContainersAndDiskUsage()
                    }
                    .disabled(settings.sandboxContainersLoading || !settings.sandboxRuntimeAvailable)

                    if !settings.sandboxContainers.isEmpty {
                        Button("Clean All", role: .destructive) {
                            settings.cleanAllSandboxContainers()
                        }
                        .disabled(settings.sandboxContainersBusy || !settings.sandboxRuntimeAvailable)
                    }
                }

                if let usage = settings.sandboxDiskUsage {
                    Text(
                        "Containers: \(usage.containersTotal) total, \(usage.containersActive) active "
                            + "· \(formatBytes(usage.containersSizeBytes)) "
                            + "(\(formatBytes(usage.containersReclaimableBytes)) reclaimable)"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    Text(
                        "Images: \(usage.imagesTotal) total, \(usage.imagesActive) active "
                            + "· \(formatBytes(usage.imagesSizeBytes))"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                if let error = settings.sandboxContainersError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }

                if settings.sandboxContainersLoading && settings.sandboxContainers.isEmpty {
                    ProgressView("Loading containers…")
                } else if settings.sandboxContainers.isEmpty {
                    Text("No containers found.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(settings.sandboxContainers) { container in
                        HStack(alignment: .top, spacing: 12) {
                            VStack(alignment: .leading, spacing: 2) {
                                HStack(spacing: 8) {
                                    Text(truncateHash(container.name))
                                        .font(.system(.caption, design: .monospaced))
                                        .lineLimit(1)
                                    Text(container.state)
                                        .font(.caption2)
                                        .foregroundStyle(stateColor(for: container.state))
                                }
                                Text("\(backendIcon(for: container.backend)) \(truncateHash(container.image))")
                                    .font(.system(.caption2, design: .monospaced))
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                if let cpus = container.cpus,
                                   let memoryMb = container.memoryMb {
                                    Text("\(cpus) CPU · \(memoryMb) MB")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                } else if let cpus = container.cpus {
                                    Text("\(cpus) CPU")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                } else if let memoryMb = container.memoryMb {
                                    Text("\(memoryMb) MB")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                            if container.state == "running" {
                                Button("Stop") {
                                    settings.stopSandboxContainer(name: container.name)
                                }
                                .controlSize(.small)
                                .disabled(settings.sandboxContainersBusy || !settings.sandboxRuntimeAvailable)
                            }
                            Button("Delete", role: .destructive) {
                                settings.removeSandboxContainer(name: container.name)
                            }
                            .controlSize(.small)
                            .disabled(settings.sandboxContainersBusy || !settings.sandboxRuntimeAvailable)
                        }
                    }
                }
            }

            Section("Cached images") {
                HStack(spacing: 8) {
                    Button("Prune all", role: .destructive) {
                        settings.pruneSandboxImages()
                    }
                    .disabled(settings.sandboxImagesBusy || !settings.sandboxRuntimeAvailable)

                    if let message = settings.sandboxImagesMessage {
                        Text(message)
                            .font(.caption)
                            .foregroundStyle(.green)
                    }
                }

                if settings.sandboxRuntimeBackend == "apple-container" {
                    Text(
                        "Apple Container provides VM-isolated execution but does not support "
                            + "building images. Docker (or OrbStack) is required alongside "
                            + "Apple Container to build and cache custom images. "
                            + "Sandboxed commands run via Apple Container, image builds use Docker."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                if let error = settings.sandboxImagesError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }

                if settings.sandboxImagesLoading {
                    ProgressView("Loading cached images…")
                } else if settings.sandboxImages.isEmpty {
                    Text("No cached images.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(settings.sandboxImages) { image in
                        HStack(alignment: .top, spacing: 12) {
                            VStack(alignment: .leading, spacing: 2) {
                                HStack(spacing: 8) {
                                    Text(truncateHash(image.tag))
                                        .font(.system(.caption, design: .monospaced))
                                        .lineLimit(1)
                                    Text(image.kind)
                                        .font(.caption2)
                                        .foregroundStyle(image.kind == "sandbox" ? .orange : .secondary)
                                }
                                Text("\(image.size) · \(image.created)")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button("Delete", role: .destructive) {
                                settings.deleteSandboxImage(tag: image.tag)
                            }
                            .controlSize(.small)
                            .disabled(settings.sandboxImagesBusy || !settings.sandboxRuntimeAvailable)
                        }
                    }
                }
            }

            Section("Build custom image") {
                TextField("Image name (e.g. my-tools)", text: $settings.sandboxBuildName)
                TextField("Base image (e.g. ubuntu:25.10)", text: $settings.sandboxBuildBase)
                    .font(.system(.body, design: .monospaced))
                TextEditor(text: $settings.sandboxBuildPackages)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 72)
                Button(settings.sandboxBuilding ? "Building…" : "Build") {
                    settings.buildSandboxImage()
                }
                .disabled(
                    settings.sandboxBuilding
                        || settings.sandboxBuildName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        || settings.sandboxBuildPackages.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        || !settings.sandboxRuntimeAvailable
                )

                if !settings.sandboxBuildWarning.isEmpty {
                    Text(settings.sandboxBuildWarning)
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
                if !settings.sandboxBuildStatus.isEmpty {
                    Text(settings.sandboxBuildStatus)
                        .font(.caption)
                        .foregroundStyle(settings.sandboxBuildStatus.hasPrefix("Error:") ? .red : .secondary)
                }
            }
        }
        .onAppear {
            settings.loadSandboxSettings()
        }
    }

    private var backendLabel: String {
        switch settings.sandboxRuntimeBackend {
        case "apple-container":
            "Apple Container (VM-isolated)"
        case "docker":
            "Docker"
        case "cgroup":
            "cgroup (systemd-run)"
        case "restricted-host":
            "Restricted Host (env + rlimits)"
        case "wasm":
            "Wasmtime (WASM-isolated)"
        case "none":
            "None (host execution)"
        default:
            settings.sandboxRuntimeBackend
        }
    }

    private var backendColor: Color {
        switch settings.sandboxRuntimeBackend {
        case "none":
            .red
        case "apple-container":
            .orange
        case "wasm":
            .green
        case "restricted-host":
            .yellow
        default:
            .secondary
        }
    }

    private var recommendation: Recommendation? {
        let os = settings.sandboxRuntimeOS
        let backend = settings.sandboxRuntimeBackend

        if backend == "none" {
            if os == "macos" {
                return Recommendation(
                    level: .warning,
                    text: "No container runtime detected. Install Apple Container (macOS 26+) "
                        + "for VM-isolated sandboxing, or install Docker as an alternative."
                )
            }
            if os == "linux" {
                return Recommendation(
                    level: .warning,
                    text: "No container runtime detected. Install Docker for sandboxed execution, "
                        + "or ensure systemd is available for cgroup isolation."
                )
            }
            return Recommendation(
                level: .warning,
                text: "No container runtime detected. Install Docker for sandboxed execution."
            )
        }

        if os == "macos", backend == "docker" {
            return Recommendation(
                level: .info,
                text: "Apple Container provides stronger VM-level isolation on macOS 26+. "
                    + "Install it for automatic use (moltis prefers it over Docker). "
                    + "Run: brew install container"
            )
        }

        if os == "linux", backend == "docker" {
            return Recommendation(
                level: .info,
                text: "Docker is a good choice on Linux. For lighter-weight isolation without "
                    + "Docker overhead, systemd cgroup sandboxing is also supported."
            )
        }

        if backend == "restricted-host" {
            return Recommendation(
                level: .info,
                text: "Using restricted host execution (env clearing, rlimits). "
                    + "For stronger isolation, install Docker or Apple Container."
            )
        }

        if backend == "wasm" {
            return Recommendation(
                level: .info,
                text: "Using WASM sandbox with filesystem isolation. "
                    + "For container-level isolation, install Docker or Apple Container."
            )
        }

        return nil
    }

    private var sandboxDisabledHint: String {
        "Sandboxes are disabled on cloud deploys without a container runtime. "
            + "Install on a VM with Docker or Apple Container to enable this feature."
    }

    private func backendIcon(for backend: String) -> String {
        switch backend {
        case "apple-container":
            "🍎"
        case "docker":
            "🐳"
        default:
            ""
        }
    }

    private func stateColor(for state: String) -> Color {
        switch state {
        case "running":
            .orange
        default:
            .secondary
        }
    }

    private func truncateHash(_ value: String) -> String {
        if let index = value.lastIndex(of: ":") {
            let suffix = value[value.index(after: index)...]
            if suffix.count > 12 {
                let prefix = value[...index]
                let start = suffix.prefix(6)
                let end = suffix.suffix(6)
                return "\(prefix)\(start)…\(end)"
            }
        }

        if !value.contains(":"), value.count > 24 {
            let start = value.prefix(6)
            let end = value.suffix(6)
            return "\(start)…\(end)"
        }

        return value
    }

    private func formatBytes(_ bytes: UInt64) -> String {
        if bytes < 1_024 {
            return "\(bytes) B"
        }
        if bytes < 1_048_576 {
            return String(format: "%.1f KB", Double(bytes) / 1_024.0)
        }
        if bytes < 1_073_741_824 {
            return String(format: "%.1f MB", Double(bytes) / 1_048_576.0)
        }
        return String(format: "%.1f GB", Double(bytes) / 1_073_741_824.0)
    }
}

private struct Recommendation {
    enum Level {
        case warning
        case info
    }

    let level: Level
    let text: String
}
// swiftlint:enable file_length type_body_length
