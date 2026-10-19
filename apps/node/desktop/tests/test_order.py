import unittest
from types import SimpleNamespace
from wispnet_desktop.app import Window


class Settings:
    def value(self, group, default):
        return ["removed", "b", "a"]


class OrderTests(unittest.TestCase):
    def test_saved_order_survives_refresh_and_appends_new_cards(self):
        window = SimpleNamespace(order_settings=Settings(), order_visible={})
        cards = [{"id": "a"}, {"id": "new"}, {"id": "b"}]
        result = Window.ordered(window, "services/host", cards, lambda card: card["id"])
        self.assertEqual([card["id"] for card in result], ["b", "a", "new"])
        self.assertEqual(window.order_visible["services/host"], ["b", "a", "new"])
        self.assertEqual(cards, [{"id": "a"}, {"id": "new"}, {"id": "b"}])

    def test_drag_preview_drop_cancel_and_refresh_guard(self):
        from unittest.mock import patch
        from PySide6.QtWidgets import QApplication, QWidget, QVBoxLayout, QLabel
        from PySide6.QtCore import QPoint, Qt
        from PySide6.QtTest import QTest
        from wispnet_desktop.app import ReorderHandle

        app = QApplication.instance() or QApplication([])
        saved = {}
        window = QWidget()
        window.order_visible = {"machines": ["a", "b"]}
        window.order_settings = SimpleNamespace(setValue=lambda group, order: saved.update({group: order}))
        window.reordering = False
        window.finish_reorder = lambda: None
        window.deferred_update = None
        layout = QVBoxLayout(window)
        handles = []
        for group, key in [("machines", "a"), ("machines", "b"), ("services/host", "x")]:
            card = QWidget()
            body = QVBoxLayout(card)
            handle = ReorderHandle(window, group, key, card)
            body.addWidget(handle)
            body.addWidget(QLabel("Named card " + key))
            layout.addWidget(card)
            handles.append(handle)
        window.resize(400, 400)
        window.show()
        app.processEvents()
        source, target, other = handles
        with patch("wispnet_desktop.app.QTimer.singleShot"):
            QTest.mousePress(source, Qt.MouseButton.LeftButton)
            QTest.mouseMove(source, QPoint(source.width() + 40, 20))
            self.assertTrue(window.reordering)
            source.move_drag(other.parentWidget().mapToGlobal(QPoint(10, 10)))
            self.assertIsNone(source.target)
            source.move_drag(target.parentWidget().mapToGlobal(QPoint(10, 10)))
            self.assertIs(source.target, target)
            self.assertFalse(source.preview.pixmap().isNull())
            self.assertTrue(source.preview.isVisible())
            self.assertTrue(source.outline.isVisible())
            # Polling must not delete the source or target during a drag.
            Window.updated(window, {"latest": True}, None)
            self.assertEqual(window.deferred_update, ({"latest": True}, None))
            Window.draw_peers(window)
            QTest.mouseRelease(source, Qt.MouseButton.LeftButton)
            self.assertFalse(window.reordering)
            self.assertEqual(saved, {"machines": ["b", "a"]})
            saved.clear()
            source.begin_drag()
            source.move_drag(target.parentWidget().mapToGlobal(QPoint(10, 10)))
            QTest.keyClick(source, Qt.Key.Key_Escape)
            self.assertEqual(saved, {})
        window.close()
        window.deleteLater()
        app.processEvents()

    def test_deferred_status_is_applied_after_drag(self):
        calls = []
        window = SimpleNamespace(deferred_update=(None, "offline"),
                                 updated=lambda result, error: calls.append((result, error)))
        Window.finish_reorder(window)
        self.assertEqual(calls, [(None, "offline")])
        self.assertIsNone(window.deferred_update)
