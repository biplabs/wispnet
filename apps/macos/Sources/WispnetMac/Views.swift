import AppKit
import SwiftUI
import WispnetCore

private enum Section: String, CaseIterable, Identifiable {
    case directory = "Services", local = "My services", devices = "Devices", pairing = "Pairing requests"
    var id: Self { self }
    var icon: String {
        switch self {
        case .directory: return "square.grid.2x2"
        case .local: return "desktopcomputer"
        case .devices: return "laptopcomputer.and.iphone"
        case .pairing: return "person.badge.key"
        }
    }
}
struct ContentView: View {
    @ObservedObject var model: Model
    @State private var section: Section? = .directory
    @State private var pair = false
    @State private var editor: ServiceDraft?
    @State private var approval: ApprovalDraft?

    var body: some View {
        NavigationSplitView {
            List(Section.allCases, selection: $section) { item in
                Label {
                    HStack {
                        Text(item.rawValue)
                        if item == .pairing, let count = model.status?.pending.count, count > 0 {
                            Spacer()
                            Text("\(count)").foregroundStyle(.secondary)
                        }
                    }
                } icon: { Image(systemName: item.icon) }.tag(item)
            }
            .navigationSplitViewColumnWidth(min: 190, ideal: 215)
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 6) {
                    Label(model.online ? "Online" : "Offline", systemImage: model.online ? "circle.fill" : "circle")
                        .foregroundStyle(model.online ? .green : .secondary)
                    Text(model.status.map { "Relay: \($0.relay) · \($0.activeClients) active" } ?? "Node unavailable")
                        .font(.caption).foregroundStyle(.secondary)
                }.padding().frame(maxWidth: .infinity, alignment: .leading)
            }
        } detail: {
            VStack(spacing: 0) {
                if let message = model.message {
                    HStack {
                        Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                        Text(message).textSelection(.enabled)
                        Spacer()
                        Button { model.message = nil } label: { Image(systemName: "xmark") }.buttonStyle(.plain)
                    }.padding().background(.orange.opacity(0.1))
                }
                if !model.online {
                    VStack(spacing: 16) {
                        Image(systemName: "network.slash").font(.system(size: 48)).foregroundStyle(.secondary)
                        Text("Your Mac is offline").font(.title2)
                        Text("Go online to see services and connect your devices.").foregroundStyle(.secondary)
                        Button("Go online") { model.setOnline(true) }.buttonStyle(.borderedProminent).disabled(model.busy)
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 24) {
                            header
                            switch section ?? .directory {
                            case .directory: directory
                            case .local: localServices
                            case .devices: devices
                            case .pairing: requests
                            }
                        }.padding(28).frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
            .background {
                Starfield(online: model.online, warpStartedAt: model.warpStartedAt, active: model.windowVisible)
            }
            .navigationTitle(section?.rawValue ?? "Wispnet")
            .toolbar {
                ToolbarItem(id: "progress") {
                    if model.busy { ProgressView().controlSize(.small) }
                }
                ToolbarItem(id: "refresh") {
                    Button { model.refresh(); model.refreshPeers() } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                        .disabled(model.busy)
                }
                ToolbarItem(id: "node-state") {
                    Button(model.online ? "Go offline" : "Go online") { model.setOnline(!model.online) }
                        .accessibilityLabel(model.online ? "Go offline" : "Go online").disabled(model.busy)
                }
            }
        }
        .groupBoxStyle(TransparentPanelStyle())
        .preferredColorScheme(.dark)
        .frame(minWidth: 850, minHeight: 580)
        .sheet(isPresented: $pair) { PairView(model: model) }
        .sheet(item: $model.qr) { qr in QRView(model: model, qr: qr) }
        .sheet(item: $editor) { draft in ServiceEditor(model: model, draft: draft) }
        .sheet(item: $approval) { draft in ApprovalView(model: model, draft: draft) }
    }
    private var header: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 6) {
                Text(model.status?.host_name ?? "This Mac").font(.largeTitle.bold())
                Text("Your services. Any device.").foregroundStyle(.secondary)
            }
            Spacer()
            Button("Pair a machine…") { pair = true }.disabled(model.busy)
            Button { model.generateQR() } label: { Label("Pair this machine", systemImage: "qrcode") }.disabled(model.busy)
        }
        .padding(18)
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(.cyan.opacity(0.18)))
    }
    private var directory: some View {
        VStack(alignment: .leading, spacing: 24) {
            if let status = model.status {
                serviceGroup("On this Mac", services: status.host.services, host: status.host.host)
                ForEach(status.peer_pins, id: \.self) { host in
                    VStack(alignment: .leading, spacing: 12) {
                        HStack {
                            Text(model.peerName(host)).font(.title3.bold())
                            if model.peers[host]?.refreshing == true { ProgressView().controlSize(.small) }
                            Spacer()
                            Button("Refresh") { model.refreshPeers([host]) }.disabled(model.busy || model.peers[host]?.refreshing == true)
                            Button("Forget…", role: .destructive) {
                                let revision = status.host.revision
                                if confirm("Forget this machine?", detail: host + "\nIts owner still controls any access granted to this Mac.") {
                                    model.perform(["op": "unpair", "host": host, "expected_revision": revision])
                                }
                            }.disabled(model.busy)
                        }
                        Text(host).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                        if let error = model.peers[host]?.error { Text(error).foregroundStyle(.secondary) }
                        else if let catalog = model.peers[host]?.catalog {
                            serviceGrid(catalog.services, host: host, enabled: model.peers[host]?.refreshing != true)
                        } else { Text("Fetching services…").foregroundStyle(.secondary) }
                    }
                }
                if status.peer_pins.isEmpty {
                    Label("Pair another machine to see its shared services here.", systemImage: "link").foregroundStyle(.secondary)
                }
            }
        }
    }
    private func serviceGroup(_ title: String, services: [Service], host: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.title3.bold())
            serviceGrid(services, host: host)
        }
    }
    private func serviceGrid(_ services: [Service], host: String, enabled: Bool = true) -> some View {
        Group {
            if services.isEmpty { Text("No shared services.").foregroundStyle(.secondary) }
            else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 235), spacing: 12)], spacing: 12) {
                    ForEach(services) { service in
                        Button { model.open(service, host: host) } label: {
                            HStack(spacing: 14) {
                                Image(systemName: service.kind == "http" ? "globe" : "terminal").font(.title2).foregroundStyle(.tint)
                                VStack(alignment: .leading, spacing: 5) {
                                    Text(service.title).font(.headline)
                                    Text(service.kind == "http" ? "Open in browser" : "TCP · use the terminal client").font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer(minLength: 0)
                                Image(systemName: "arrow.up.right").foregroundStyle(.secondary)
                            }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(RoundedRectangle(cornerRadius: 12))
                        }.buttonStyle(.plain)
                            .overlay(RoundedRectangle(cornerRadius: 12).stroke(.cyan.opacity(0.18)))
                            .disabled(model.busy || !enabled || service.kind != "http")
                    }
                }
            }
        }
    }
    private var localServices: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Share a service running on this Mac. You choose which devices can open it.").foregroundStyle(.secondary)
                Spacer()
                Button("Share service…") {
                    if let status = model.status { editor = ServiceDraft(revision: status.host.revision) }
                }.buttonStyle(.borderedProminent).disabled(model.busy)
            }
            ForEach(model.status?.host.services ?? []) { service in
                GroupBox {
                    HStack {
                        VStack(alignment: .leading, spacing: 5) {
                            Text(service.title).font(.headline)
                            Text("\(service.backend ?? "") · \(service.kind.uppercased()) · \(service.allow?.count ?? 0) allowed devices")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Open") { if let host = model.status?.host.host { model.open(service, host: host) } }.disabled(service.kind != "http")
                        Button("Edit…") {
                            if let status = model.status { editor = ServiceDraft(service: service, revision: status.host.revision) }
                        }
                        Button("Remove…", role: .destructive) {
                            if let revision = model.status?.host.revision,
                               confirm("Remove \(service.title)?", detail: "This stops access through Wispnet. The local service keeps running.") {
                                model.mutate(["op": "remove", "id": service.id], revision: revision)
                            }
                        }
                    }.padding(8)
                }.disabled(model.busy)
            }
            if model.status?.host.services.isEmpty == true { Text("No services shared yet.").foregroundStyle(.secondary) }
        }
    }
    private var devices: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let status = model.status {
                Text("\(status.host.clients.count) approved · \(status.activeClients) active").font(.title3.bold())
                Text("Active means connected now or seen within the last 60 seconds.").foregroundStyle(.secondary)
                ForEach(status.host.clients) { client in
                    GroupBox {
                        HStack {
                            Image(systemName: "laptopcomputer.and.iphone").font(.title2).padding(8)
                            VStack(alignment: .leading, spacing: 5) {
                                Text(client.name).font(.headline)
                                Text(client.peer).font(.caption.monospaced()).textSelection(.enabled)
                                Text(activity(client.peer, status)).font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button("Revoke…", role: .destructive) {
                                let revision = status.host.revision
                                if confirm("Revoke \(client.name)?", detail: client.peer + "\nThis removes this device’s access to services on your Mac.") {
                                    model.mutate(["op": "revoke", "peer": client.peer], revision: revision)
                                }
                            }.disabled(model.busy)
                        }.padding(8)
                    }
                }
                if status.host.clients.isEmpty { Text("No approved devices. Use Pair this machine to connect your phone.").foregroundStyle(.secondary) }
                Divider()
                Text("This Mac’s identity").font(.headline)
                Text(status.host.host).font(.caption.monospaced()).textSelection(.enabled)
            }
        }
    }
    private func activity(_ peer: String, _ status: NodeStatus) -> String {
        if (status.connected_peers[peer] ?? 0) > 0 { return "Connected" }
        if let age = status.recent_peers[peer] { return "Seen \(Int(age)) seconds ago" }
        return "Idle"
    }
    private var requests: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Compare the full device identity before approving. Each machine controls its own service permissions.")
                .foregroundStyle(.secondary)
            if let status = model.status {
                ForEach(status.pending) { invitation in
                    GroupBox {
                        VStack(alignment: .leading, spacing: 10) {
                            Text(invitation.name ?? "New device").font(.headline)
                            Text(invitation.peer ?? "").font(.caption.monospaced()).textSelection(.enabled)
                            HStack {
                                Button("Approve…") { approval = ApprovalDraft(invitation: invitation, status: status) }
                                    .buttonStyle(.borderedProminent)
                                Button("Reject…", role: .destructive) {
                                    let revision = status.host.revision
                                    if confirm("Reject this request?", detail: invitation.peer ?? "") {
                                        model.mutate(["op": "reject_invitation", "invitation": invitation.invitation], revision: revision)
                                    }
                                }
                            }.disabled(model.busy)
                        }.padding(8).frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                if status.pending.isEmpty { Text("No pending requests.").foregroundStyle(.secondary) }
            }
        }
    }
}

@MainActor func confirm(_ title: String, detail: String) -> Bool {
    let alert = NSAlert()
    alert.messageText = title
    alert.informativeText = detail
    alert.addButton(withTitle: "Continue")
    alert.addButton(withTitle: "Cancel")
    return alert.runModal() == .alertFirstButtonReturn
}

private struct TransparentPanelStyle: GroupBoxStyle {
    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            configuration.label
            configuration.content
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(.cyan.opacity(0.18)))
    }
}
