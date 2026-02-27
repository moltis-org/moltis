import Foundation
import Network

struct DiscoveredServer: Identifiable, Hashable {
    let id: String
    let name: String
    let host: String
    let port: UInt16
    let version: String?

    var url: URL? {
        URL(string: "https://\(host):\(port)")
    }
}

@MainActor
final class BonjourBrowser: ObservableObject {
    @Published private(set) var servers: [DiscoveredServer] = []

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
                print("[BonjourBrowser] failed: \(error)")
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

            let version = txtRecord.flatMap { record in
                record.getEntry(for: "version").flatMap { entry in
                    if case .string(let v) = entry { return v }
                    return nil
                }
            }

            resolve(endpoint: result.endpoint, name: name, version: version)
        }

        // Remove servers that disappeared.
        servers.removeAll { !seen.contains($0.id) }
    }

    private func resolve(endpoint: NWEndpoint, name: String, version: String?) {
        let conn = NWConnection(to: endpoint, using: .tcp)

        conn.stateUpdateHandler = { [weak self] state in
            guard case .ready = state else { return }

            if let innerEndpoint = conn.currentPath?.remoteEndpoint,
               case .hostPort(let host, let port) = innerEndpoint {
                let hostString: String = {
                    switch host {
                    case .ipv4(let addr): return "\(addr)"
                    case .ipv6(let addr): return "\(addr)"
                    case .name(let h, _): return h
                    @unknown default: return "\(host)"
                    }
                }()

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
}
