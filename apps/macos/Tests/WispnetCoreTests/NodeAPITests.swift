import Testing
import Foundation
import Darwin
@testable import WispnetCore

struct NodeAPITests {
    @Test func testBrowserHandoffRejectsCredentialsTokensAndNonlocalRoutes() throws {
        #expect(try browserURL("http://service.localhost:1234/").port == 1234)
        for value in ["https://service.localhost:1234/", "http://localhost:1234/", "http://evil.example:1234/",
                      "http://service.localhost:1234/?token=secret", "http://service.localhost:1234/#secret",
                      "http://user@service.localhost:1234/", "http://service.localhost/", "http://service.localhost:0/",
                      "http://service.localhost:1234/path", "file:///tmp/handoff.html"] {
            #expect(throws: (any Error).self) { try browserURL(value) }
        }
    }
    @Test func testOwnerOnlySocketAndProtocolValidation() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("wn-" + UUID().uuidString.prefix(8))
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let path = directory.appendingPathComponent("node.sock").path
        let api = NodeAPI(socketPath: path)
        #expect(throws: (any Error).self) { try api.request(["op": "status"]) }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        try #require(fd >= 0)
        defer { close(fd) }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let pathBytes = Array(path.utf8CString)
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            pathBytes.withUnsafeBytes { buffer.copyBytes(from: $0) }
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        #expect(bound == 0)
        #expect(listen(fd, 4) == 0)
        chmod(path, 0o666)
        do {
            _ = try api.request(["op": "status"])
            Issue.record("Accepted a public socket")
        } catch { #expect(error.localizedDescription.contains("owner-only")) }
        chmod(path, 0o600)
        #expect(throws: (any Error).self) { try api.request(["op": "pair", "invitation": String(repeating: "a", count: 16384)]) }
        for (reply, expected) in [
            ("{\"ok\":true,\"result\":{\"api_version\":99}}\n", "Unsupported"),
            ("{\"ok\":false,\"error\":\"denied\"}\n", "denied"),
            (String(repeating: "x", count: 2 * 1024 * 1024 + 1) + "\n", "oversized"),
        ] {
            let done = DispatchSemaphore(value: 0)
            DispatchQueue.global().async {
                let client = accept(fd, nil, nil)
                guard client >= 0 else { done.signal(); return }
                defer { close(client); done.signal() }
                var noSigPipe: Int32 = 1
                setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, 4)
                var request = [UInt8](repeating: 0, count: 16384)
                _ = read(client, &request, request.count)
                let bytes = Data(reply.utf8)
                bytes.withUnsafeBytes { buffer in
                    var offset = 0
                    while offset < buffer.count {
                        let count = write(client, buffer.baseAddress!.advanced(by: offset), buffer.count - offset)
                        guard count > 0 else { break }
                        offset += count
                    }
                }
            }
            do {
                _ = try api.request(["op": "status"])
                Issue.record("Accepted an invalid reply")
            } catch { #expect(error.localizedDescription.contains(expected)) }
            #expect(done.wait(timeout: .now() + 5) == .success)
        }
        // A symlink to an otherwise valid socket is never accepted.
        let alias = directory.appendingPathComponent("alias.sock").path
        #expect(symlink(path, alias) == 0)
        #expect(throws: (any Error).self) { try NodeAPI(socketPath: alias).request(["op": "status"]) }
    }
    @Test func testActiveCountOnlyIncludesApprovedPeersOnce() throws {
        let json = #"{"api_version":1,"host":{"host":"owner","revision":0,"clients":[{"peer":"a","name":"A"},{"peer":"b","name":"B"},{"peer":"c","name":"C"}],"services":[]},"host_name":"Mac","peer_pins":[],"connected_peers":{"a":3,"stranger":1},"recent_peers":{"a":1,"b":20,"stranger":1},"invitations":[],"relay":"connected","access_enabled":false}"#
        let status = try JSONDecoder().decode(NodeStatus.self, from: Data(json.utf8))
        #expect(status.activeClients == 2)
    }
}
