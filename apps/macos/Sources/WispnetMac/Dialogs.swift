import AppKit
import SwiftUI
import WispnetCore

struct ServiceDraft: Identifiable {
    let id = UUID()
    var serviceID = ""
    var name = ""
    var backend = "127.0.0.1:8080"
    var kind = "http"
    var allow = Set<String>()
    var revision: UInt64
    var existing = false
    init(revision: UInt64) { self.revision = revision }
    init(service: Service, revision: UInt64) {
        serviceID = service.id; name = service.display_name; backend = service.backend ?? ""
        kind = service.kind; allow = Set(service.allow ?? []); self.revision = revision; existing = true
    }
}
struct ServiceEditor: View {
    @ObservedObject var model: Model
    @State var draft: ServiceDraft
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text(draft.existing ? "Edit service" : "Share a service").font(.title2.bold())
            Form {
                TextField("Service ID", text: $draft.serviceID).disabled(draft.existing)
                TextField("Display name", text: $draft.name)
                TextField("Loopback address", text: $draft.backend)
                Picker("Type", selection: $draft.kind) {
                    Text("Web / HTTP").tag("http")
                    Text("TCP").tag("tcp")
                }
            }
            Text("Use a loopback address such as 127.0.0.1:8080. Only selected devices can access this service.")
                .font(.callout).foregroundStyle(.secondary)
            Text("Allowed devices").font(.headline)
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(model.status?.host.clients ?? []) { client in
                        Toggle(isOn: Binding(get: { draft.allow.contains(client.peer) }, set: {
                            if $0 { draft.allow.insert(client.peer) } else { draft.allow.remove(client.peer) }
                        })) { Text(client.name + " · " + client.peer.prefix(12)) }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(height: 110)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Save service") {
                    if confirm("Save this service and its allowed devices?", detail: "The selected list replaces the existing permissions for this service.") {
                        model.mutate(["op": "service", "id": draft.serviceID, "display_name": draft.name.isEmpty ? draft.serviceID : draft.name,
                                      "backend": draft.backend, "kind": draft.kind, "allow": Array(draft.allow).sorted()], revision: draft.revision)
                        dismiss()
                    }
                }.keyboardShortcut(.defaultAction).disabled(draft.serviceID.isEmpty || draft.backend.isEmpty || model.busy)
            }
        }.padding(24).frame(width: 530)
    }
}

struct ApprovalDraft: Identifiable {
    var id: String { invitation.invitation }
    let invitation: Invitation
    let status: NodeStatus
}
struct ApprovalView: View {
    @ObservedObject var model: Model
    let draft: ApprovalDraft
    @State private var selected = Set<String>()
    @Environment(\.dismiss) private var dismiss
    private var existing: Set<String> {
        Set(draft.status.host.services.filter { $0.allow?.contains(draft.invitation.peer ?? "") == true }.map(\.id))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Approve \(draft.invitation.name ?? "device")").font(.title2.bold())
            Text(draft.invitation.peer ?? "").font(.system(.caption, design: .monospaced)).textSelection(.enabled)
            Text(draft.invitation.mutual == true
                 ? "Both machines will be paired after approval. Each machine grants access to its own services independently."
                 : "Choose which services this device may access on your Mac.").foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(draft.status.host.services) { service in
                        Toggle(service.title, isOn: Binding(get: { selected.contains(service.id) || existing.contains(service.id) }, set: {
                            if $0 { selected.insert(service.id) } else { selected.remove(service.id) }
                        })).disabled(existing.contains(service.id))
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(height: 150)
            Text("Existing grants are preserved. Selecting no services approves pairing without adding service access.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Approve device") {
                    model.mutate(["op": "approve_invitation", "invitation": draft.invitation.invitation,
                                  "services": Array(selected.union(existing)).sorted()], revision: draft.status.host.revision)
                    dismiss()
                }.keyboardShortcut(.defaultAction).disabled(model.busy)
            }
        }.padding(24).frame(width: 560)
    }
}

struct PairView: View {
    @ObservedObject var model: Model
    @State private var code = ""
    @State private var name = ""
    @State private var host: String?
    @State private var document = ""
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Pair another machine").font(.title2.bold())
            if let host = host {
                Text("Compare this full identity with the other machine before requesting access.")
                Text(host).font(.system(.callout, design: .monospaced)).textSelection(.enabled)
                TextField("Private display name (optional)", text: $name)
                Text("After its owner approves, the machines will be paired. Each owner chooses their own service permissions.")
                    .font(.callout).foregroundStyle(.secondary)
            } else {
                Text("Paste the pairing code from the remote machine.").foregroundStyle(.secondary)
                TextEditor(text: $code)
                    .font(.system(.callout, design: .monospaced))
                    .frame(height: 132)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.quaternary))
                    .accessibilityLabel("Pairing code")
            }
            if let message = model.message { Text(message).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Spacer()
                Button("Cancel") { code = ""; document = ""; dismiss() }.keyboardShortcut(.cancelAction)
                Button(host == nil ? "Pair" : "Request pairing") {
                    if let host = host {
                        model.perform(["op": "pair", "host": host, "invitation": document]) { _ in
                            model.saveName(name, host: host)
                            model.peers[host] = PeerViewState(error: "Awaiting approval")
                            model.message = "Pairing requested. Approve it on the other machine, then refresh services."
                            document = ""
                            dismiss()
                        }
                    } else {
                        do {
                            let input = try normalizedPairingCode(code)
                            model.perform(["op": "preview_invitation", "invitation": input]) { data in
                                let result = try JSONSerialization.jsonObject(with: data) as? [String: Any]
                                guard let identity = result?["host"] as? String else { throw NodeError("Invalid invitation preview.") }
                                document = input; code = ""; host = identity
                                name = result?["host_name"] as? String ?? ""
                            }
                        } catch { model.message = error.localizedDescription }
                    }
                }.keyboardShortcut(.defaultAction).disabled(model.busy || (host == nil && code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
            }
        }.padding(24).frame(width: 560)
    }
}

struct QRView: View {
    @ObservedObject var model: Model
    let qr: PairingQR
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Pair with this Mac").font(.title2.bold())
            Text("Scan this QR on your phone or copy the pairing code to another computer. Then approve the request and choose allowed services here.")
                .foregroundStyle(.secondary)
            Text(qr.host).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let remaining = max(0, Int(qr.expires_at - context.date.timeIntervalSince1970))
                let used = model.status?.invitations.contains {
                    $0.invitation == qr.invitation && $0.state != "issued" && $0.state != "challenged"
                } == true
                VStack(spacing: 12) {
                    if qr.isUsable(at: context.date.timeIntervalSince1970, invitations: model.status?.invitations ?? []) {
                        Canvas { context, size in
                            let count = qr.matrix.count
                            guard count > 0, count <= 256, qr.matrix.allSatisfy({ $0.count == count }) else { return }
                            let scale = floor(min(size.width, size.height) / CGFloat(count))
                            let origin = CGPoint(x: (size.width - CGFloat(count) * scale) / 2, y: (size.height - CGFloat(count) * scale) / 2)
                            context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white))
                            for (y, row) in qr.matrix.enumerated() {
                                for (x, dark) in row.enumerated() where dark {
                                    let rect = CGRect(x: origin.x + CGFloat(x) * scale, y: origin.y + CGFloat(y) * scale, width: scale, height: scale)
                                    context.fill(Path(rect), with: .color(.black))
                                }
                            }
                        }.frame(width: 320, height: 320).accessibilityLabel("Pairing QR code")
                        ScrollView {
                            Text(qr.code).font(.system(.caption, design: .monospaced))
                                .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading).padding(8)
                        }.frame(height: 88).background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 6))
                            .accessibilityLabel("Pairing code")
                        Button("Copy pairing code") {
                            guard qr.isUsable(at: Date().timeIntervalSince1970, invitations: model.status?.invitations ?? []) else { return }
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(qr.code, forType: .string)
                        }
                        Text("Expires in \(remaining / 60):\(String(format: "%02d", remaining % 60))").monospacedDigit()
                    } else {
                        Image(systemName: used ? "checkmark.circle" : "clock").font(.system(size: 48)).frame(width: 400, height: 200)
                        Text(used ? "Request received. Close to review it." : "QR expired. Close and generate a new code.")
                    }
                }.frame(maxWidth: .infinity)
            }
            HStack { Spacer(); Button("Close") { dismiss() }.keyboardShortcut(.cancelAction) }
        }.padding(24).frame(width: 560)
    }
}
