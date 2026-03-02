import AppKit
import SwiftUI

struct HttpdPane: View {
    @ObservedObject var settings: AppSettings
    @State private var serverAddr: String?
    @State private var errorMessage: String?
    @State private var isStarting = false

    private let client = MoltisClient()

    var body: some View {
        Group {
            Section {
                // swiftlint:disable:next line_length
                Text("The HTTP server runs the full Moltis gateway — web UI, REST API, and WebSocket — on your local machine. Open the address in a browser to use the web interface.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Section {
                Toggle("Enable HTTP server", isOn: $settings.httpdEnabled)
                    .disabled(isStarting)
                    .onChange(of: settings.httpdEnabled) { _, enabled in
                        if enabled {
                            startServer()
                        } else {
                            stopServer()
                        }
                    }

                Picker("Bind address", selection: $settings.httpdBindMode) {
                    Text("Loopback (127.0.0.1)").tag("loopback")
                    Text("All interfaces (0.0.0.0)").tag("all")
                }
                .disabled(serverAddr != nil || isStarting)

                TextField("Port", text: $settings.httpdPort)
                    .disabled(serverAddr != nil || isStarting)

                if isStarting {
                    HStack(spacing: 8) {
                        ProgressView()
                            .controlSize(.small)
                        Text("Starting gateway…")
                            .foregroundStyle(.secondary)
                    }
                    .font(.callout)
                }

                if let addr = serverAddr {
                    LabeledContent("Listening on") {
                        HStack(spacing: 8) {
                            Text(addr)
                                .font(.system(.body, design: .monospaced))
                                .textSelection(.enabled)
                            Button("Open in Browser") {
                                if let url = URL(string: "http://\(addr)") {
                                    NSWorkspace.shared.open(url)
                                }
                            }
                            .controlSize(.small)
                        }
                    }
                }

                if let error = errorMessage {
                    HStack(spacing: 6) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                        Text(error)
                            .foregroundStyle(.secondary)
                    }
                    .font(.callout)
                }
            }
        }
        .onAppear { syncStatus() }
    }

    private func startServer() {
        errorMessage = nil
        isStarting = true
        let port = UInt16(settings.httpdPort) ?? 8080
        let host = settings.httpdBindMode == "all" ? "0.0.0.0" : "127.0.0.1"

        // Gateway init runs DB migrations and can take 1-3s — dispatch off main thread.
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                let status = try client.startHttpd(host: host, port: port)
                DispatchQueue.main.async {
                    serverAddr = status.addr
                    isStarting = false
                }
            } catch {
                DispatchQueue.main.async {
                    errorMessage = error.localizedDescription
                    settings.httpdEnabled = false
                    isStarting = false
                }
            }
        }
    }

    private func stopServer() {
        errorMessage = nil
        do {
            _ = try client.stopHttpd()
            serverAddr = nil
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func syncStatus() {
        guard let status = try? client.httpdStatus() else { return }
        if status.running {
            settings.httpdEnabled = true
            serverAddr = status.addr
        }
    }
}
