import SwiftUI

struct ConnectView: View {
    @Environment(\.openURL) private var openURL
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
    @State private var serverTrustStates: [String: ServerTrustState] = [:]

    enum AuthMode {
        case check
        case password
        case apiKey
    }

    enum ServerTrustState {
        case unknown
        case checking
        case trusted
        case needsCA
        case unavailable
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
                            VStack(alignment: .leading, spacing: 6) {
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
                                .buttonStyle(.plain)

                                switch trustState(for: server) {
                                case .checking:
                                    HStack(spacing: 8) {
                                        ProgressView()
                                            .scaleEffect(0.8)
                                        Text("Checking certificate trust...")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                case .needsCA:
                                    if server.caCertURL != nil {
                                        Button {
                                            downloadCACertificate(for: server)
                                        } label: {
                                            Label("Download CA Certificate", systemImage: "arrow.down.doc.fill")
                                        }
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.small)
                                    }
                                case .trusted, .unknown, .unavailable:
                                    EmptyView()
                                }
                            }
                        }
                    }

                    if needsCertificateTrustHelp {
                        Section("Trust This Certificate (iOS)") {
                            Text("Only required when the app says Download CA Certificate.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Button("Re-check Certificate Trust") {
                                Task { await refreshNearbyServerTrustStates(force: true) }
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)

                            Text("1. Tap Download CA Certificate for your server.")
                            Text("2. In Safari, allow the profile download.")
                            Text("3. Open Settings > General > VPN & Device Management, then install the downloaded profile.")
                            Text("4. Open Settings > General > About > Certificate Trust Settings.")
                            Text("5. Enable full trust for Moltis Local CA, then return and tap Check Connection.")
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
                        Button {
                            Task { await checkServer() }
                        } label: {
                            Text("Check Connection")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                        .disabled(serverURL.isEmpty || authManager.isAuthenticating)

                        Text("Remote access needs two server settings: password auth configured and GraphQL enabled.")
                            .font(.caption)
                            .foregroundStyle(.secondary)

                    case .password:
                        SecureField("Password", text: $password)
                            .textContentType(.password)

                        Button {
                            Task { await loginWithPassword() }
                        } label: {
                            Text("Login & Connect")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                        .disabled(password.isEmpty || authManager.isAuthenticating)

                        Button("Use API Key Instead") {
                            authMode = .apiKey
                        }
                        .font(.caption)

                    case .apiKey:
                        TextField("API Key (mk_...)", text: $apiKey)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)

                        Button {
                            Task { await connectWithApiKey() }
                        } label: {
                            Text("Connect with API Key")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                        .disabled(apiKey.isEmpty || authManager.isAuthenticating)

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
            .onAppear {
                bonjourBrowser.start()
                Task { await refreshNearbyServerTrustStates(force: true) }
            }
            .onChange(of: bonjourBrowser.servers.map(\.id)) { _, _ in
                Task { await refreshNearbyServerTrustStates(force: false) }
            }
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
                let graphQLEnabled: Bool?
                if let statusGraphQLEnabled = status.graphqlEnabled {
                    graphQLEnabled = statusGraphQLEnabled
                } else {
                    graphQLEnabled = await authManager.checkGraphQLEnabled(url: url)
                }
                if graphQLEnabled == false {
                    showError(
                        message: "GraphQL is disabled on this server. Enable it in Moltis (Settings > GraphQL), then check connection again."
                    )
                } else if status.setupRequired || !status.setupComplete {
                    showError(
                        message: "Server auth is not fully configured for remote access. On the Moltis host, set a password first, then try again from iOS."
                    )
                } else if !status.authDisabled && !status.hasPassword {
                    showError(
                        message: "This server has no password configured. The iOS companion currently requires password auth. Set a password in Moltis, then reconnect."
                    )
                } else if status.authDisabled {
                    // No auth needed — connect directly with empty key
                    await connectWithApiKey()
                } else {
                    authMode = .password
                }
            }
        } catch {
            if isCertificateTrustError(error) {
                showError(message: "TLS certificate is not trusted yet. Download and trust the Moltis Local CA for this server.")
                await refreshNearbyServerTrustStates(force: true)
                return
            }
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

    private var needsCertificateTrustHelp: Bool {
        bonjourBrowser.servers.contains { trustState(for: $0) == .needsCA }
    }

    private func trustState(for server: DiscoveredServer) -> ServerTrustState {
        serverTrustStates[server.id] ?? .unknown
    }

    private func downloadCACertificate(for server: DiscoveredServer) {
        guard let caCertURL = server.caCertURL else { return }
        openURL(caCertURL)
    }

    private func refreshNearbyServerTrustStates(force: Bool) async {
        let servers = bonjourBrowser.servers
        let visibleIDs = Set(servers.map(\.id))
        serverTrustStates = serverTrustStates.filter { visibleIDs.contains($0.key) }

        for server in servers {
            if !force {
                let existing = trustState(for: server)
                if existing == .trusted || existing == .needsCA || existing == .checking {
                    continue
                }
            }

            serverTrustStates[server.id] = .checking
            serverTrustStates[server.id] = await detectTrustState(for: server)
        }
    }

    private func detectTrustState(for server: DiscoveredServer) async -> ServerTrustState {
        guard let serverURL = server.url else { return .unavailable }

        var components = URLComponents(url: serverURL, resolvingAgainstBaseURL: false)
        components?.path = "/api/auth/status"

        guard let statusURL = components?.url else {
            return .unavailable
        }

        var request = URLRequest(url: statusURL)
        request.httpMethod = "GET"
        request.timeoutInterval = 4

        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            if response is HTTPURLResponse {
                return .trusted
            }
            return .unavailable
        } catch {
            if isCertificateTrustError(error) {
                return .needsCA
            }
            return .unavailable
        }
    }

    private func isCertificateTrustError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == NSURLErrorDomain else { return false }
        return [
            NSURLErrorServerCertificateHasBadDate,
            NSURLErrorServerCertificateUntrusted,
            NSURLErrorServerCertificateHasUnknownRoot,
            NSURLErrorServerCertificateNotYetValid,
            NSURLErrorSecureConnectionFailed,
        ].contains(nsError.code)
    }
}
