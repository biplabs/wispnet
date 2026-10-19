import Foundation
import Darwin

public struct NodeError: LocalizedError {
    public let message: String
    public init(_ message: String) { self.message = message }
    public var errorDescription: String? { message }
}

/// Owner-only IPC to the existing node. No second identity or network endpoint.
public struct NodeAPI: Sendable {
    public let socketPath: String
    public init(socketPath: String) { self.socketPath = socketPath }

    public func request(_ payload: [String: Any]) throws -> Data {
        var metadata = stat()
        guard lstat(socketPath, &metadata) == 0 else {
            throw NodeError("Node offline. Choose Go online to start it.")
        }
        guard metadata.st_mode & S_IFMT == S_IFSOCK,
              metadata.st_uid == geteuid(), metadata.st_mode & 0o077 == 0 else {
            throw NodeError("Node socket must be an owner-only socket.")
        }
        var bytes = try JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
        bytes.append(10)
        guard bytes.count <= 16384 else { throw NodeError("Request exceeds 16 KiB.") }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw NodeError("Cannot create local socket.") }
        defer { close(fd) }
        var timeout = timeval(tv_sec: 27, tv_usec: 0)
        guard setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout))) == 0,
              setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout))) == 0 else {
            throw NodeError("Cannot set socket timeout.")
        }
        var noSigPipe: Int32 = 1
        guard setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size)) == 0 else {
            throw NodeError("Cannot configure local socket.")
        }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let path = Array(socketPath.utf8CString)
        guard path.count <= MemoryLayout.size(ofValue: address.sun_path) else {
            throw NodeError("Node socket path is too long.")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            path.withUnsafeBytes { buffer.copyBytes(from: $0) }
        }
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { throw NodeError("Cannot connect to the local node.") }
        var uid: uid_t = 0
        var gid: gid_t = 0
        guard getpeereid(fd, &uid, &gid) == 0, uid == geteuid() else {
            throw NodeError("Unexpected local node owner.")
        }
        try bytes.withUnsafeBytes { buffer in
            var offset = 0
            while offset < buffer.count {
                let count = Darwin.write(fd, buffer.baseAddress!.advanced(by: offset), buffer.count - offset)
                if count < 0 && errno == EINTR { continue }
                guard count > 0 else { throw NodeError("Request unconfirmed. Refresh before retrying.") }
                offset += count
            }
        }
        var reply = Data()
        var buffer = [UInt8](repeating: 0, count: 65536)
        while reply.count <= 2 * 1024 * 1024 {
            let count = Darwin.read(fd, &buffer, buffer.count)
            if count < 0 && errno == EINTR { continue }
            guard count > 0 else { throw NodeError("Node reply unconfirmed. Refresh before retrying.") }
            reply.append(contentsOf: buffer.prefix(count))
            if reply.last == 10 { break }
        }
        guard reply.count <= 2 * 1024 * 1024, reply.last == 10,
              let envelope = try JSONSerialization.jsonObject(with: reply) as? [String: Any],
              let ok = envelope["ok"] as? Bool else { throw NodeError("Invalid or oversized node reply.") }
        guard ok else { throw NodeError(envelope["error"] as? String ?? "Node rejected the operation.") }
        guard let result = envelope["result"] else { throw NodeError("Missing node result.") }
        if payload["op"] as? String == "status" {
            guard (result as? [String: Any])?["api_version"] as? Int == 1 else {
                throw NodeError("Unsupported node API version.")
            }
        }
        return try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys, .fragmentsAllowed])
    }
}

public func browserURL(_ value: String) throws -> URL {
    guard let parts = URLComponents(string: value), parts.scheme == "http",
          let host = parts.host, host.hasSuffix(".localhost"), host.count > 10,
          let port = parts.port, (1...65535).contains(port), parts.path == "/",
          parts.query == nil, parts.fragment == nil, parts.user == nil, parts.password == nil,
          let url = parts.url else { throw NodeError("Invalid local browser handoff.") }
    return url
}

public struct NodeLifecycle: Sendable {
    public let home: URL
    public init(home: URL = FileManager.default.homeDirectoryForCurrentUser) { self.home = home }
    public var service: String { "gui/\(geteuid())/app.wispnet.node" }
    public var plist: URL { home.appendingPathComponent("Library/LaunchAgents/app.wispnet.node.plist") }

    public func setOnline(_ online: Bool) throws {
        let loaded = (try? run(["print", service])) != nil
        if online {
            _ = try run(loaded ? ["kickstart", service] : ["bootstrap", "gui/\(geteuid())", plist.path])
        } else if loaded {
            _ = try run(["bootout", service])
        }
    }
    private func run(_ arguments: [String]) throws -> Data {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = arguments
        // Use files so a verbose launchctl cannot block on a full pipe.
        let output = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        _ = FileManager.default.createFile(atPath: output.path, contents: nil, attributes: [.posixPermissions: 0o600])
        defer { try? FileManager.default.removeItem(at: output) }
        let handle = try FileHandle(forWritingTo: output)
        defer { try? handle.close() }
        process.standardOutput = handle
        process.standardError = handle
        try process.run()
        let deadline = Date().addingTimeInterval(30)
        while process.isRunning && Date() < deadline { Thread.sleep(forTimeInterval: 0.05) }
        if process.isRunning {
            kill(process.processIdentifier, SIGKILL)
            process.waitUntilExit()
            throw NodeError("Node control timed out. Refresh its status before retrying.")
        }
        process.waitUntilExit()
        let data = try Data(contentsOf: output)
        guard process.terminationStatus == 0 else {
            throw NodeError(String(data: data.prefix(4096), encoding: .utf8) ?? "Could not change node state.")
        }
        return data
    }
}
