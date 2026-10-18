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
    QScrollArea,
    QStackedWidget,
)
import math
import random
from .api import NodeAPI


class StarfieldBanner(QWidget):
    """Star Wars-inspired navicomputer header with drifting 3D starfield and 3-beat hyperjump."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("headerCard")
        self.setMinimumHeight(92)
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
        nebula = QRadialGradient(w * 0.5, cy, max(1.0, w * 0.45))
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
        self.setFixedSize(208, 44)

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
                QColor("#65ceff" if on else "#2e4674" if enabled else "#1e2c46"),
                1.6,
            )
        )
        painter.setBrush(
            QColor("#163264" if on else "#0f1a30" if enabled else "#0b1220")
        )
        painter.drawRoundedRect(2, 2, w - 4, h - 4, (h - 4) / 2, (h - 4) / 2)

        bx, by = 24, h // 2
        if on:
            painter.setPen(Qt.PenStyle.NoPen)
            painter.setBrush(QColor(101, 206, 255, 76))
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
        font.setLetterSpacing(QFont.SpacingType.AbsoluteSpacing, 1.0)
        painter.setFont(font)
        label = "HYPERSPACE ON" if on else "JUMP TO SPACE"
        painter.drawText(
            42, 0, w - 50, h, Qt.AlignmentFlag.AlignVCenter | Qt.AlignmentFlag.AlignLeft, label
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
        self.resize(860, 660)

        main = QWidget()
        layout = QVBoxLayout(main)
        layout.setContentsMargins(20, 18, 20, 18)
        layout.setSpacing(14)
        self.setCentralWidget(main)

        # ── Minimal Starfield Command Header ──
        self.header_banner = StarfieldBanner()
        header_layout = QHBoxLayout(self.header_banner)
        header_layout.setContentsMargins(20, 14, 20, 14)
        header_layout.setSpacing(14)

        logo = QLabel()
        logo.setPixmap(icon().pixmap(40, 40))
        logo.setFixedSize(40, 40)
        header_layout.addWidget(logo)

        title_box = QVBoxLayout()
        title_box.setSpacing(2)
        self.heading_label = QLabel("◈  WISPNET")
        self.heading_label.setObjectName("heading")
        title_box.addWidget(self.heading_label)
        self.subtitle_label = QLabel("◌ Sublight Standby")
        self.subtitle_label.setObjectName("statusText")
        title_box.addWidget(self.subtitle_label)
        header_layout.addLayout(title_box, 1)

        self.access = HyperspaceToggle("Browser access")
        self.access.clicked.connect(self.toggle_access)
        header_layout.addWidget(self.access)

        self.link_btn = QPushButton("+  Link")
        self.link_btn.setProperty("variant", "primary")
        self.link_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        self.link_btn.clicked.connect(self.pair)
        header_layout.addWidget(self.link_btn)

        self.settings_btn = QPushButton("⚙")
        self.settings_btn.setProperty("variant", "secondary")
        self.settings_btn.setToolTip("Fleet, requests & diagnostics")
        self.settings_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        self.settings_btn.clicked.connect(self.toggle_settings_view)
        header_layout.addWidget(self.settings_btn)

        layout.addWidget(self.header_banner)

        # ── Incoming Pairing Alert Banner (only shown when a request is pending) ──
        self.incoming_banner = QFrame()
        self.incoming_banner.setObjectName("incomingBanner")
        incoming_layout = QHBoxLayout(self.incoming_banner)
        incoming_layout.setContentsMargins(14, 10, 14, 10)
        incoming_layout.setSpacing(10)
        self.incoming_label = QLabel("")
        self.incoming_label.setObjectName("incomingText")
        incoming_layout.addWidget(self.incoming_label, 1)
        approve_btn = QPushButton("✓ Approve")
        approve_btn.setProperty("variant", "primary")
        approve_btn.clicked.connect(self._approve_first_pending)
        reject_btn = QPushButton("✕ Reject")
        reject_btn.setProperty("variant", "danger")
        reject_btn.clicked.connect(self._reject_first_pending)
        incoming_layout.addWidget(approve_btn)
        incoming_layout.addWidget(reject_btn)
        self.incoming_banner.hide()
        layout.addWidget(self.incoming_banner)

        # ── Main Content Stack: Page 0 = Simple Home Canvas, Page 1 = Settings & Diagnostics ──
        self.view_stack = QStackedWidget()
        layout.addWidget(self.view_stack, 1)

        # Page 0: Single-Page Navicomputer Canvas (No Tabs, No Tables)
        self.home_page = QScrollArea()
        self.home_page.setWidgetResizable(True)
        self.home_page.setFrameShape(QFrame.Shape.NoFrame)
        self.home_page.setObjectName("homeScroll")
        home_container = QWidget()
        home_container.setObjectName("homeContainer")
        home_layout = QVBoxLayout(home_container)
        home_layout.setContentsMargins(2, 4, 2, 8)
        home_layout.setSpacing(16)

        # Section 1: This Machine
        local_sector = QFrame()
        local_sector.setObjectName("peerSectorCard")
        local_sector_layout = QVBoxLayout(local_sector)
        local_sector_layout.setContentsMargins(16, 14, 16, 14)
        local_sector_layout.setSpacing(10)
        local_head_row = QHBoxLayout()
        self.local_sector_title = QLabel("◈  THIS MACHINE")
        self.local_sector_title.setObjectName("peerSectorHeader")
        local_head_row.addWidget(self.local_sector_title, 1)
        share_btn = QPushButton("+  Share Port")
        share_btn.setProperty("variant", "primary")
        share_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        share_btn.clicked.connect(self.share)
        local_head_row.addWidget(share_btn)
        local_sector_layout.addLayout(local_head_row)

        self.local_cards_box = QWidget()
        self.local_cards_layout = QGridLayout(self.local_cards_box)
        self.local_cards_layout.setContentsMargins(0, 2, 0, 0)
        self.local_cards_layout.setSpacing(10)
        local_sector_layout.addWidget(self.local_cards_box)
        home_layout.addWidget(local_sector)

        # Section 2: Linked Machines & Devices (Wisp Space)
        peers_head_row = QHBoxLayout()
        peers_head_row.setContentsMargins(4, 4, 4, 0)
        peers_title = QLabel("◈  LINKED MACHINES & DEVICES")
        peers_title.setObjectName("peerSectorHeader")
        peers_head_row.addWidget(peers_title, 1)
        scan_btn = QPushButton("↻  Refresh")
        scan_btn.setProperty("variant", "secondary")
        scan_btn.setCursor(Qt.CursorShape.PointingHandCursor)
        scan_btn.clicked.connect(self.refresh)
        peers_head_row.addWidget(scan_btn)
        home_layout.addLayout(peers_head_row)

        self.peer_cards_box = QWidget()
        self.peer_cards_layout = QVBoxLayout(self.peer_cards_box)
        self.peer_cards_layout.setContentsMargins(0, 0, 0, 0)
        self.peer_cards_layout.setSpacing(12)
        home_layout.addWidget(self.peer_cards_box)
        home_layout.addStretch(1)
        self.home_page.setWidget(home_container)
        self.view_stack.addWidget(self.home_page)

        # Page 1: Settings, Fleet & Diagnostics (Tucked behind ⚙ button)
        self.settings_page = QWidget()
        settings_layout = QVBoxLayout(self.settings_page)
        settings_layout.setContentsMargins(0, 0, 0, 0)
        settings_layout.setSpacing(10)
        self.status = QLabel("Connecting to local node…")
        self.status.setObjectName("statusText")
        self.status.setTextFormat(Qt.TextFormat.PlainText)
        self.status.setWordWrap(True)
        settings_layout.addWidget(self.status)
        self.tabs = QTabWidget()
        settings_layout.addWidget(self.tabs, 1)
        self.view_stack.addWidget(self.settings_page)

        self.services = self.table(
            ["Service", "Backend", "Permitted clients"],
            "Local Ports",
            "Loopback services shared from this machine",
            [
                ("✦ Open", self.open_local, "primary"),
                ("+ Share Port", self.share, "primary"),
                ("Edit", self.edit, "secondary"),
                ("Remove", self.remove, "danger"),
            ],
        )
        self.devices = self.table(
            ["Client", "Peer ID", "Approval", "Live connections", "Activity"],
            "Paired Devices",
            "Phones and computers allowed to access services on this machine",
            [
                (
                    "+ Add across hosts",
                    lambda: self.combined_dialog("approve"),
                    "primary",
                ),
                ("Revoke device", self.revoke, "danger"),
            ],
        )
        self.peers = self.table(
            ["Peer", "Status", "Shared services", "Last refreshed"],
            "Linked Machines",
            "Remote machines linked from this computer",
            [
                (
                    "✦ Open service",
                    lambda: self.open_peer(max(0, self.peers.currentRow()), 0),
                    "primary",
                ),
                ("+ Link Machine", self.pair, "primary"),
            ],
        )
        self.pending = self.table(
            ["Invitation", "Client", "Peer ID", "State"],
            "Link Requests",
            "Incoming pairing requests on this machine",
            [
                ("✓ Approve", lambda: self.decide(True), "primary"),
                ("✕ Reject", lambda: self.decide(False), "danger"),
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
            "Diagnostics",
            None,
            [
                (
                    "+ Add across hosts",
                    lambda: self.combined_dialog("approve"),
                    "primary",
                ),
                (
                    "Revoke across hosts",
                    lambda: self.combined_dialog("revoke"),
                    "danger",
                ),
                ("↻ Retry pending", self.retry_combined, "secondary"),
            ],
            summary_widget=self.operations_summary,
            disable_actions=self.combined is None,
        )
        self.view_stack.setCurrentWidget(self.home_page)
        self.tabs.currentChanged.connect(self._on_tab_selected)

        self.message = QLabel("")
        self.message.setObjectName("messageBar")
        self.message.setTextFormat(Qt.TextFormat.PlainText)
        self.message.setWordWrap(True)
        self.message.hide()
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

    def toggle_settings_view(self):
        if self.view_stack.currentWidget() is self.settings_page:
            self.view_stack.setCurrentWidget(self.home_page)
            self.settings_btn.setText("⚙")
        else:
            self.view_stack.setCurrentWidget(self.settings_page)
            self.settings_btn.setText("←  Back")

    def _on_tab_selected(self, _index):
        self.view_stack.setCurrentWidget(self.settings_page)
        self.settings_btn.setText("←  Back")

    def _approve_first_pending(self):
        for r in range(self.pending.rowCount()):
            if self.pending.item(r, 3) and self.pending.item(r, 3).text() == "pending":
                self.pending.selectRow(r)
                self.decide(True)
                return

    def _reject_first_pending(self):
        for r in range(self.pending.rowCount()):
            if self.pending.item(r, 3) and self.pending.item(r, 3).text() == "pending":
                self.pending.selectRow(r)
                self.decide(False)
                return

    def _set_message(self, text):
        self.message.setText(text)
        if text:
            self.message.show()
        else:
            self.message.hide()

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
            self.heading_label.setText("◈  NODE OFFLINE")
            self.subtitle_label.setText("✕ Local background service unavailable")
            self.status.setText("Node unavailable — " + error)
            self.incoming_banner.hide()
            self.devices.setRowCount(0)
            self.pending.setRowCount(0)
            self.services.setRowCount(0)
            self._clear_layout(self.local_cards_layout)
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
        host_title = result["host_name"].upper()
        self.heading_label.setText(f"◈  {host_title}")
        self.local_sector_title.setText(f"◈  THIS MACHINE · {host_title}")
        if result["access_enabled"]:
            self.subtitle_label.setText(
                f"● Hyperspace Active  ·  {count} connected device{'s' if count != 1 else ''}"
            )
        else:
            self.subtitle_label.setText(
                f"◌ Sublight Standby  ·  {len(result['host']['clients'])} paired device{'s' if len(result['host']['clients']) != 1 else ''}"
            )
        self.status.setText(
            f"{result['host_name']}  ·  Active clients: {count}  ·  {result['transport_streams']} transport streams  ·  Activity window: {window}s  ·  Relay: {result['relay']}"
        )
        pending_reqs = [i for i in result["invitations"] if i["state"] == "pending"]
        if pending_reqs:
            first_req = pending_reqs[0]
            who = first_req.get("name") or (first_req.get("peer") or "")[:12] or "Device"
            self.incoming_label.setText(
                f"⚡ Incoming Link Request from {who} ({len(pending_reqs)} pending)"
            )
            self.incoming_banner.show()
        else:
            self.incoming_banner.hide()
        if self.tray:
            pending = len(pending_reqs)
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
                tile = QFrame()
                tile.setObjectName("serviceTileFrame")
                tile_layout = QHBoxLayout(tile)
                tile_layout.setContentsMargins(12, 10, 10, 10)
                tile_layout.setSpacing(8)
                port_label = s["backend"].removeprefix("127.0.0.1:")
                open_btn = QPushButton(
                    f"✦  {s.get('display_name') or s['id']}   ↗\n:{port_label}  ·  {len(s['allow'])} device{'s' if len(s['allow']) != 1 else ''}"
                )
                open_btn.setObjectName("sectorServiceCard")
                open_btn.setCursor(Qt.CursorShape.PointingHandCursor)
                open_btn.clicked.connect(
                    lambda _=False, r=idx: (
                        self.services.selectRow(r),
                        self.open_local(),
                    )
                )
                tile_layout.addWidget(open_btn, 1)
                del_btn = QPushButton("✕")
                del_btn.setObjectName("tileIconBtn")
                del_btn.setToolTip("Stop sharing this port")
                del_btn.setCursor(Qt.CursorShape.PointingHandCursor)
                del_btn.clicked.connect(
                    lambda _=False, r=idx: (
                        self.services.selectRow(r),
                        self.remove(),
                    )
                )
                tile_layout.addWidget(del_btn)
                self.local_cards_layout.addWidget(tile, idx // 2, idx % 2)
        else:
            empty = QLabel(
                "No local ports shared yet — click '+ Share Port' to expose a local web app."
            )
            empty.setObjectName("sectorSubtitle")
            self.local_cards_layout.addWidget(empty, 0, 0, 1, 2)
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
                    self._set_message(
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
                self._set_message(
                    "Catalog refreshed, but its cache could not be saved"
                )
        self.draw_peers()

    def _format_telemetry(self, peer):
        if not self.snapshot:
            return ""
        info = self.snapshot.get("peer_telemetry", {}).get(peer)
        if not isinstance(info, dict):
            return ""
        path = info.get("path")
        rtt = info.get("rtt_ms")
        parts = []
        if path in ("direct", "relay"):
            parts.append("Last path: " + path.upper())
        if isinstance(rtt, int) and rtt > 0:
            parts.append(f"{rtt}ms")
        return " · ".join(parts)

    def draw_peers(self):
        rows = []
        self.peer_routes = []
        self._clear_layout(self.peer_cards_layout)
        pins = self.snapshot["peer_pins"] if self.snapshot else list(self.catalogs)
        clients = self.snapshot["host"]["clients"] if self.snapshot else []
        client_by_peer = {c["peer"]: c for c in clients}
        connected = self.snapshot.get("connected_peers", {}) if self.snapshot else {}
        recent = self.snapshot.get("recent_peers", {}) if self.snapshot else {}
        local_services = (
            self.snapshot["host"].get("services", []) if self.snapshot else []
        )

        for peer in pins:
            data = self.catalogs.get(peer, {})
            services = data.get("catalog", {}).get("services", [])
            state = data.get("state", "Refreshing")
            host_label = (
                self.peer_names.get(peer)
                or (client_by_peer[peer]["name"] if peer in client_by_peer else None)
                or ("Host " + peer[:8])
            )
            age = (
                f"{max(0, int(time.time() - data['fetched']))}s ago"
                if "fetched" in data
                else "Never"
            )
            tel_badge = self._format_telemetry(peer)
            status_badge = (
                f"● ONLINE · {tel_badge}"
                if state == "Available" and tel_badge
                else "● ONLINE"
                if state == "Available"
                else f"◌ {state.upper()}"
            )
            sector_frame = QFrame()
            sector_frame.setObjectName("peerSectorCard")
            sec_layout = QVBoxLayout(sector_frame)
            sec_layout.setContentsMargins(16, 14, 16, 14)
            sec_layout.setSpacing(10)
            sec_head = QLabel(
                f"◈  {host_label.upper()} · {peer[:8]}   ·   {status_badge}"
            )
            sec_head.setObjectName("peerSectorHeader")
            sec_layout.addWidget(sec_head)
            grid = QGridLayout()
            grid.setSpacing(10)
            for idx, service in enumerate(services or [None]):
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
                    grid.addWidget(s_btn, idx // 2, idx % 2)
            if services:
                sec_layout.addLayout(grid)
            else:
                no_svc = QLabel("No shared services announced by this machine.")
                no_svc.setObjectName("sectorSubtitle")
                sec_layout.addWidget(no_svc)
            self.peer_cards_layout.addWidget(sector_frame)

        # Also display paired client devices (e.g. Android phones/tablets) that connect to this machine
        pin_set = set(pins)
        client_only = [c for c in clients if c["peer"] not in pin_set]
        for client in client_only:
            c_peer = client["peer"]
            c_name = client["name"] or ("Device " + c_peer[:8])
            tel_badge = self._format_telemetry(c_peer)
            if c_peer in connected:
                streams = connected[c_peer]
                c_status = f"● CONNECTED ({streams} stream{'s' if streams != 1 else ''})"
                if tel_badge:
                    c_status += f" · {tel_badge}"
            elif c_peer in recent:
                c_status = f"● ACTIVE ({recent[c_peer]}s AGO)"
                if tel_badge:
                    c_status += f" · {tel_badge}"
            else:
                c_status = "◌ STANDBY · CLIENT"
                if tel_badge:
                    c_status += f" · LAST: {tel_badge}"
            allowed = [
                s.get("display_name") or s["id"]
                for s in local_services
                if c_peer in s.get("allow", [])
            ]
            sector_frame = QFrame()
            sector_frame.setObjectName("peerSectorCard")
            sec_layout = QVBoxLayout(sector_frame)
            sec_layout.setContentsMargins(16, 12, 16, 12)
            sec_layout.setSpacing(6)
            head_row = QHBoxLayout()
            sec_head = QLabel(
                f"◈  {c_name.upper()} · {c_peer[:8]}   ·   {c_status}"
            )
            sec_head.setObjectName("peerSectorHeader")
            head_row.addWidget(sec_head, 1)
            revoke_btn = QPushButton("✕")
            revoke_btn.setObjectName("tileIconBtn")
            revoke_btn.setToolTip("Revoke this device's access")
            revoke_btn.setCursor(Qt.CursorShape.PointingHandCursor)
            revoke_btn.clicked.connect(
                lambda _=False, p=c_peer: self._revoke_peer_by_id(p)
            )
            head_row.addWidget(revoke_btn)
            sec_layout.addLayout(head_row)
            access_summary = (
                "Can access on this machine: " + ", ".join(allowed)
                if allowed
                else "No local ports granted yet"
            )
            info_lbl = QLabel(
                f"Client device (no hosted services)   ·   {access_summary}"
            )
            info_lbl.setObjectName("sectorSubtitle")
            sec_layout.addWidget(info_lbl)
            self.peer_cards_layout.addWidget(sector_frame)

        if not pins and not client_only:
            empty_frame = QFrame()
            empty_frame.setObjectName("peerSectorCard")
            empty_layout = QHBoxLayout(empty_frame)
            empty_layout.setContentsMargins(16, 14, 16, 14)
            empty_lbl = QLabel(
                "No machines or devices linked yet — click '+ Link' in the top bar to connect."
            )
            empty_lbl.setObjectName("sectorSubtitle")
            empty_layout.addWidget(empty_lbl, 1)
            self.peer_cards_layout.addWidget(empty_frame)
        self.fill(
            self.peers,
            rows
            or [["No paired hosts — use Pair with a host", "Not paired", "—", "—"]],
        )

    def _revoke_peer_by_id(self, peer):
        for r in range(self.devices.rowCount()):
            if self.devices.item(r, 1) and self.devices.item(r, 1).text() == peer:
                self.devices.selectRow(r)
                self.revoke()
                return

    def perform(self, request, success="Saved"):
        self.submit(
            lambda: self.api.request(**request),
            lambda result, error: self.result(result, error, success),
        )

    def result(self, value, error, success):
        self._set_message(error or success)
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
        dialog.setWindowTitle("Share a local port")
        dialog.setMinimumWidth(400)
        form = QFormLayout(dialog)
        display_name = QLineEdit()
        display_name.setPlaceholderText("e.g. Jellyfin, Syncthing, Dashboard")
        backend = QLineEdit()
        backend.setPlaceholderText("e.g. 8096 or 127.0.0.1:8096")
        kind = QComboBox()
        kind.addItem("Web service (HTTP)", "http")
        kind.addItem("TCP (native tunnel CLI)", "tcp")
        clients = QListWidget()
        for client in self.snapshot["host"]["clients"]:
            row = QListWidgetItem(client["name"] + " · " + client["peer"][:12])
            row.setData(Qt.ItemDataRole.UserRole, client["peer"])
            row.setFlags(row.flags() | Qt.ItemFlag.ItemIsUserCheckable)
            row.setCheckState(
                Qt.CheckState.Checked
                if (
                    not existing
                    or client["peer"]
                    in next(
                        s["allow"]
                        for s in self.snapshot["host"]["services"]
                        if s["id"] == existing
                    )
                )
                else Qt.CheckState.Unchecked
            )
            clients.addItem(row)
        if existing:
            service = next(
                s for s in self.snapshot["host"]["services"] if s["id"] == existing
            )
            display_name.setText(service["display_name"] or existing)
            backend.setText(service["backend"])
            kind.setCurrentIndex(0 if service["kind"] == "http" else 1)
        form.addRow("Service name", display_name)
        form.addRow("Local port", backend)
        form.addRow("Protocol", kind)
        if self.snapshot["host"]["clients"]:
            form.addRow("Allow paired devices", clients)
        buttons = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Save
            | QDialogButtonBox.StandardButton.Cancel
        )
        form.addRow(buttons)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        if dialog.exec():
            raw_title = display_name.text().strip()
            raw_port = backend.text().strip()
            if raw_port.isdigit():
                raw_port = f"127.0.0.1:{raw_port}"
            elif raw_port.startswith(":"):
                raw_port = f"127.0.0.1{raw_port}"
            svc_id = (
                existing
                or re.sub(r"[^a-z0-9-]+", "-", raw_title.lower()).strip("-")
                or "service"
            )
            if (
                any(
                    s["id"] == svc_id
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
                    "id": svc_id,
                    "backend": raw_port,
                    "display_name": raw_title or svc_id,
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
            self._set_message("Only proved pending requests can be approved")
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
        dialog.setWindowTitle("Link a Remote Machine")
        dialog.setMinimumWidth(420)
        form = QFormLayout(dialog)
        hint = QLabel(
            "Paste the pairing code from the remote machine or import an invitation file."
        )
        hint.setObjectName("sectorSubtitle")
        hint.setWordWrap(True)
        form.addRow(hint)
        code = QLineEdit()
        code.setPlaceholderText("Paste pairing code or JSON…")
        code.setEchoMode(QLineEdit.EchoMode.Password)
        form.addRow("Pairing code", code)
        load = QPushButton("Import invitation file…")
        load.setProperty("variant", "secondary")
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
                    self._set_message(str(error))

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
            self._set_message(error)
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
            self._set_message(error)
            return
        peer = preview["host"]
        self.catalogs.pop(peer, None)
        # Explicit target re-pairing permits a new catalog generation baseline.
        try:
            self.catalog_cache.remove(peer)
        except (OSError, ValueError):
            self._set_message("Cannot reset the catalog cache; retry pairing")
            return
        self.cache_loaded.add(peer)
        self.peer_names[peer] = preview.get("host_name") or "Host " + peer[:8]
        path = None
        try:
            fd, path = tempfile.mkstemp(prefix="peer-names-", dir=self.api.state)
            with os.fdopen(fd, "w") as output:
                json.dump(self.peer_names, output)
            os.replace(path, self.api.state / "desktop-peer-names.json")
            self._set_message(
                "Request sent. Approve this node on the target host and grant services."
            )
        except OSError:
            self._set_message(
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
            self._set_message("Cannot load owner targets: " + str(error))
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
            self._set_message(str(error))
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
            self._set_message(str(error))
        elif value["complete"]:
            self._set_message(
                "Every selected host acknowledged this operation. Current permissions may have changed since it was applied."
            )
        else:
            self._set_message(
                "Not fully applied. Review each host's pending or rejected status."
            )

    def open_local(self, *args):
        row = self.services.currentRow()
        if not self.online or not self.snapshot or row < 0:
            return
        if not self.access.isChecked():
            self._set_message("Enable Browser access first")
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
            self._set_message("Refresh this peer before opening its service")
            return
        if not self.access.isChecked():
            self._set_message("Enable Browser access first")
            return
        record = next(
            s for s in self.catalogs[peer]["catalog"]["services"] if s["id"] == service
        )
        if record["kind"] != "http":
            self._set_message("This TCP service requires the native tunnel CLI")
            return
        self.submit(
            lambda: self.api.request(
                op="open", host=peer, service=service, incarnation=incarnation
            ),
            self.open_result,
        )

    def open_result(self, value, error):
        if error:
            self._set_message(error)
            return
        url = QUrl(value["url"])
        parsed = urlsplit(value["url"])
        if (
            parsed.scheme != "http"
            or not parsed.hostname
            or not parsed.hostname.endswith(".localhost")
            or parsed.path != "/_wipsnet/start"
        ):
            self._set_message("Invalid local browser handoff")
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
            self._set_message("Could not open your browser")


def appearance(app):
    app.setStyle("Fusion")
    app.setStyleSheet(
        """
        QWidget {
            background: #070b14;
            color: #edf2ff;
            font-size: 14px;
        }
        QMainWindow, QDialog, QMessageBox, QScrollArea#homeScroll, QWidget#homeContainer {
            background: #070b14;
        }
        QWidget#headerCard QLabel {
            background: transparent;
        }
        QLabel#heading {
            font-size: 19px;
            font-weight: 700;
            letter-spacing: 1.4px;
            color: #f4f8ff;
        }
        QLabel#statusText {
            font-size: 12px;
            color: #93accf;
        }
        QFrame#incomingBanner {
            background: #13284c;
            border: 1px solid #65ceff;
            border-radius: 14px;
        }
        QLabel#incomingText {
            color: #f4f8ff;
            font-weight: 600;
            background: transparent;
        }
        QLabel#sectorSubtitle {
            color: #8ea3c3;
            font-size: 13px;
            background: transparent;
        }
        QFrame#peerSectorCard {
            background: #0b1426;
            border: 1px solid #223862;
            border-radius: 16px;
        }
        QFrame#serviceTileFrame {
            background: #101d36;
            border: 1px solid #2b477c;
            border-radius: 12px;
        }
        QLabel#peerSectorHeader {
            color: #dce9ff;
            font-size: 12px;
            font-weight: 700;
            letter-spacing: 1.1px;
            background: transparent;
        }
        QPushButton#sectorServiceCard {
            background: #111e38;
            border: 1px solid #32518a;
            border-radius: 12px;
            padding: 10px 14px;
            color: #f4f8ff;
            font-weight: 700;
            text-align: left;
        }
        QFrame#serviceTileFrame QPushButton#sectorServiceCard {
            background: transparent;
            border: 0;
            padding: 4px 6px;
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
        QPushButton#tileIconBtn {
            background: transparent;
            border: 1px solid transparent;
            border-radius: 8px;
            color: #8ea3c3;
            padding: 4px 8px;
            font-size: 13px;
        }
        QPushButton#tileIconBtn:hover {
            background: #2b182e;
            border-color: #66304e;
            color: #ffb3c4;
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
            padding: 9px 16px;
            margin-right: 6px;
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
