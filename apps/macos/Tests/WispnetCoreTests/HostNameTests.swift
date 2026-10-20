import Foundation
import Testing
@testable import WispnetCore

struct HostNameTests {
    @Test func openRequestsUseNamesBoundToTheSelectedHost() throws {
        let json = #"{"api_version":1,"host":{"host":"mac","revision":1,"clients":[{"peer":"desktop","name":"j-desktop"},{"peer":"other","name":"Other computer"}],"services":[]},"host_name":"My Mac","peer_pins":["desktop","other"],"connected_peers":{},"recent_peers":{},"invitations":[],"relay":"connected","access_enabled":true}"#
        let status = try JSONDecoder().decode(NodeStatus.self, from: Data(json.utf8))
        let service = Service(id: "llama", incarnation: "incarnation", display_name: "Llama", kind: "http", backend: nil, allow: nil)
        let remote = status.openRequest(for: service, on: "desktop")
        #expect(remote == ["op": "open", "host": "desktop", "service": "llama", "incarnation": "incarnation", "host_name": "j-desktop"])
        #expect(status.openRequest(for: service, on: "mac", privateNames: ["mac": "Wrong alias"])["host_name"] == "My Mac")
        #expect(status.openRequest(for: service, on: "desktop", privateNames: ["desktop": "Home Desktop"])["host_name"] == "Home Desktop")
        #expect(status.hostName(for: "desktop", privateNames: ["desktop": " \n"]) == "j-desktop")
        #expect(status.hostName(for: "other") == "Other computer")
        #expect(status.hostName(for: "unknown") == nil)
    }
}
