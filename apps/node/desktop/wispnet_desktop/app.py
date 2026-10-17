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
from wispnet_desktop.catalog_cache import CatalogCache, valid_catalog
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


def _base_icon_pixmap():
    here = Path(__file__).resolve().parent
    for candidate in [
        here / "icon.png",
        here.parent / "icon.png",
        here.parents[2] / "assets/icon.jpeg",
    ]:
        if candidate.is_file():
            pix = QPixmap(str(candidate))
            if not pix.isNull():
                return pix
    return None


def icon(count=0, online=False):
    pixmap = QPixmap(64, 64)
    pixmap.fill(Qt.GlobalColor.transparent)
    painter = QPainter(pixmap)
    painter.setRenderHint(QPainter.RenderHint.Antialiasing)
    painter.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
    base = _base_icon_pixmap()
    if base is not None:
        scaled = base.scaled(
            60,
            60,
            Qt.AspectRatioMode.KeepAspectRatio,
            Qt.TransformationMode.SmoothTransformation,
        )
        painter.drawPixmap(2, 2, scaled)
        if online:
            badge_w = 28 if count < 10 else 34
            badge_x = 62 - badge_w
            badge_y = 38
            painter.setBrush(QColor("#3b66e6"))
            painter.setPen(QColor("#79b4ff"))
            painter.drawRoundedRect(badge_x, badge_y, badge_w, 24, 11, 11)
            painter.setPen(QColor("#f4f8ff"))
            painter.setFont(QFont("sans-serif", 11, QFont.Weight.Bold))
            painter.drawText(
                badge_x,
                badge_y,
                badge_w,
                24,
                Qt.AlignmentFlag.AlignCenter,
                str(count),
            )
    else:
        painter.setBrush(QColor("#3b66e6" if online else "#142038"))
        painter.setPen(QColor("#79b4ff" if online else "#30456e"))
        painter.drawRoundedRect(4, 4, 56, 56, 18, 18)
        painter.setPen(QColor("#f4f8ff"))
        painter.setFont(QFont("sans-serif", 28, QFont.Weight.Bold))
        painter.drawText(
            pixmap.rect(), Qt.AlignmentFlag.AlignCenter, str(count) if online else "W"
        )
    painter.end()
    return QIcon(pixmap)


class Window(QMainWindow):
    def __init__(self, api, tray=True):
        super().__init__()
        self.api = api
        self.snapshot = None
        self.catalogs = {}
        self.catalog_cache = CatalogCache(getattr(api, "state", None))
        self.cache_loaded = set()
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
        self.resize(1100, 720)
        main = QWidget()
        layout = QVBoxLayout(main)
        layout.setContentsMargins(24, 20, 24, 20)
        layout.setSpacing(16)
        self.setCentralWidget(main)
        header_card = QWidget()
        header_card.setObjectName("headerCard")
        header_layout = QHBoxLayout(header_card)
        header_layout.setContentsMargins(18, 14, 18, 14)
        header_layout.setSpacing(14)
        logo = QLabel()
        logo.setPixmap(icon().pixmap(44, 44))
        logo.setFixedSize(44, 44)
        header_layout.addWidget(logo)
        title_box = QVBoxLayout()
        title_box.setSpacing(4)
        name = QLabel("Your node")
        name.setObjectName("heading")
        title_box.addWidget(name)
        self.status = QLabel("Connecting to local node…")
        self.status.setObjectName("statusText")
        self.status.setTextFormat(Qt.TextFormat.PlainText)
        self.status.setWordWrap(True)
        title_box.addWidget(self.status)
        header_layout.addLayout(title_box, 1)
        self.access = QCheckBox("Browser access")
        self.access.setObjectName("accessToggle")
        self.access.clicked.connect(self.toggle_access)
        header_layout.addWidget(self.access)
        refresh = QPushButton("Refresh")
        refresh.setProperty("variant", "secondary")
        refresh.clicked.connect(self.refresh)
        header_layout.addWidget(refresh)
        layout.addWidget(header_card)
        self.tabs = QTabWidget()
        layout.addWidget(self.tabs, 1)
        self.services = self.table(
            ["Service", "Backend", "Permitted clients"], "My services"
        )
        self.devices = self.table(
            ["Client", "Peer ID", "Approval", "Live connections", "Activity"], "Devices"
        )
        self.peers = self.table(
            ["Peer", "Status", "Shared services", "Last refreshed"], "Peer services"
        )
        self.pending = self.table(
            ["Invitation", "Client", "Peer ID", "State"], "Pairing requests"
        )
        actions = QHBoxLayout()
        actions.setSpacing(10)
        for title, fn, variant in [
            ("Open local service", self.open_local, "primary"),
            ("Share service", self.share, "primary"),
            ("Edit service", self.edit, "secondary"),
            ("Remove service", self.remove, "danger"),
            ("Revoke client", self.revoke, "danger"),
            ("Pair with a host", self.pair, "primary"),
            ("Approve request", lambda: self.decide(True), "primary"),
            ("Reject request", lambda: self.decide(False), "danger"),
        ]:
            button = QPushButton(title)
            button.setProperty("variant", variant)
            button.clicked.connect(fn)
            actions.addWidget(button)
        layout.addLayout(actions)
        self.message = QLabel("")
        self.message.setObjectName("messageBar")
        self.message.setTextFormat(Qt.TextFormat.PlainText)
        self.message.setWordWrap(True)
        layout.addWidget(self.message)
        self.services.cellDoubleClicked.connect(self.open_local)
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
        table.setAlternatingRowColors(True)
        table.setShowGrid(False)
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
        recent = result.get("recent_peers", {})
        active = set(result["connected_peers"]) | set(recent)
        count = len(active)
        window = result.get("presence_timeout_seconds", 60)
        self.status.setText(
            f"{result['host_name']}  ·  Active clients: {count}  ·  {result['transport_streams']} transport streams  ·  Activity window: {window}s  ·  Relay: {result['relay']}"
        )
        if self.tray:
            pending = sum(i["state"] == "pending" for i in result["invitations"])
            self.request_action.setText(f"{pending} pending pairing requests")
            self.tray.setIcon(icon(count, True))
            self.tray.setToolTip(
                f"Wispnet · Active clients: {count} · {len(result['host']['clients'])} approved · {pending} pending · activity window {window}s · relay {result['relay']}"
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
                    "Connected" if c["peer"] in result["connected_peers"] else (
                        f"Seen {recent[c['peer']]}s ago" if c["peer"] in recent else "Idle"
                    ),
                ]
                for c in clients
            ]
            + [
                ["Unapproved/revoked peer", peer, "Not approved", count, "Transport only"]
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
            if peer not in self.cache_loaded:
                self.cache_loaded.add(peer)
                try:
                    cached = self.catalog_cache.load(peer)
                    if cached and peer not in self.catalogs:
                        self.catalogs[peer] = cached
                except (OSError, ValueError):
                    self.message.setText("Invalid catalog cache ignored; refreshing its pinned host")
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
        elif not valid_catalog(peer, value):
            previous["state"] = "Catalog rejected; invalid or wrong host"
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
            try:
                self.catalog_cache.save(peer, self.catalogs[peer])
            except (OSError, ValueError):
                self.message.setText("Catalog refreshed, but its cache could not be saved")
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
                f"{max(0, int(time.time() - data['fetched']))}s ago"
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
        # Explicit target re-pairing permits a new catalog generation baseline.
        try:
            self.catalog_cache.remove(peer)
        except (OSError, ValueError):
            self.message.setText("Cannot reset the catalog cache; retry pairing")
            return
        self.cache_loaded.add(peer)
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

    def open_local(self, *args):
        row = self.services.currentRow()
        if not self.online or not self.snapshot or row < 0:
            return
        if not self.access.isChecked():
            self.message.setText("Enable Browser access first")
            return
        host = self.snapshot["host"]["host"]
        service = self.services.item(row, 0).text()
        self.submit(
            lambda: self.api.request(op="open", host=host, service=service),
            self.open_result,
        )

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
        """
        QWidget {
            background: #070b14;
            color: #edf2ff;
            font-size: 14px;
        }
        QMainWindow, QDialog, QMessageBox {
            background: #070b14;
        }
        QWidget#headerCard {
            background: qlineargradient(x1:0, y1:0, x2:1, y2:1, stop:0 #111d38, stop:0.55 #0d162b, stop:1 #091020);
            border: 1px solid #283c66;
            border-radius: 16px;
        }
        QWidget#headerCard QLabel {
            background: transparent;
        }
        QLabel#heading {
            font-size: 22px;
            font-weight: 700;
            color: #f4f8ff;
        }
        QLabel#statusText {
            font-size: 13px;
            color: #94a6c4;
        }
        QLabel#messageBar {
            background: #101c34;
            color: #b8d4ff;
            border: 1px solid #283e6b;
            border-radius: 12px;
            padding: 8px 14px;
        }
        QPushButton {
            background: #142038;
            color: #edf2ff;
            padding: 9px 15px;
            border: 1px solid #283b60;
            border-radius: 12px;
            font-weight: 600;
        }
        QPushButton:hover {
            background: #1c2e50;
            border-color: #3d5b94;
        }
        QPushButton:pressed {
            background: #101a30;
        }
        QPushButton[variant="primary"] {
            background: #1f3666;
            border: 1px solid #4b73c2;
            color: #f4f8ff;
        }
        QPushButton[variant="primary"]:hover {
            background: #284582;
            border-color: #65ceff;
        }
        QPushButton[variant="danger"] {
            background: #221526;
            border: 1px solid #4e2942;
            color: #ffb3c4;
        }
        QPushButton[variant="danger"]:hover {
            background: #311c36;
            border-color: #783a62;
        }
        QCheckBox#accessToggle {
            background: #142038;
            border: 1px solid #2b4270;
            border-radius: 12px;
            padding: 8px 14px;
            font-weight: 600;
            color: #edf2ff;
            spacing: 10px;
        }
        QCheckBox#accessToggle:hover {
            border-color: #4b73c2;
        }
        QCheckBox::indicator {
            width: 28px;
            height: 16px;
            border: 1px solid #30456e;
            border-radius: 8px;
            background: #0d1526;
        }
        QCheckBox::indicator:checked {
            background: #3b66e6;
            border: 1px solid #79b4ff;
        }
        QCheckBox::indicator:disabled {
            background: #121a2b;
            border-color: #222e47;
        }
        QTabWidget::pane {
            border: 1px solid #243659;
            border-radius: 14px;
            background: #0d1526;
            top: -1px;
        }
        QTabBar::tab {
            background: #10192e;
            color: #94a6c4;
            border: 1px solid #203050;
            border-radius: 10px;
            padding: 10px 18px;
            margin-right: 8px;
            margin-bottom: 8px;
            font-weight: 600;
        }
        QTabBar::tab:hover {
            background: #162340;
            color: #edf2ff;
        }
        QTabBar::tab:selected {
            background: #1f3666;
            color: #f4f8ff;
            border: 1px solid #4b73c2;
        }
        QTableWidget {
            background: #0d1526;
            alternate-background-color: #101a30;
            gridline-color: #1c2a46;
            border: 0;
            border-radius: 14px;
            selection-background-color: #1f3666;
            selection-color: #f4f8ff;
        }
        QTableWidget::item {
            padding: 8px 10px;
            border-bottom: 1px solid #16223a;
        }
        QHeaderView::section {
            background: #131f38;
            color: #94a6c4;
            padding: 11px 12px;
            border: 0;
            border-bottom: 1px solid #243659;
            font-weight: 600;
        }
        QLineEdit, QListWidget {
            background: #0d1526;
            color: #edf2ff;
            padding: 10px 12px;
            border: 1px solid #25385c;
            border-radius: 10px;
        }
        QLineEdit:focus, QListWidget:focus {
            border: 1px solid #65ceff;
        }
        QMenu {
            background: #0e172a;
            color: #edf2ff;
            border: 1px solid #283c66;
            border-radius: 10px;
            padding: 6px;
        }
        QMenu::item {
            padding: 8px 16px;
            border-radius: 6px;
        }
        QMenu::item:selected {
            background: #1f3666;
        }
        """
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
