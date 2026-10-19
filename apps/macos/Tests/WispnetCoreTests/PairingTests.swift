import Foundation
import Testing
@testable import WispnetCore

struct PairingTests {
    @Test func visibleCodesAcceptWrappedPasteAndRejectFilesOrOversizedInput() throws {
        #expect(try normalizedPairingCode("  wispnet1:abc\n def\tghi  ") == "wispnet1:abcdefghi")
        for invalid in ["", "   \n", "{\"host\":\"a\"}", "/tmp/invitation.json", "wispnet1:" + String(repeating: "a", count: 8192)] {
            #expect(throws: (any Error).self) { try normalizedPairingCode(invalid) }
        }
    }
    @Test func pairingSecretsDisappearOnExpiryOrReceiptOfTheirOwnRequest() {
        let qr = PairingQR(host: "host", invitation: "this-invitation", expires_at: 100,
                           matrix: [[false]], code: "wispnet1:fixture")
        func invitation(_ state: String, id: String = "this-invitation") -> Invitation {
            Invitation(invitation: id, peer: nil, name: nil, state: state, mutual: nil)
        }
        #expect(qr.isUsable(at: 99, invitations: []))
        #expect(qr.isUsable(at: 99, invitations: [invitation("issued")]))
        #expect(qr.isUsable(at: 99, invitations: [invitation("challenged")]))
        #expect(qr.isUsable(at: 99, invitations: [invitation("pending", id: "other")]))
        #expect(!qr.isUsable(at: 100, invitations: []))
        for state in ["pending", "approved", "rejected", "expired"] {
            #expect(!qr.isUsable(at: 99, invitations: [invitation(state)]))
        }
    }
}
