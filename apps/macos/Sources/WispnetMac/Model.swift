import AppKit
import SwiftUI
import WispnetCore

struct PeerViewState {
    var catalog: Catalog?
    var error: String?
    var refreshing = false
}

@MainActor final class Model: ObservableObject {
    let api: NodeAPI
    @Published var status: NodeStatus?
    @Published var online = false
    @Published var busy = false
    @Published var message: String?
    @Published var peers: [String: PeerViewState] = [:]
    @Published var qr: PairingQR?
    @Published var names: [String: String] = UserDefaults.standard.dictionary(forKey: "peerNames") as? [String: String] ?? [:]
    var onStatus: (() -> Void)?
    private var polling = false
    private var lastCatalogRefresh = Date.distantPast
    private var timer: Timer?

    init() {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let path = Bundle.main.object(forInfoDictionaryKey: "WispnetStateDirectory") as? String
            ?? home.appendingPathComponent(".local/state/wispnet-node").path
        api = NodeAPI(socketPath: path + "/node.sock")
    }
    func start() {
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }
    func refresh() {
        guard !polling, !busy else { return }
        polling = true
        Task {
            defer { polling = false; onStatus?() }
            do {
                let api = self.api
                let data = try await Task.detached { try api.request(["op": "status", "refresh_clients": true]) }.value
                let result = try JSONDecoder().decode(NodeStatus.self, from: data)
                status = result
                online = true
                peers = peers.filter { result.peer_pins.contains($0.key) }
                if Date().timeIntervalSince(lastCatalogRefresh) >= 15 {
                    lastCatalogRefresh = Date()
                    refreshPeers(result.peer_pins)
                }
            } catch {
                online = false
                status = nil
                peers = [:]
            }
        }
    }
    func refreshPeers(_ hosts: [String]? = nil) {
        let hosts = hosts ?? status?.peer_pins ?? []
        // Each host updates independently; cap simultaneous local API requests.
        Task {
            await withTaskGroup(of: Void.self) { group in
                var iterator = hosts.makeIterator()
                for _ in 0..<min(4, hosts.count) {
                    if let host = iterator.next() { group.addTask { await self.refreshPeer(host) } }
                }
                while await group.next() != nil {
                    if let host = iterator.next() { group.addTask { await self.refreshPeer(host) } }
                }
            }
        }
    }
    private func refreshPeer(_ host: String) async {
        guard peers[host]?.refreshing != true else { return }
        peers[host, default: PeerViewState()].refreshing = true
        do {
            let api = self.api
            let data = try await Task.detached { try api.request(["op": "catalog", "host": host]) }.value
            let catalog = try JSONDecoder().decode(Catalog.self, from: data)
            guard catalog.host == host else { throw NodeError("Wrong catalog host.") }
            guard online, status?.peer_pins.contains(host) == true else { return }
            peers[host] = PeerViewState(catalog: catalog)
        } catch {
            guard online, status?.peer_pins.contains(host) == true else { return }
            let reason = error.localizedDescription
            peers[host] = PeerViewState(error: reason.localizedCaseInsensitiveContains("awaiting approval") ? "Awaiting approval" : reason)
        }
    }
    func perform(_ payload: [String: Any], done: @escaping (Data) throws -> Void = { _ in }) {
        guard !busy else { return }
        busy = true
        message = nil
        Task {
            defer { busy = false; refresh(); onStatus?() }
            do {
                let api = self.api
                let data = try await Task.detached { try api.request(payload) }.value
                try done(data)
            } catch { message = error.localizedDescription }
        }
    }
    func mutate(_ operation: [String: Any], revision: UInt64) {
        var operation = operation
        operation["expected_revision"] = revision
        perform(["op": "mutate", "operation": operation])
    }
    func setOnline(_ online: Bool) {
        guard !busy else { return }
        busy = true
        message = nil
        Task {
            defer { busy = false; refresh(); onStatus?() }
            do {
                let api = self.api
                try await Task.detached {
                    try NodeLifecycle().setOnline(online)
                    if online {
                        let deadline = Date().addingTimeInterval(10)
                        while true {
                            if (try? api.request(["op": "status"])) != nil { break }
                            guard Date() < deadline else { throw NodeError("Node started but its API is not ready.") }
                            try await Task.sleep(nanoseconds: 100_000_000)
                        }
                    }
                }.value
                if !online { self.online = false; status = nil; peers = [:] }
                lastCatalogRefresh = .distantPast
            } catch { message = error.localizedDescription }
        }
    }
    func open(_ service: Service, host: String) {
        guard online, !busy, service.kind == "http", let status = status else { return }
        let payload = status.openRequest(for: service, on: host, privateNames: names)
        busy = true
        message = nil
        Task {
            defer { busy = false; refresh() }
            do {
                let api = self.api
                let data = try await Task.detached {
                    _ = try api.request(["op": "enable"])
                    return try api.request(payload)
                }.value
                let result = try JSONSerialization.jsonObject(with: data) as? [String: Any]
                let url = try browserURL(result?["url"] as? String ?? "")
                guard NSWorkspace.shared.open(url) else { throw NodeError("Could not open your browser.") }
            } catch { message = error.localizedDescription }
        }
    }
    func generateQR() {
        perform(["op": "invite_qr"]) { data in
            self.qr = try JSONDecoder().decode(PairingQR.self, from: data)
        }
    }
    func peerName(_ host: String) -> String {
        status?.hostName(for: host, privateNames: names) ?? String(host.prefix(12))
    }
    func saveName(_ name: String, host: String) {
        names[host] = name.isEmpty ? nil : name
        UserDefaults.standard.set(names, forKey: "peerNames")
    }
}
