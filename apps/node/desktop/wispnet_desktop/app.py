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
from wispnet_desktop.combined import Combined, validate_plan
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
    QPen,
    QPixmap,
    QFont,
    QRadialGradient,
    QLinearGradient,
)
from PySide6.QtWidgets import (
    QApplication,
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QGridLayout,
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
    QComboBox,
    QFormLayout,
    QListWidget,
    QListWidgetItem,
    QFileDialog,
    QFrame,
)
import math
import random
from .api import NodeAPI


class StarfieldBanner(QWidget):
    """Star Wars-inspired navicomputer header with drifting 3D starfield and 3-beat hyperjump."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("headerCard")
        self.setMinimumHeight(112)
        rng = random.Random(17)
        self.stars = [
            (rng.random() * math.tau, rng.random(), 0.6 + rng.random() * 0.8, i)
            for i in range(96)
        ]
        self.hyperspace = False
        self.warp_start = 0.0
        self.warp_duration = 0.0
        self.decelerating = False
        self.anim_timer = QTimer(self)
        self.anim_timer.timeout.connect(self._tick)
        self.anim_timer.start(33)

    def jump(self):
        self.decelerating = False
        self.warp_start = time.monotonic()
        self.warp_duration = 0.9
        self.update()

    def drop_out(self):
        self.decelerating = True
        self.warp_start = time.monotonic()
        self.warp_duration = 0.32
        self.update()

    def _tick(self):
        if self.isVisible():
            self.update()

    def paintEvent(self, event):
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = float(self.width()), float(self.height())
        cx, cy = w * 0.5, h * 0.5
        scale = math.hypot(cx, cy)
        now = time.monotonic()
        elapsed = now - self.warp_start if self.warp_duration > 0 else 999.0
        warp = elapsed < self.warp_duration
        progress = min(1.0, max(0.0, elapsed / self.warp_duration)) if warp else 0.0
        if not warp and self.warp_duration > 0:
            self.warp_duration = 0.0
            self.decelerating = False

        bg = QLinearGradient(0, 0, w, h)
        if self.hyperspace:
            bg.setColorAt(0.0, QColor("#12244a"))
            bg.setColorAt(0.55, QColor("#0c1834"))
            bg.setColorAt(1.0, QColor("#081022"))
        else:
            bg.setColorAt(0.0, QColor("#0e1930"))
            bg.setColorAt(0.55, QColor("#0a1224"))
            bg.setColorAt(1.0, QColor("#070c18"))
        painter.setBrush(bg)
        painter.setPen(
            QPen(QColor("#3868b8" if self.hyperspace else "#263c66"), 1.2)
        )
        painter.drawRoundedRect(1, 1, w - 2, h - 2, 18, 18)

        pulse = 0.82 + 0.18 * math.sin(now * 1.6)
        nebula = QRadialGradient(w * 0.72, cy, max(1.0, w * 0.45))
        nebula.setColorAt(
            0.0,
            QColor(56, 118, 224, int((110 if self.hyperspace else 68) * pulse)),
        )
        nebula.setColorAt(0.55, QColor(24, 52, 112, int(45 * pulse)))
        nebula.setColorAt(1.0, QColor(7, 11, 20, 0))
        painter.setPen(Qt.PenStyle.NoPen)
        painter.setBrush(nebula)
        painter.drawRoundedRect(2, 2, w - 4, h - 4, 17, 17)

        lock_on = (
            math.sin((progress / 0.18) * math.pi)
            if warp and not self.decelerating and progress < 0.18
            else 0.0
        )
        warp_phase = (
            max(0.0, (progress - 0.14) / 0.86)
            if warp and not self.decelerating
            else (1.0 - progress) * 0.45
            if warp
            else 0.0
        )
        surge = math.sin(math.pi * (warp_phase ** 0.75)) if warp else 0.0

        if warp and surge > 0:
            flash = QRadialGradient(cx, cy, max(1.0, w * 0.45))
            flash.setColorAt(0.0, QColor(228, 251, 255, int(180 * surge)))
            flash.setColorAt(0.2, QColor(101, 206, 255, int(120 * surge)))
            flash.setColorAt(1.0, QColor(16, 25, 56, 0))
            painter.setBrush(flash)
            painter.drawRoundedRect(2, 2, w - 4, h - 4, 17, 17)

        if warp and not self.decelerating and progress > 0.76:
            wave = (progress - 0.76) / 0.24
            painter.setBrush(Qt.BrushStyle.NoBrush)
            painter.setPen(
                QPen(
                    QColor(101, 206, 255, int((1.0 - wave) * 165)),
                    max(1.0, 2.8 - wave * 2.0),
                )
            )
            painter.drawEllipse(
                int(cx - wave * scale * 0.65),
                int(cy - wave * scale * 0.65),
                int(wave * scale * 1.3),
                int(wave * scale * 1.3),
            )

        cruise = ((now * (1.8 if self.hyperspace else 1.0)) / 24.0) % 1.0
        for angle, base_depth, speed, idx in self.stars:
            depth = (base_depth + cruise * speed + (warp_phase * 0.75 if warp else 0.0)) % 1.0
            radius = max(4.0, ((0.03 + depth * depth) - lock_on * 0.02) * scale)
            ux, uy = math.cos(angle), math.sin(angle)
            twinkle = 0.7 + 0.3 * math.sin(now * 4.0 * (0.5 + speed) + idx)
            tail = (
                surge * surge * (0.025 + depth * 0.36) * scale
                if warp
                else depth * depth * (14.0 if self.hyperspace else 5.5)
            )
            c_hex = (
                "#ffdfa8"
                if idx % 15 == 0
                else "#7da2ff"
                if idx % 9 == 0
                else "#65ceff"
                if idx % 6 == 0
                else "#f0f8ff"
            )
            alpha = int(
                min(
                    255,
                    (125 + speed * 115 if warp else (75 + depth * 155) * twinkle)
                    * (min(1.0, 0.35 + surge * 3.0) if warp else 1.0),
                )
            )
            color = QColor(c_hex)
            color.setAlpha(alpha)
            width = 1.2 + speed * 1.5 if warp else 1.0 + depth * 1.8
            pen = QPen(color, width, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap)
            painter.setPen(pen)
            x0, y0 = cx + ux * radius, cy + uy * radius
            if tail < 1.5:
                painter.drawPoint(int(x0), int(y0))
            else:
                painter.drawLine(
                    int(x0),
                    int(y0),
                    int(cx + ux * (radius + tail)),
                    int(cy + uy * (radius + tail)),
                )
        painter.end()


class HyperspaceToggle(QCheckBox):
    """Accessible QCheckBox styled as a Star Wars Hyperdrive Core / Hyperspace Beacon pill."""

    def __init__(self, text="Browser access", parent=None):
        super().__init__(text, parent)
        self.setObjectName("accessToggle")
        self.setCursor(Qt.CursorShape.PointingHandCursor)
        self.setFixedSize(192, 42)

    def hitButton(self, pos):
        return self.rect().contains(pos)

    def paintEvent(self, event):
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = self.width(), self.height()
        on = self.isChecked()
        enabled = self.isEnabled()

        painter.setPen(
            QPen(
                QColor("#5b9cf7" if on else "#2e4674" if enabled else "#1e2c46"),
                1.5,
            )
        )
        painter.setBrush(
            QColor("#142b56" if on else "#0f1a30" if enabled else "#0b1220")
        )
        painter.drawRoundedRect(2, 2, w - 4, h - 4, (h - 4) / 2, (h - 4) / 2)

        bx, by = 22, h // 2
        if on:
            painter.setPen(Qt.PenStyle.NoPen)
            painter.setBrush(QColor(101, 206, 255, 72))
            painter.drawEllipse(bx - 9, by - 9, 18, 18)
            painter.setBrush(QColor("#65ceff"))
            painter.drawEllipse(bx - 5, by - 5, 10, 10)
            painter.setBrush(QColor("#f4fcff"))
            painter.drawEllipse(bx - 2, by - 2, 4, 4)
        else:
            painter.setBrush(Qt.BrushStyle.NoBrush)
            painter.setPen(
                QPen(QColor("#65ceff" if enabled else "#4a5d7a"), 1.5)
            )
            painter.drawEllipse(bx - 6, by - 6, 12, 12)
            painter.setPen(Qt.PenStyle.NoPen)
            painter.setBrush(QColor("#65ceff" if enabled else "#4a5d7a"))
            painter.drawEllipse(bx - 2, by - 2, 4, 4)

        painter.setPen(
            QColor("#f4f8ff" if enabled else "#5c6f8c")
        )
        font = QFont("sans-serif", 9, QFont.Weight.Bold)
        font.setLetterSpacing(QFont.SpacingType.AbsoluteSpacing, 0.9)
        painter.setFont(font)
        label = "HYPERSPACE ON" if on else "JUMP TO SPACE"
        painter.drawText(
            38, 0, w - 46, h, Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft, label
        )
        painter.end()


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
        here.parents[2] / "assets/icon2.jpeg",
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
        self.combined = Combined(api.config) if hasattr(api, "config") else None
        self.operation_routes = []
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
        self.resize(1140, 760)
        main = QWidget()
        layout = QVBoxLayout(main)
        layout.setContentsMargins(24, 20, 24, 20)
        layout.setSpacing(14)
        self.setCentralWidget(main)
        self.header_banner = StarfieldBanner()
        header_layout = QHBoxLayout(self.header_banner)
        header_layout.setContentsMargins(22, 16, 22, 16)
        header_layout.setSpacing(16)
        logo = QLabel()
        logo.setPixmap(icon().pixmap(48, 48))
        logo.setFixedSize(48, 48)
        header_layout.addWidget(logo)
        title_box = QVBoxLayout()
        title_box.setSpacing(5)
        self.heading_label = QLabel("◈  YOUR NODE · NAVICOMPUTER")
        self.heading_label.setObjectName("heading")
        title_box.addWidget(self.heading_label)
        self.status = QLabel("Connecting to local node…")
        self.status.setObjectName("statusText")
        self.status.setTextFormat(Qt.TextFormat.PlainText)
        self.status.setWordWrap(True)
        title_box.addWidget(self.status)
        badges_row = QHBoxLayout()
        badges_row.setSpacing(8)
        self.relay_badge = QLabel("◌ RELAY STANDBY")
        self.relay_badge.setObjectName("telemetryBadge")
        self.clients_badge = QLabel("◈ 0 ACTIVE CLIENTS")
        self.clients_badge.setObjectName("telemetryBadge")
        self.streams_badge = QLabel("⇄ 0 STREAMS")
        self.streams_badge.setObjectName("telemetryBadge")
        for badge in (self.relay_badge, self.clients_badge, self.streams_badge):
            badges_row.addWidget(badge)
        badges_row.addStretch(1)
        title_box.addLayout(badges_row)
        header_layout.addLayout(title_box, 1)
        self.access = HyperspaceToggle("Browser access")
        self.access.clicked.connect(self.toggle_access)
        header_layout.addWidget(self.access)
        refresh = QPushButton("↻  Scan")
        refresh.setProperty("variant", "secondary")
        refresh.clicked.connect(self.refresh)
        header_layout.addWidget(refresh)
        layout.addWidget(self.header_banner)
        self.tabs = QTabWidget()
        layout.addWidget(self.tabs, 1)
        self.local_cards_box = QWidget()
        self.local_cards_layout = QHBoxLayout(self.local_cards_box)
        self.local_cards_layout.setContentsMargins(0, 0, 0, 4)
        self.local_cards_layout.setSpacing(10)
        self.local_cards_box.hide()
        self.services = self.table(
            ["Service", "Backend", "Permitted clients"],
            "◈  Local Services",
            "Loopback services shared from this node to approved peers",
            [
                ("✦ Open local service", self.open_local, "primary"),
                ("+ Share service", self.share, "primary"),
                ("Edit service", self.edit, "secondary"),
                ("Remove service", self.remove, "danger"),
            ],
            cards=self.local_cards_box,
        )
        self.devices = self.table(
            ["Client", "Peer ID", "Approval", "Live connections", "Activity"],
            "◈  Fleet & Devices",
            "Approved client identities and active transport streams on this node",
            [
                (
                    "+ Add client across hosts",
                    lambda: self.combined_dialog("approve"),
                    "primary",
                ),
                ("Revoke client", self.revoke, "danger"),
            ],
        )
        self.peer_cards_box = QWidget()
        self.peer_cards_layout = QHBoxLayout(self.peer_cards_box)
        self.peer_cards_layout.setContentsMargins(0, 0, 0, 4)
        self.peer_cards_layout.setSpacing(10)
        self.peer_cards_box.hide()
        self.peers = self.table(
            ["Peer", "Status", "Shared services", "Last refreshed"],
            "◈  Wisp Space · Peers",
            "Remote systems and holographic service endpoints across your Wisp network",
            [
                (
                    "✦ Open peer service",
                    lambda: self.open_peer(max(0, self.peers.currentRow()), 0),
                    "primary",
                ),
                ("+ Pair with a host", self.pair, "primary"),
            ],
            cards=self.peer_cards_box,
        )
        self.pending = self.table(
            ["Invitation", "Client", "Peer ID", "State"],
            "◈  Pairing Beacons",
            "Incoming pairing requests awaiting owner verification on this node",
            [
                ("✓ Approve request", lambda: self.decide(True), "primary"),
                ("✕ Reject request", lambda: self.decide(False), "danger"),
            ],
        )
        self.operations_summary = QLabel("Each host acknowledges independently")
        self.operations_summary.setObjectName("sectorSubtitle")
        self.operations_summary.setTextFormat(Qt.TextFormat.PlainText)
        self.operations = self.table(
            [
                "Operation",
                "Action",
                "Target host",
                "State",
                "Applied revision",
                "Details",
            ],
            "◈  Combined Operations",
            None,
            [
                (
                    "+ Add client across hosts",
                    lambda: self.combined_dialog("approve"),
                    "primary",
                ),
                (
                    "Revoke across hosts",
                    lambda: self.combined_dialog("revoke"),
                    "danger",
                ),
                ("↻ Retry pending hosts", self.retry_combined, "secondary"),
            ],
            summary_widget=self.operations_summary,
            disable_actions=self.combined is None,
        )
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

    def table(
        self,
        columns,
        title,
        subtitle="",
        actions=(),
        cards=None,
        summary_widget=None,
        disable_actions=False,
    ):
        table = QTableWidget(0, len(columns))
        table.setHorizontalHeaderLabels(columns)
        table.horizontalHeader().setStretchLastSection(True)
        table.setAlternatingRowColors(True)
        table.setShowGrid(False)
        table.setSelectionBehavior(QTableWidget.SelectionBehavior.SelectRows)
        table.setSelectionMode(QTableWidget.SelectionMode.SingleSelection)
        table.verticalHeader().hide()
        panel = QWidget()
        panel_layout = QVBoxLayout(panel)
        panel_layout.setContentsMargins(14, 12, 14, 12)
        panel_layout.setSpacing(10)
        top_bar = QHBoxLayout()
        top_bar.setSpacing(8)
        if summary_widget is not None:
            top_bar.addWidget(summary_widget, 1)
        else:
            sub = QLabel(subtitle)
            sub.setObjectName("sectorSubtitle")
            top_bar.addWidget(sub, 1)
        for btn_title, fn, variant in actions:
            button = QPushButton(btn_title)
            button.setProperty("variant", variant)
            button.clicked.connect(fn)
            if disable_actions:
                button.setEnabled(False)
            top_bar.addWidget(button)
        panel_layout.addLayout(top_bar)
        if cards is not None:
            panel_layout.addWidget(cards)
        panel_layout.addWidget(table, 1)
        self.tabs.addTab(panel, title)
        return table

    def _clear_layout(self, box_layout):
        while box_layout.count():
            child = box_layout.takeAt(0)
            if child.widget():
                child.widget().deleteLater()

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
        self.draw_operations()
        self.online = error is None
        if error:
            self.snapshot = None
            self.access.setEnabled(False)
            self.header_banner.hyperspace = False
            self.header_banner.update()
            self.heading_label.setText("◈  NODE OFFLINE · NAVICOMPUTER")
            self.relay_badge.setText("◌ RELAY OFFLINE")
            self.clients_badge.setText("◈ 0 ACTIVE CLIENTS")
            self.streams_badge.setText("⇄ 0 STREAMS")
            self.status.setText("Node unavailable — " + error)
            self.devices.setRowCount(0)
            self.pending.setRowCount(0)
            self.services.setRowCount(0)
            self.local_cards_box.hide()
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
        self.header_banner.hyperspace = bool(result["access_enabled"])
        self.header_banner.update()
        recent = result.get("recent_peers", {})
        active = set(result["connected_peers"]) | set(recent)
        count = len(active)
        window = result.get("presence_timeout_seconds", 60)
        self.heading_label.setText(
            f"◈  {result['host_name'].upper()} · NAVICOMPUTER"
        )
        relay_ok = str(result.get("relay", "")).lower() == "connected"
        self.relay_badge.setText(
            f"{'●' if relay_ok else '◌'} RELAY: {str(result.get('relay', 'unknown')).upper()}"
        )
        self.clients_badge.setText(f"◈ {count} ACTIVE CLIENT{'S' if count != 1 else ''}")
        self.streams_badge.setText(
            f"⇄ {result['transport_streams']} STREAM{'S' if result['transport_streams'] != 1 else ''}"
        )
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
        self._clear_layout(self.local_cards_layout)
        host_services = result["host"]["services"]
        if host_services:
            for idx, s in enumerate(host_services):
                card_btn = QPushButton(
                    f"✦  {s.get('display_name') or s['id']}   ↗\n{s.get('kind', 'http').upper()} · {s['backend']}"
                )
                card_btn.setObjectName("sectorServiceCard")
                card_btn.setCursor(Qt.CursorShape.PointingHandCursor)
                card_btn.clicked.connect(
                    lambda _=False, r=idx: (
                        self.services.selectRow(r),
                        self.open_local(),
                    )
                )
                self.local_cards_layout.addWidget(card_btn)
            self.local_cards_layout.addStretch(1)
            self.local_cards_box.show()
        else:
            self.local_cards_box.hide()
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
                    "Connected"
                    if c["peer"] in result["connected_peers"]
                    else (
                        f"Seen {recent[c['peer']]}s ago"
                        if c["peer"] in recent
                        else "Idle"
                    ),
                ]
                for c in clients
            ]
            + [
                [
                    "Unapproved/revoked peer",
                    peer,
                    "Not approved",
                    count,
                    "Transport only",
                ]
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
                    self.message.setText(
                        "Invalid catalog cache ignored; refreshing its pinned host"
                    )
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
            previous["state"] = (
                "Denied"
                if "denied" in error.lower()
                else "Catalog rejected; refresh or verify this host"
                if "catalog" in error.lower()
                else "Unavailable"
            )
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
                self.message.setText(
                    "Catalog refreshed, but its cache could not be saved"
                )
        self.draw_peers()

    def draw_peers(self):
        rows = []
        self.peer_routes = []
        self._clear_layout(self.peer_cards_layout)
        pins = self.snapshot["peer_pins"] if self.snapshot else list(self.catalogs)
        for peer in pins:
            data = self.catalogs.get(peer, {})
            services = data.get("catalog", {}).get("services", [])
            state = data.get("state", "Refreshing")
            host_label = self.peer_names.get(peer, "Host " + peer[:8])
            age = (
                f"{max(0, int(time.time() - data['fetched']))}s ago"
                if "fetched" in data
                else "Never"
            )
            sector_frame = QFrame()
            sector_frame.setObjectName("peerSectorCard")
            sec_layout = QVBoxLayout(sector_frame)
            sec_layout.setContentsMargins(12, 10, 12, 10)
            sec_layout.setSpacing(6)
            sec_head = QLabel(
                f"◈  {host_label.upper()}   ·   {'● ONLINE' if state == 'Available' else '◌ ' + state.upper()}"
            )
            sec_head.setObjectName("peerSectorHeader")
            sec_layout.addWidget(sec_head)
            btn_row = QHBoxLayout()
            btn_row.setSpacing(8)
            for service in services or [None]:
                route_idx = len(self.peer_routes)
                rows.append(
                    [
                        host_label + "\n" + peer,
                        state,
                        (
                            service["display_name"]
                            + (" · TCP" if service["kind"] == "tcp" else "")
                        )
                        if service
                        else "No services",
                        age,
                    ]
                )
                self.peer_routes.append(
                    (peer, service["id"] if service else None, state)
                )
                if service:
                    s_btn = QPushButton(
                        f"✦  {service['display_name']}   ↗\n{service['kind'].upper()} · {host_label}"
                    )
                    s_btn.setObjectName("sectorServiceCard")
                    s_btn.setEnabled(state == "Available" and service["kind"] == "http")
                    s_btn.setCursor(Qt.CursorShape.PointingHandCursor)
                    s_btn.clicked.connect(
                        lambda _=False, r=route_idx: (
                            self.peers.selectRow(r),
                            self.open_peer(r, 0),
                        )
                    )
                    btn_row.addWidget(s_btn)
            if services:
                sec_layout.addLayout(btn_row)
            self.peer_cards_layout.addWidget(sector_frame)
        if pins:
            self.peer_cards_layout.addStretch(1)
            self.peer_cards_box.show()
        else:
            self.peer_cards_box.hide()
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
        self.header_banner.hyperspace = bool(enabled)
        if enabled:
            self.header_banner.jump()
        else:
            self.header_banner.drop_out()
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
        display_name = QLineEdit()
        kind = QComboBox()
        kind.addItem("Web service", "http")
        kind.addItem("TCP (native tunnel CLI)", "tcp")
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
            display_name.setText(service["display_name"])
            kind.setCurrentIndex(0 if service["kind"] == "http" else 1)
        form.addRow("Service ID", name)
        form.addRow("Display name", display_name)
        form.addRow("Service type", kind)
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
                    "display_name": display_name.text().strip() or None,
                    "kind": kind.currentData(),
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

    def draw_operations(self):
        if self.combined is None:
            return
        try:
            operations = self.combined.list()
            rows = []
            self.operation_routes = []
            for operation in operations:
                for target in operation["targets"]:
                    rows.append(
                        [
                            operation["id"][:12],
                            operation["action"]["op"]
                            + " "
                            + operation["action"]["peer"][:12],
                            target.get("ssh", "This host")
                            + " · "
                            + target["host"][:12],
                            target["state"],
                            target.get("revision") or "—",
                            target.get("reason") or "Acknowledged by this host",
                        ]
                    )
                    self.operation_routes.append(operation["id"])
            self.fill(self.operations, rows)
            pending = sum(
                t["state"] == "pending" for o in operations for t in o["targets"]
            )
            rejected = sum(
                t["state"] == "rejected" for o in operations for t in o["targets"]
            )
            complete = sum(o["complete"] for o in operations)
            self.operations_summary.setText(
                f"{complete} fully acknowledged · {pending} pending hosts · {rejected} rejected hosts"
            )
        except (OSError, ValueError, KeyError, TypeError) as error:
            self.operations_summary.setText(
                "Cannot read operation status: " + str(error)
            )

    def combined_dialog(self, op):
        if self.combined is None or not self.snapshot:
            return
        targets = [{"host": self.snapshot["host"]["host"]}]
        path = self.api.config.parent / "admin-targets.json"
        try:
            if path.exists():
                targets = self.combined.read_private(path)
            validate_plan(
                {
                    "action": {"op": "revoke", "peer": self.snapshot["host"]["host"]},
                    "targets": targets,
                }
            )
        except (OSError, ValueError, KeyError, TypeError) as error:
            self.message.setText("Cannot load owner targets: " + str(error))
            return
        dialog = QDialog(self)
        dialog.setWindowTitle(
            "Add client across hosts" if op == "approve" else "Revoke across hosts"
        )
        form = QFormLayout(dialog)
        peer = QLineEdit()
        name = QLineEdit()
        selected = self.devices.currentRow()
        if selected >= 0:
            peer.setText(self.devices.item(selected, 1).text())
            name.setText(self.devices.item(selected, 0).text())
        form.addRow("Client public ID", peer)
        if op == "approve":
            form.addRow("Client name", name)
        choices = QListWidget()
        for target in targets:
            row = QListWidgetItem(
                target.get("ssh", "This host") + " · " + target["host"]
            )
            row.setData(Qt.ItemDataRole.UserRole, target)
            row.setCheckState(Qt.CheckState.Unchecked)
            choices.addItem(row)
        form.addRow("Selected owner targets", choices)
        note = QLabel(
            "Each selected host applies independently. Offline hosts remain pending. Adding a client grants no services automatically."
        )
        note.setWordWrap(True)
        form.addRow(note)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | QDialogButtonBox.StandardButton.Cancel
        )
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        form.addRow(buttons)
        if dialog.exec() != QDialog.DialogCode.Accepted:
            return
        action = {"op": op, "peer": peer.text().strip()}
        if op == "approve":
            action["name"] = name.text().strip()
        plan = {
            "action": action,
            "targets": [
                choices.item(i).data(Qt.ItemDataRole.UserRole)
                for i in range(choices.count())
                if choices.item(i).checkState() == Qt.CheckState.Checked
            ],
        }
        try:
            validate_plan(plan)
        except ValueError as error:
            self.message.setText(str(error))
            return
        if (
            QMessageBox.question(
                self,
                "Confirm host operations",
                "Apply this client action independently on "
                + str(len(plan["targets"]))
                + " selected hosts? Offline targets will remain pending.",
            )
            != QMessageBox.StandardButton.Yes
        ):
            return
        self.submit(lambda: self.combined.start(plan), self.combined_result)
        self.tabs.setCurrentIndex(4)

    def retry_combined(self):
        row = self.operations.currentRow()
        if self.combined is None or row < 0 or row >= len(self.operation_routes):
            return
        operation = self.operation_routes[row]
        self.submit(lambda: self.combined.retry(operation), self.combined_result)

    def combined_result(self, value, error):
        self.draw_operations()
        if error:
            self.message.setText(str(error))
        elif value["complete"]:
            self.message.setText(
                "Every selected host acknowledged this operation. Current permissions may have changed since it was applied."
            )
        else:
            self.message.setText(
                "Not fully applied. Review each host's pending or rejected status."
            )

    def open_local(self, *args):
        row = self.services.currentRow()
        if not self.online or not self.snapshot or row < 0:
            return
        if not self.access.isChecked():
            self.message.setText("Enable Browser access first")
            return
        host = self.snapshot["host"]["host"]
        service = self.services.item(row, 0).text()
        incarnation = next(
            s["incarnation"]
            for s in self.snapshot["host"]["services"]
            if s["id"] == service
        )
        self.submit(
            lambda: self.api.request(
                op="open", host=host, service=service, incarnation=incarnation
            ),
            self.open_result,
        )

    def open_peer(self, row, column):
        if row >= len(self.peer_routes):
            return
        peer, service, state = self.peer_routes[row]
        incarnation = next(
            (
                s.get("incarnation")
                for s in self.catalogs.get(peer, {})
                .get("catalog", {})
                .get("services", [])
                if s["id"] == service
            ),
            None,
        )
        if not service or state != "Available" or not self.online:
            self.message.setText("Refresh this peer before opening its service")
            return
        if not self.access.isChecked():
            self.message.setText("Enable Browser access first")
            return
        record = next(
            s for s in self.catalogs[peer]["catalog"]["services"] if s["id"] == service
        )
        if record["kind"] != "http":
            self.message.setText("This TCP service requires the native tunnel CLI")
            return
        self.submit(
            lambda: self.api.request(
                op="open", host=peer, service=service, incarnation=incarnation
            ),
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
        QWidget#headerCard QLabel {
            background: transparent;
        }
        QLabel#heading {
            font-size: 20px;
            font-weight: 700;
            letter-spacing: 1.5px;
            color: #f4f8ff;
        }
        QLabel#statusText {
            font-size: 12px;
            color: #8fa6c8;
        }
        QLabel#telemetryBadge {
            background: rgba(16, 28, 52, 210);
            color: #c6dcff;
            border: 1px solid #2b4475;
            border-radius: 10px;
            padding: 3px 10px;
            font-size: 11px;
            font-weight: 700;
            letter-spacing: 0.6px;
        }
        QLabel#sectorSubtitle {
            color: #94a6c4;
            font-size: 13px;
            background: transparent;
        }
        QFrame#peerSectorCard {
            background: #0b1426;
            border: 1px solid #263e6c;
            border-radius: 14px;
        }
        QLabel#peerSectorHeader {
            color: #edf4ff;
            font-size: 12px;
            font-weight: 700;
            letter-spacing: 1px;
            background: transparent;
        }
        QPushButton#sectorServiceCard {
            background: #111e38;
            border: 1px solid #32518a;
            border-radius: 12px;
            padding: 10px 16px;
            color: #f4f8ff;
            font-weight: 700;
            text-align: left;
            min-width: 170px;
        }
        QPushButton#sectorServiceCard:hover {
            background: #192d54;
            border-color: #65ceff;
        }
        QPushButton#sectorServiceCard:disabled {
            background: #0d1628;
            border-color: #1d2842;
            color: #687a98;
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
            padding: 8px 14px;
            border: 1px solid #283b60;
            border-radius: 11px;
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
        QTabWidget::pane {
            border: 1px solid #243659;
            border-radius: 16px;
            background: #0b1222;
            top: -1px;
        }
        QTabBar::tab {
            background: #0f182c;
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
            border: 1px solid #1c2b48;
            border-radius: 12px;
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
            padding: 10px 12px;
            border: 0;
            border-bottom: 1px solid #243659;
            font-weight: 600;
        }
        QLineEdit,
        QComboBox, QListWidget {
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
