import Foundation

public struct Service: Decodable, Identifiable, Sendable {
    public let id: String
    public let incarnation: String
    public let display_name: String
    public let kind: String
    public let backend: String?
    public let allow: [String]?
    public var title: String { display_name.isEmpty ? id : display_name }
}
public struct Client: Decodable, Identifiable, Sendable {
    public let peer: String
    public let name: String
    public var id: String { peer }
}
public struct Invitation: Decodable, Identifiable, Sendable {
    public let invitation: String
    public let peer: String?
    public let name: String?
    public let state: String
    public let mutual: Bool?
    public var id: String { invitation }
}
public struct Host: Decodable, Sendable {
    public let host: String
    public let revision: UInt64
    public let clients: [Client]
    public let services: [Service]
}
public struct NodeStatus: Decodable, Sendable {
    public let api_version: Int
    public let host: Host
    public let host_name: String
    public let peer_pins: [String]
    public let connected_peers: [String: Int]
    public let recent_peers: [String: Double]
    public let invitations: [Invitation]
    public let relay: String
    public let access_enabled: Bool
    public var pending: [Invitation] { invitations.filter { $0.state == "pending" } }
    public var activeClients: Int {
        host.clients.filter { (connected_peers[$0.peer] ?? 0) > 0 || recent_peers[$0.peer] != nil }.count
    }
}
public struct Catalog: Decodable, Sendable {
    public let host: String
    public let services: [Service]
}
public struct PairingQR: Decodable, Identifiable, Sendable {
    public let host: String
    public let invitation: String
    public let expires_at: Double
    public let matrix: [[Bool]]
    public let code: String
    public var id: String { invitation }
    public func isUsable(at now: Double, invitations: [Invitation]) -> Bool {
        now < expires_at && !invitations.contains {
            $0.invitation == invitation && $0.state != "issued" && $0.state != "challenged"
        }
    }
}

public func normalizedPairingCode(_ input: String) throws -> String {
    let code = input.filter { !$0.isWhitespace }
    guard code.hasPrefix("wispnet1:"), code.utf8.count <= 8192 else {
        throw NodeError("Enter a valid Wispnet pairing code.")
    }
    return code
}
