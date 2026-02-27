import SwiftUI

struct ConnectView: View {
    @EnvironmentObject var authManager: AuthManager
    @EnvironmentObject var connectionStore: ConnectionStore
    @StateObject private var bonjourBrowser = BonjourBrowser()

    @State private var serverURL = ""
    @State private var serverName = ""
    @State private var password = ""
    @State private var apiKey = ""
    @State private var authStatus: AuthStatusResponse?
    @State private var showError = false
    @State private var errorMessage = ""
    @State private var authMode: AuthMode = .check

    enum AuthMode {
        case check
        case password
        case apiKey
    }

    var body: some View {
        NavigationStack {
            Form {
                // Saved servers
                if !authManager.servers.isEmpty {
                    Section("Saved Servers") {
                        ServerListView()
                    }
                }

                // Discovered via Bonjour/mDNS
                if !bonjourBrowser.servers.isEmpty {
                    Section("Nearby Servers") {
                        ForEach(bonjourBrowser.servers) { server in
                            Button {
                                selectDiscovered(server)
                            } label: {
                                HStack {
                                    VStack(alignment: .leading) {
                                        Text(server.name)
                                        Text("\(server.host):\(server.port)")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                    Spacer()
                                    if let version = server.version {
                                        Text("v\(version)")
                                            .font(.caption2)
                                            .foregroundStyle(.tertiary)
                                    }
                                }
                            }
                        }
                    }
                }

                // New server
                Section("Connect to Server") {
                    TextField("Server URL", text: $serverURL)
                        .textContentType(.URL)
                        .keyboardType(.URL)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)

                    TextField("Display Name", text: $serverName)

                    switch authMode {
                    case .check:
                        Button("Check Connection") {
                            Task { await checkServer() }
                        }
                        .disabled(serverURL.isEmpty)

                    case .password:
                        SecureField("Password", text: $password)
                            .textContentType(.password)

                        Button("Login & Connect") {
                            Task { await loginWithPassword() }
                        }
                        .disabled(password.isEmpty)

                        Button("Use API Key Instead") {
                            authMode = .apiKey
                        }
                        .font(.caption)

                    case .apiKey:
                        TextField("API Key (mk_...)", text: $apiKey)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)

                        Button("Connect with API Key") {
                            Task { await connectWithApiKey() }
                        }
                        .disabled(apiKey.isEmpty)

                        Button("Use Password Instead") {
                            authMode = .password
                        }
                        .font(.caption)
                    }
                }

                if authManager.isAuthenticating {
                    Section {
                        HStack {
                            ProgressView()
                            Text("Connecting...")
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .navigationTitle("Connect")
            .onAppear { bonjourBrowser.start() }
            .onDisappear { bonjourBrowser.stop() }
            .alert("Connection Error", isPresented: $showError) {
                Button("OK") {}
            } message: {
                Text(errorMessage)
            }
        }
    }

    // MARK: - Actions

    private func selectDiscovered(_ server: DiscoveredServer) {
        if let discoveredURL = server.url {
            serverURL = discoveredURL.absoluteString
        } else {
            serverURL = "https://\(server.host):\(server.port)"
        }
        serverName = server.name
        authMode = .check
        Task { await checkServer() }
    }

    private func checkServer() async {
        guard let url = URL(string: normalizeURL(serverURL)) else {
            showError(message: "Invalid URL")
            return
        }

        do {
            authStatus = try await authManager.checkStatus(url: url)
            if let status = authStatus {
                if status.authDisabled {
                    // No auth needed — connect directly with empty key
                    await connectWithApiKey()
                } else if status.setupRequired {
                    showError(message: "Server requires initial setup. Complete setup in the terminal first.")
                } else {
                    authMode = .password
                }
            }
        } catch {
            showError(message: error.localizedDescription)
        }
    }

    private func loginWithPassword() async {
        guard let url = URL(string: normalizeURL(serverURL)) else {
            showError(message: "Invalid URL")
            return
        }
        let name = serverName.isEmpty ? url.host ?? "Server" : serverName

        do {
            let server = try await authManager.loginAndCreateApiKey(
                serverURL: url, password: password, serverName: name
            )
            await connectionStore.connect(to: server, authManager: authManager)
        } catch {
            showError(message: error.localizedDescription)
        }
    }

    private func connectWithApiKey() async {
        guard let url = URL(string: normalizeURL(serverURL)) else {
            showError(message: "Invalid URL")
            return
        }
        let name = serverName.isEmpty ? url.host ?? "Server" : serverName

        do {
            let server = try await authManager.connectWithApiKey(
                serverURL: url, apiKey: apiKey, serverName: name
            )
            await connectionStore.connect(to: server, authManager: authManager)
        } catch {
            showError(message: error.localizedDescription)
        }
    }

    private func showError(message: String) {
        errorMessage = message
        showError = true
    }

    private func normalizeURL(_ input: String) -> String {
        var url = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if !url.hasPrefix("http://") && !url.hasPrefix("https://") {
            url = "https://\(url)"
        }
        return url
    }
}
