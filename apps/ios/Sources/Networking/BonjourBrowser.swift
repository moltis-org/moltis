import Foundation
import Network
import os

struct DiscoveredServer: Identifiable, Hashable {
    let id: String
    let name: String
    let host: String
    let port: UInt16
    let version: String?

    var url: URL? {
        var components = URLComponents()
        components.scheme = "https"
        components.host = host
        components.port = Int(port)
        return components.url
    }

    var caCertURL: URL? {
        guard port < UInt16.max else { return nil }
        var components = URLComponents()
        components.scheme = "http"
        components.host = host
        components.port = Int(port) + 1
        components.path = "/certs/ca.pem"
        return components.url
    }
}

@MainActor
final class BonjourBrowser: ObservableObject {
    @Published private(set) var servers: [DiscoveredServer] = []

    private let logger = Logger(subsystem: "org.moltis.ios", category: "bonjour")
    private var browser: NWBrowser?
    private var connections: [String: NWConnection] = [:]

    func start() {
        let params = NWParameters()
        params.includePeerToPeer = true

        let browser = NWBrowser(
            for: .bonjourWithTXTRecord(type: "_moltis._tcp", domain: nil),
            using: params
        )

        browser.browseResultsChangedHandler = { [weak self] results, _ in
            Task { @MainActor [weak self] in
                self?.handleResults(results)
            }
        }

        browser.stateUpdateHandler = { state in
            switch state {
            case .failed(let error):
                self.logger.error("Bonjour browse failed: \(error.localizedDescription, privacy: .public)")
            default:
                break
            }
        }

        browser.start(queue: .main)
        self.browser = browser
    }

    func stop() {
        browser?.cancel()
        browser = nil
        for conn in connections.values {
            conn.cancel()
        }
        connections.removeAll()
        servers.removeAll()
    }

    private func handleResults(_ results: Set<NWBrowser.Result>) {
        var seen = Set<String>()

        for result in results {
            guard case .service(let name, _, _, _) = result.endpoint else { continue }

            seen.insert(name)

            // Already resolved — skip.
            if servers.contains(where: { $0.id == name }) { continue }

            let txtRecord: NWTXTRecord? = {
                if case .bonjour(let txt) = result.metadata { return txt }
                return nil
            }()

            let version = Self.txtValue(for: "version", in: txtRecord)
            let advertisedHostname = Self.txtValue(for: "hostname", in: txtRecord)

            resolve(
                endpoint: result.endpoint,
                name: name,
                version: version,
                advertisedHostname: advertisedHostname
            )
        }

        // Remove servers that disappeared.
        servers.removeAll { !seen.contains($0.id) }
    }

    private func resolve(
        endpoint: NWEndpoint,
        name: String,
        version: String?,
        advertisedHostname: String?
    ) {
        let conn = NWConnection(to: endpoint, using: .tcp)

        conn.stateUpdateHandler = { [weak self] state in
            guard case .ready = state else { return }

            if let innerEndpoint = conn.currentPath?.remoteEndpoint,
               case .hostPort(let host, let port) = innerEndpoint {
                let hostString = Self.preferredHost(
                    advertisedHostname: advertisedHostname,
                    resolvedHost: host
                )

                let server = DiscoveredServer(
                    id: name,
                    name: name,
                    host: hostString,
                    port: port.rawValue,
                    version: version
                )

                Task { @MainActor [weak self] in
                    guard let self else { return }
                    if !self.servers.contains(where: { $0.id == name }) {
                        self.servers.append(server)
                    }
                }
            }

            conn.cancel()
            Task { @MainActor [weak self] in
                self?.connections.removeValue(forKey: name)
            }
        }

        connections[name] = conn
        conn.start(queue: .main)
    }

    nonisolated private static func txtValue(for key: String, in record: NWTXTRecord?) -> String? {
        guard let record, let entry = record.getEntry(for: key) else { return nil }
        switch entry {
        case .string(let value):
            return value
        case .data(let data):
            return String(data: data, encoding: .utf8)
        case .none, .empty:
            return nil
        @unknown default:
            return nil
        }
    }

    nonisolated private static func preferredHost(
        advertisedHostname: String?,
        resolvedHost: NWEndpoint.Host
    ) -> String {
        if let host = normalizedAdvertisedHostname(advertisedHostname) {
            return host
        }
        return normalizedResolvedHost(rawHostString(for: resolvedHost))
    }

    nonisolated private static func normalizedAdvertisedHostname(_ hostname: String?) -> String? {
        guard var host = hostname?.trimmingCharacters(in: .whitespacesAndNewlines),
              !host.isEmpty else {
            return nil
        }

        if host.hasSuffix(".") {
            host.removeLast()
        }

        if !host.contains(".") {
            host += ".local"
        }

        return host
    }

    nonisolated private static func rawHostString(for host: NWEndpoint.Host) -> String {
        switch host {
        case .ipv4(let addr): return "\(addr)"
        case .ipv6(let addr): return "\(addr)"
        case .name(let value, _): return value
        @unknown default: return "\(host)"
        }
    }

    nonisolated private static func normalizedResolvedHost(_ host: String) -> String {
        var value = host.trimmingCharacters(in: .whitespacesAndNewlines)
        if let scopeIndex = value.firstIndex(of: "%") {
            value = String(value[..<scopeIndex])
        }
        if value.hasSuffix(".") {
            value.removeLast()
        }
        return value
    }
}
