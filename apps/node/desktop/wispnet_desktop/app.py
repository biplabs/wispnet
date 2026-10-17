import argparse
import json
import os
import tempfile
import stat
import re
import sys
import time
from pathlib import Path
from urllib.parse import urlsplit
from PySide6.QtCore import (
    QObject,
    QRunnable,
    QThreadPool,
    QTimer,
    Signal,
    QUrl,
    Qt,
    QLockFile,
    QStandardPaths,
)
from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtGui import (
    QAction,
    QColor,
    QDesktopServices,
    QIcon,
    QPainter,
    QPixmap,
    QFont,
)
from PySide6.QtWidgets import (
    QApplication,
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QTabWidget,
    QTableWidget,
    QTableWidgetItem,
    QSystemTrayIcon,
    QMenu,
    QMessageBox,
    QCheckBox,
    QDialog,
    QDialogButtonBox,
    QLineEdit,
    QFormLayout,
    QListWidget,
    QListWidgetItem,
    QFileDialog,
)
from .api import NodeAPI


class Signals(QObject):
    done = Signal(object, object)


class Job(QRunnable):
    def __init__(self, fn, callback):
        super().__init__()
        self.fn = fn
        self.signals = Signals()
        self.signals.done.connect(callback)

    def run(self):
        try:
            self.signals.done.emit(self.fn(), None)
        except Exception as error:
            self.signals.done.emit(None, str(error))


def item(text):
    value = QTableWidgetItem(str(text))
    value.setFlags(value.flags() & ~Qt.ItemFlag.ItemIsEditable)
    return value


def icon(count=0, online=False):
    pixmap = QPixmap(64, 64)
    pixmap.fill(Qt.GlobalColor.transparent)
    painter = QPainter(pixmap)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    painter.setBrush(QColor("#617bf5" if online else "#677085"))
    painter.setPen(Qt.PenStyle.NoPen)
    painter.drawRoundedRect(4, 4, 56, 56, 18, 18)
    painter.setPen(QColor("white"))
    painter.setFont(QFont("sans-serif", 29, QFont.Weight.Bold))
    painter.drawText(
        pixmap.rect(), Qt.AlignmentFlag.AlignCenter, str(count) if online else "w"
    )
    painter.end()
    return QIcon(pixmap)


class Window(QMainWindow):
    def __init__(self, api, tray=True):
        super().__init__()
        self.api = api
        self.snapshot = None
        self.catalogs = {}
        self.peer_busy = set()
        self.busy = False
        self.online = False
        self.launch_files = set()
        self.peer_names = {}
        if hasattr(api, "state"):
            try:
                names_file = api.state / "desktop-peer-names.json"
                metadata = names_file.lstat()
                if (
                    stat.S_ISREG(metadata.st_mode)
                    and metadata.st_uid == os.getuid()
                    and metadata.st_mode & 0o077 == 0
                    and metadata.st_size <= 32768
                ):
                    names = json.loads(names_file.read_text())
                    self.peer_names = {
                        peer: name
                        for peer, name in names.items()
                        if re.fullmatch("[0-9a-f]{64}", peer)
                        and isinstance(name, str)
                        and 0 < len(name) <= 128
                        and not any(ord(c) < 32 for c in name)
                    }
            except (OSError, ValueError, TypeError, AttributeError):
                pass
        self.pool = QThreadPool(self)
        self.pool.setMaxThreadCount(4)
        self.peer_pool = QThreadPool(self)
        self.peer_pool.setMaxThreadCount(4)
        self.jobs = set()
        self.setWindowTitle("Wispnet")
        self.setWindowIcon(icon())
        self.resize(1080, 700)
        main = QWidget()
        layout = QVBoxLayout(main)
        layout.setContentsMargins(28, 24, 28, 24)
        layout.setSpacing(18)
        self.setCentralWidget(main)
        top = QHBoxLayout()
        name = QLabel("Your node")
        name.setObjectName("heading")
        top.addWidget(name)
        top.addStretch()
        self.access = QCheckBox("Browser access")
        self.access.clicked.connect(self.toggle_access)
        top.addWidget(self.access)
        refresh = QPushButton("Refresh")
        refresh.clicked.connect(self.refresh)
        top.addWidget(refresh)
        layout.addLayout(top)
        self.status = QLabel("Connecting to local node…")
        self.status.setTextFormat(Qt.TextFormat.PlainText)
        self.status.setWordWrap(True)
        layout.addWidget(self.status)
        self.tabs = QTabWidget()
        layout.addWidget(self.tabs)
        self.services = self.table(
            ["Service", "Backend", "Permitted clients"], "My services"
        )
        self.devices = self.table(
            ["Client", "Peer ID", "Approval", "Live connections"], "Devices"
        )
        self.peers = self.table(
            ["Peer", "Status", "Shared services", "Last refreshed"], "Peer services"
        )
        self.pending = self.table(
            ["Invitation", "Client", "Peer ID", "State"], "Pairing requests"
        )
        actions = QHBoxLayout()
        for title, fn in [
            ("Share service", self.share),
            ("Edit service", self.edit),
            ("Remove service", self.remove),
            ("Revoke client", self.revoke),
            ("Pair with a host", self.pair),
            ("Approve request", lambda: self.decide(True)),
            ("Reject request", lambda: self.decide(False)),
        ]:
            button = QPushButton(title)
            button.clicked.connect(fn)
            actions.addWidget(button)
        layout.addLayout(actions)
        self.message = QLabel("")
        self.message.setTextFormat(Qt.TextFormat.PlainText)
        self.message.setWordWrap(True)
        layout.addWidget(self.message)
        self.peers.cellDoubleClicked.connect(self.open_peer)
        self.tray = QSystemTrayIcon(icon(), self) if tray else None
        if self.tray:
            menu = QMenu()
            menu.addAction("Open Wispnet", self.reveal)
            menu.addAction("Refresh", self.refresh)
            self.request_action = menu.addAction(
                "Pairing requests",
                lambda: (self.tabs.setCurrentIndex(3), self.reveal()),
            )
            menu.addSeparator()
            menu.addAction(
                "Quit desktop (node keeps running)", QApplication.instance().quit
            )
            self.tray.setContextMenu(menu)
            self.tray.activated.connect(
                lambda reason: (
                    self.reveal()
                    if reason == QSystemTrayIcon.ActivationReason.Trigger
                    else None
                )
            )
            self.tray.show()
        self.timer = QTimer(self)
        self.timer.timeout.connect(self.refresh)
        self.timer.start(2000)
        self.refresh()

    def table(self, columns, title):
        table = QTableWidget(0, len(columns))
        table.setHorizontalHeaderLabels(columns)
        table.horizontalHeader().setStretchLastSection(True)
        table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        table.setSelectionMode(QTableWidget.SelectionMode.SingleSelection)
        table.verticalHeader().hide()
        self.tabs.addTab(table, title)
        return table

    def submit(self, fn, callback, peer=False):
        job = Job(fn, lambda result, error: self.finished(job, callback, result, error))
        self.jobs.add(job)
        (self.peer_pool if peer else self.pool).start(job)

    def finished(self, job, callback, result, error):
        self.jobs.discard(job)
        callback(result, error)

    def reveal(self):
        self.show()
        self.raise_()
        self.activateWindow()

    def closeEvent(self, event):
        if self.tray and QSystemTrayIcon.isSystemTrayAvailable():
            self.hide()
            event.ignore()
        else:
            event.accept()

    def refresh(self):
        if self.busy:
            return
        self.busy = True
        self.submit(lambda: self.api.request(op="status"), self.updated)

    def updated(self, result, error):
        self.busy = False
        self.online = error is None
        if error:
            self.snapshot = None
            self.access.setEnabled(False)
            self.status.setText("Node unavailable — " + error)
            self.devices.setRowCount(0)
            self.pending.setRowCount(0)
            self.services.setRowCount(0)
            for value in self.catalogs.values():
                value["state"] = "Node unavailable"
            self.draw_peers()
            if self.tray:
                self.tray.setIcon(icon())
                self.tray.setToolTip("Wispnet · node unavailable")
            return
        self.snapshot = result
        self.access.setEnabled(True)
        self.access.setChecked(result["access_enabled"])
        count = len(result["connected_peers"])
        self.status.setText(
            f"{result['host_name']}  ·  {count} clients with active connections  ·  {result['transport_streams']} transport streams  ·  Relay: {result['relay']}"
        )
        if self.tray:
            pending = sum(i["state"] == "pending" for i in result["invitations"])
            self.request_action.setText(f"{pending} pending pairing requests")
            self.tray.setIcon(icon(count, True))
            self.tray.setToolTip(
                f"Wispnet · {count} clients with active connections · {len(result['host']['clients'])} approved · {pending} pending · relay {result['relay']}"
            )
        self.fill(
            self.services,
            [
                [
                    s["id"],
                    s["backend"],
                    ", ".join(self.client_name(p) for p in s["allow"]) or "Nobody",
                ]
                for s in result["host"]["services"]
            ],
        )
        clients = result["host"]["clients"]
        known = {c["peer"] for c in clients}
        self.fill(
            self.devices,
            [
                [
                    c["name"],
                    c["peer"],
                    "Approved",
                    result["connected_peers"].get(c["peer"], 0),
                ]
                for c in clients
            ]
            + [
                ["Unapproved/revoked peer", peer, "Not approved", count]
                for peer, count in result["connected_peers"].items()
                if peer not in known
            ],
        )
        self.fill(
            self.pending,
            [
                [i["invitation"], i["name"] or "—", i["peer"] or "—", i["state"]]
                for i in result["invitations"]
            ],
        )
        for peer in list(self.catalogs):
            if peer not in result["peer_pins"]:
                self.catalogs.pop(peer)
        for peer in result["peer_pins"]:
            if peer not in self.peer_busy:
                self.peer_busy.add(peer)
                self.submit(
                    lambda p=peer: self.api.request(op="catalog", host=p),
                    lambda value, error, p=peer: self.peer_updated(p, value, error),
                    peer=True,
                )
        self.draw_peers()

    def client_name(self, peer):
        return next(
            (
                c["name"] + " · " + peer[:8]
                for c in self.snapshot["host"]["clients"]
                if c["peer"] == peer
            ),
            peer[:12],
        )

    def fill(self, table, rows):
        selected = table.currentRow()
        key_column = 1 if table is self.devices else 0
        key = (
            table.item(selected, key_column).text()
            if selected >= 0 and table.item(selected, key_column)
            else None
        )
        table.setRowCount(len(rows))
        for row, values in enumerate(rows):
            for col, value in enumerate(values):
                table.setItem(row, col, item(value))
            if str(values[key_column]) == key:
                table.selectRow(row)
        table.resizeColumnsToContents()

    def peer_updated(self, peer, value, error):
        self.peer_busy.discard(peer)
        previous = self.catalogs.get(peer, {})
        if error:
            previous["state"] = "Denied" if "denied" in error.lower() else "Unavailable"
            self.catalogs[peer] = previous
        elif value.get("host") != peer:
            previous["state"] = "Catalog rejected; wrong host"
            self.catalogs[peer] = previous
        elif previous.get("catalog") and (
            previous["catalog"]["generation"] != value["generation"]
            or value["revision"] < previous["catalog"]["revision"]
            or (
                value["revision"] == previous["catalog"]["revision"]
                and value["services"] != previous["catalog"]["services"]
            )
        ):
            previous["state"] = "Catalog rejected; re-pair required"
            self.catalogs[peer] = previous
        else:
            self.catalogs[peer] = {
                "catalog": value,
                "fetched": time.time(),
                "state": "Available",
            }
        self.draw_peers()

    def draw_peers(self):
        rows = []
        self.peer_routes = []
        pins = self.snapshot["peer_pins"] if self.snapshot else list(self.catalogs)
        for peer in pins:
            data = self.catalogs.get(peer, {})
            services = data.get("catalog", {}).get("services", [])
            state = data.get("state", "Refreshing")
            age = (
                f"{int(time.time() - data['fetched'])}s ago"
                if "fetched" in data
                else "Never"
            )
            for service in services or [None]:
                rows.append(
                    [
                        self.peer_names.get(peer, "Host " + peer[:8]) + "\n" + peer,
                        state,
                        service["id"] if service else "No services",
                        age,
                    ]
                )
                self.peer_routes.append(
                    (peer, service["id"] if service else None, state)
                )
        self.fill(
            self.peers,
            rows
            or [["No paired hosts — use Pair with a host", "Not paired", "—", "—"]],
        )

    def perform(self, request, success="Saved"):
        self.submit(
            lambda: self.api.request(**request),
            lambda result, error: self.result(result, error, success),
        )

    def result(self, value, error, success):
        self.message.setText(error or success)
        self.refresh()

    def toggle_access(self, enabled):
        self.perform(
            {"op": "enable" if enabled else "stop"},
            "Browser access enabled" if enabled else "Browser access stopped",
        )

    def mutation(self, operation):
        if not self.snapshot:
            return
        operation.setdefault("expected_revision", self.snapshot["host"]["revision"])
        self.perform({"op": "mutate", "operation": operation})

    def edit(self):
        row = self.services.currentRow()
        if row >= 0:
            self.share(self.services.item(row, 0).text())

    def share(self, existing=None):
        if not self.snapshot:
            return
        if not isinstance(existing, str):
            existing = None
        revision = self.snapshot["host"]["revision"]
        dialog = QDialog(self)
        dialog.setWindowTitle("Share a local service")
        form = QFormLayout(dialog)
        name = QLineEdit()
        backend = QLineEdit("127.0.0.1:")
        clients = QListWidget()
        for client in self.snapshot["host"]["clients"]:
            row = QListWidgetItem(client["name"] + " · " + client["peer"][:12])
            row.setData(Qt.ItemDataRole.UserRole, client["peer"])
            row.setFlags(row.flags() | Qt.ItemFlag.ItemIsUserCheckable)
            row.setCheckState(
                Qt.CheckState.Checked
                if existing
                and client["peer"]
                in next(
                    s["allow"]
                    for s in self.snapshot["host"]["services"]
                    if s["id"] == existing
                )
                else Qt.CheckState.Unchecked
            )
            clients.addItem(row)
        if existing:
            service = next(
                s for s in self.snapshot["host"]["services"] if s["id"] == existing
            )
            name.setText(existing)
            name.setReadOnly(True)
            backend.setText(service["backend"])
        form.addRow("Service ID", name)
        form.addRow("Loopback backend", backend)
        form.addRow("Grant access to", clients)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Save
            | QDialogButtonBox.StandardButton.Cancel
        )
        form.addRow(buttons)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        if dialog.exec():
            if (
                any(
                    s["id"] == name.text().strip()
                    for s in self.snapshot["host"]["services"]
                )
                and QMessageBox.question(
                    self,
                    "Replace service",
                    "Replace this service backend and its entire access list?",
                )
                != QMessageBox.StandardButton.Yes
            ):
                return
            self.mutation(
                {
                    "op": "service",
                    "expected_revision": revision,
                    "id": name.text().strip(),
                    "backend": backend.text().strip(),
                    "allow": [
                        clients.item(i).data(Qt.ItemDataRole.UserRole)
                        for i in range(clients.count())
                        if clients.item(i).checkState() == Qt.CheckState.Checked
                    ],
                }
            )

    def remove(self):
        row = self.services.currentRow()
        if (
            row >= 0
            and QMessageBox.question(
                self,
                "Remove service",
                "Remove this local service and interrupt its active access?",
            )
            == QMessageBox.StandardButton.Yes
        ):
            self.mutation({"op": "remove", "id": self.services.item(row, 0).text()})

    def revoke(self):
        row = self.devices.currentRow()
        if (
            row >= 0
            and QMessageBox.question(
                self,
                "Revoke client",
                "Revoke this client and all its service grants on this node?\n"
                + self.devices.item(row, 1).text(),
            )
            == QMessageBox.StandardButton.Yes
        ):
            self.mutation({"op": "revoke", "peer": self.devices.item(row, 1).text()})

    def decide(self, approve):
        row = self.pending.currentRow()
        if row < 0:
            return
        if approve and self.pending.item(row, 3).text() != "pending":
            self.message.setText("Only proved pending requests can be approved")
            return
        if (
            QMessageBox.question(
                self,
                "Pairing request",
                ("Approve" if approve else "Reject")
                + " this specific request on this node?\n"
                + self.pending.item(row, 2).text(),
            )
            == QMessageBox.StandardButton.Yes
        ):
            self.mutation(
                {
                    "op": "approve_invitation" if approve else "reject_invitation",
                    "invitation": self.pending.item(row, 0).text(),
                }
            )

    def pair(self):
        dialog = QDialog(self)
        dialog.setWindowTitle("Pair with a host")
        form = QFormLayout(dialog)
        code = QLineEdit()
        code.setEchoMode(QLineEdit.EchoMode.Password)
        form.addRow("Invitation code or JSON", code)
        load = QPushButton("Import invitation file")
        form.addRow(load)

        def read():
            path, _ = QFileDialog.getOpenFileName(dialog, "Invitation file")
            if path:
                try:
                    with open(path, "rb") as source:
                        data = source.read(8193)
                    if len(data) > 8192:
                        raise ValueError("Invitation too large")
                    code.setText(data.decode())
                except Exception as error:
                    self.message.setText(str(error))

        load.clicked.connect(read)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel
        )
        form.addRow(buttons)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        if dialog.exec():
            document = code.text()
            code.clear()
            self.submit(
                lambda: self.api.request(op="preview_invitation", invitation=document),
                lambda value, error: self.confirm_pair(document, value, error),
            )

    def confirm_pair(self, document, value, error):
        if error:
            self.message.setText(error)
            return
        if (
            QMessageBox.question(
                self,
                "Compare host ID",
                "Compare this complete ID with the target computer before requesting access:\n"
                + value["host"],
            )
            == QMessageBox.StandardButton.Yes
        ):
            self.submit(
                lambda: self.api.request(
                    op="pair", host=value["host"], invitation=document
                ),
                lambda result, error: self.pair_saved(value, result, error),
            )

    def pair_saved(self, preview, result, error):
        if error:
            self.message.setText(error)
            return
        peer = preview["host"]
        self.catalogs.pop(peer, None)
        self.peer_names[peer] = preview.get("host_name") or "Host " + peer[:8]
        path = None
        try:
            fd, path = tempfile.mkstemp(prefix="peer-names-", dir=self.api.state)
            with os.fdopen(fd, "w") as output:
                json.dump(self.peer_names, output)
            os.replace(path, self.api.state / "desktop-peer-names.json")
            self.message.setText(
                "Request sent. Approve this node on the target host and grant services."
            )
        except OSError:
            self.message.setText(
                "Request sent; display name could not be saved. Approve it on the target host."
            )
        finally:
            if path:
                Path(path).unlink(missing_ok=True)
        self.refresh()

    def open_peer(self, row, column):
        if row >= len(self.peer_routes):
            return
        peer, service, state = self.peer_routes[row]
        if not service or state != "Available" or not self.online:
            self.message.setText("Refresh this peer before opening its service")
            return
        if not self.access.isChecked():
            self.message.setText("Enable Browser access first")
            return
        self.submit(
            lambda: self.api.request(op="open", host=peer, service=service),
            self.open_result,
        )

    def open_result(self, value, error):
        if error:
            self.message.setText(error)
            return
        url = QUrl(value["url"])
        parsed = urlsplit(value["url"])
        if (
            parsed.scheme != "http"
            or not parsed.hostname
            or not parsed.hostname.endswith(".localhost")
            or parsed.path != "/_wipsnet/start"
        ):
            self.message.setText("Invalid local browser handoff")
            return
        # Keep the bootstrap capability out of browser process arguments.
        fd, path = tempfile.mkstemp(
            prefix="browser-handoff-", suffix=".html", dir=self.api.state
        )
        with os.fdopen(fd, "w") as output:
            output.write(
                '<!doctype html><meta name="referrer" content="no-referrer"><script>location.replace('
                + json.dumps(value["url"])
                + ")</script>"
            )
        self.launch_files.add(path)

        def cleanup():
            Path(path).unlink(missing_ok=True)
            self.launch_files.discard(path)

        QTimer.singleShot(60000, cleanup)
        if not QDesktopServices.openUrl(QUrl.fromLocalFile(path)):
            cleanup()
            self.message.setText("Could not open your browser")


def appearance(app):
    app.setStyle("Fusion")
    app.setStyleSheet(
        "QWidget {background:#0b1020;color:#e5eaf5;font-size:14px;} QMainWindow {background:#0b1020;} QLabel#heading {font-size:28px;font-weight:600;} QPushButton {background:#202c48;padding:9px;border-radius:7px;} QPushButton:hover {background:#30446a;} QTableWidget {background:#10182b;gridline-color:#26324a;border:0;} QHeaderView::section {background:#18243b;padding:10px;border:0;} QCheckBox::indicator {width:16px;height:16px;border:1px solid #7384aa;border-radius:3px;background:#17223a;} QCheckBox::indicator:checked {background:#617bf5;} QLineEdit {background:#19243a;padding:8px;border:1px solid #354462;} QTabBar::tab {padding:12px;} QTabBar::tab:selected {background:#26395e;}"
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--config", default=str(Path.home() / ".config/wipsnet-node/host.toml")
    )
    parser.add_argument("--tray", action="store_true")
    args = parser.parse_args()
    app = QApplication(sys.argv[:1])
    app.setApplicationName("Wispnet")
    app.setQuitOnLastWindowClosed(False)
    lock = QLockFile(
        str(
            Path(
                QStandardPaths.writableLocation(
                    QStandardPaths.StandardLocation.RuntimeLocation
                )
            )
            / "wispnet-desktop.lock"
        )
    )
    if not lock.tryLock(0):
        connection = QLocalSocket()
        for attempt in range(20):
            connection.abort()
            connection.connectToServer(
                str(
                    Path(
                        QStandardPaths.writableLocation(
                            QStandardPaths.StandardLocation.RuntimeLocation
                        )
                    )
                    / "wispnet-desktop.ui"
                )
            )
            if connection.waitForConnected(100):
                connection.write(b"show")
                connection.waitForBytesWritten(1000)
                connection.disconnectFromServer()
                return
            time.sleep(0.1)
        raise SystemExit("Wispnet desktop is already running; use its tray icon")
    appearance(app)
    window = Window(NodeAPI(args.config))
    app.aboutToQuit.connect(
        lambda: [Path(path).unlink(missing_ok=True) for path in window.launch_files]
    )
    server = QLocalServer()
    server.setSocketOptions(QLocalServer.SocketOption.UserAccessOption)
    server_name = str(
        Path(
            QStandardPaths.writableLocation(
                QStandardPaths.StandardLocation.RuntimeLocation
            )
        )
        / "wispnet-desktop.ui"
    )
    QLocalServer.removeServer(server_name)
    server.listen(server_name)

    def reveal_existing():
        connection = server.nextPendingConnection()
        connection.close()
        connection.deleteLater()
        window.reveal()

    server.newConnection.connect(reveal_existing)
    if not args.tray or not QSystemTrayIcon.isSystemTrayAvailable():
        window.show()
    app.exec()


if __name__ == "__main__":
    main()
