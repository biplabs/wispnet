import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/terminal_sheet.dart';
import 'package:wispnet/ui.dart';

import 'widget_test.dart' as fixture;

class NativeTerminalBackend implements WispnetBackend {
  final calls = <(String, Map<String, dynamic>)>[];
  Completer<Map<String, dynamic>>? opening;

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, Map.of(arguments)));
    if (method == 'nativeTerminalOpen' && opening != null) {
      return opening!.future;
    }
    return {
      'state': 'attached',
      'echo_enabled': true,
      'canonical': true,
      'path': 'direct',
      'latency_ms': 15,
    };
  }
}

void main() {
  testWidgets('terminal fits a narrow phone when the keyboard opens', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(360, 640);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      (call) async {
        if (call.method == 'create') return 1;
        if (call.method == 'resize') {
          final args = call.arguments as Map;
          return {'width': args['width'], 'height': args['height']};
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform_views,
        null,
      ),
    );
    final backend = NativeTerminalBackend();
    final manager = DesktopShellManager(backend)..android = true;
    final workspace = manager.openHost(
      host: fixture.peer,
      hostName: 'Computer',
      service: 'shell',
      incarnation: fixture.incarnation,
    );
    final keyboard = ValueNotifier(0.0);
    await tester.pumpWidget(
      MaterialApp(
        theme: wispTheme(),
        builder: (context, child) => ValueListenableBuilder<double>(
          valueListenable: keyboard,
          builder: (context, inset, _) => MediaQuery(
            data: MediaQuery.of(context).copyWith(
              padding: const EdgeInsets.only(top: 24),
              viewPadding: const EdgeInsets.only(top: 24),
              viewInsets: EdgeInsets.only(bottom: inset),
            ),
            child: child!,
          ),
        ),
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => showWispSheet<void>(
                context,
                title: 'Computer',
                showHeader: false,
                scrollable: false,
                child: DesktopTerminalSheet(
                  manager: manager,
                  workspace: workspace,
                  onOpenExternal: (_) async {},
                ),
              ),
              child: const Text('Open terminal'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open terminal'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    keyboard.value = 300;
    await tester.pumpAndSettle();
    expect(
      MediaQuery.viewInsetsOf(tester.element(find.byType(DesktopTerminalSheet)))
          .bottom,
      300,
    );
    expect(tester.takeException(), isNull);
    final shortcut = find.widgetWithText(TextButton, 'Ctrl-C');
    expect(tester.getBottomLeft(shortcut).dy, lessThanOrEqualTo(340));
    expect(tester.getSize(find.byType(AndroidView)).height, greaterThan(0));
    await tester.tap(find.byKey(const ValueKey('desktop-shell-mode-button')));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await tester.ensureVisible(
      find.byKey(const ValueKey('desktop-shell-mode-line')),
    );
    await tester.tap(find.byKey(const ValueKey('desktop-shell-mode-line')));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(tester.getBottomLeft(shortcut).dy, lessThanOrEqualTo(340));
    expect(
      find.byKey(const ValueKey('desktop-shell-line-input')),
      findsOneWidget,
    );
    await tester.pumpWidget(const SizedBox());
    manager.closeAll(terminate: false, notify: false);
    manager.dispose();
    keyboard.dispose();
    await tester.pump();
  });

  for (final width in [360.0, 1000.0]) {
    testWidgets(
      'Android uses common terminal tabs, splits and input modes at ${width.toInt()}px',
      (tester) async {
        final backend = fixture.FakeBackend(shellService: true);
        await fixture.mount(tester, backend, size: Size(width, 844));
        await tester.tap(find.text('shell'));
        await tester.pumpAndSettle();
        expect(find.byType(DesktopTerminalSheet), findsOneWidget);
        expect(find.byType(AndroidView), findsOneWidget);
        expect(find.byTooltip('Open in system terminal'), findsNothing);
        await tester.tap(
          find.byKey(const ValueKey('desktop-shell-split-horizontal')),
        );
        await tester.pumpAndSettle();
        expect(find.byType(AndroidView), findsNWidgets(2));
        await tester.tap(find.byKey(const ValueKey('desktop-shell-new-tab')));
        await tester.pumpAndSettle();
        expect(
          backend.calls.where((call) => call.$1 == 'nativeTerminalOpen'),
          hasLength(3),
        );
        expect(backend.calls.where((call) => call.$1 == 'shellOpen'), isEmpty);
        await tester.tap(
          find.byKey(const ValueKey('desktop-shell-mode-button')),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('desktop-shell-mode-line')));
        await tester.pumpAndSettle();
        await tester.enterText(
          find.byKey(const ValueKey('desktop-shell-line-input')),
          'echo café 中文',
        );
        await tester.tap(find.widgetWithText(FilledButton, 'Send'));
        await tester.pump();
        expect(
          backend.calls
              .where((call) => call.$1 == 'nativeTerminalInput')
              .single
              .$2,
          {
            'host': fixture.peer,
            'host_name': 'j-strix',
            'service': 'shell',
            'incarnation': fixture.incarnation,
            'session': 'tab-3',
            'data': 'echo café 中文\r',
          },
        );
        expect(tester.takeException(), isNull);
        await tester.tap(find.byKey(const ValueKey('desktop-shell-close')));
        await tester.pumpAndSettle();
        expect(
          backend.calls.where((call) => call.$1 == 'nativeTerminalClose'),
          isEmpty,
        );
        expect(find.byKey(const ValueKey('shell-dock')), findsOneWidget);
        final layout =
            backend.calls
                    .where((call) => call.$1 == 'saveTerminalLayout')
                    .last
                    .$2['layout']
                as Map;
        expect(layout['mode'], 'line');
        expect((layout['workspaces'] as List).single['tabs'], hasLength(2));
        await fixture.close(tester);
        expect(
          backend.calls.where((call) => call.$1 == 'nativeTerminalClose'),
          isEmpty,
        );
      },
    );
  }

  testWidgets(
    'Android terminal layout restores tabs, split direction, selection and mode after UI recreation',
    (tester) async {
      final backend = NativeTerminalBackend();
      final manager = DesktopShellManager(backend)..android = true;
      final workspace = manager.openHost(
        host: fixture.peer,
        hostName: 'Computer',
        service: 'shell',
        incarnation: fixture.incarnation,
      );
      workspace.splitActivePane(Axis.vertical);
      workspace.addTab();
      manager.inputMode = DesktopInputMode.line;
      workspace.minimized = true;
      manager.persistLayout();
      await tester.pump();
      final saved = manager.layoutJson();
      manager.closeAll(terminate: false, notify: false);
      manager.dispose();
      expect(
        backend.calls.where((call) => call.$1 == 'nativeTerminalClose'),
        isEmpty,
      );

      final restored = DesktopShellManager(backend)..android = true;
      restored.restoreLayout(saved);
      await tester.pump();
      expect(restored.inputMode, DesktopInputMode.line);
      expect(restored.minimizedSnapshots.single['session'], 'tab-3');
      final active = restored.openHost(
        host: fixture.peer,
        hostName: 'Computer',
        service: 'shell',
        incarnation: fixture.incarnation,
        session: 'tab-3',
      );
      expect(active.tabs, hasLength(2));
      expect(active.tabs.first.splitAxis, Axis.vertical);
      expect(active.tabs.first.panes.map((pane) => pane.session), [
        'main',
        'pane-2',
      ]);
      expect(active.activePane.session, 'tab-3');
      active.addTab();
      expect(active.activePane.session, 'tab-4');
      restored.closeAll(terminate: true);
      restored.dispose();
      await tester.pump();
      expect(
        backend.calls.where((call) => call.$1 == 'nativeTerminalClose'),
        hasLength(4),
      );
    },
  );

  testWidgets(
    'closing an Android pane while open is pending retains termination intent',
    (tester) async {
      final backend = NativeTerminalBackend()..opening = Completer();
      final pane = DesktopShellPane(
        backend: backend,
        host: fixture.peer,
        hostName: 'Computer',
        service: 'shell',
        incarnation: fixture.incarnation,
        session: 'main',
        android: true,
      );
      final connecting = pane.connect();
      pane.close(terminate: true);
      backend.opening!.complete({'state': 'connecting'});
      await connecting;
      expect(backend.calls.map((call) => call.$1), [
        'nativeTerminalOpen',
        'nativeTerminalClose',
      ]);
      expect(backend.calls.last.$2['terminate'], true);
      expect(backend.calls.last.$2['session'], 'main');
    },
  );
}
