import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/header.dart';
import 'package:wispnet/src/theme.dart';

final peer = 'a' * 64;
final incarnation = 'c' * 64;

class FakeBackend implements WispnetBackend {
  FakeBackend({
    this.paired = true,
    this.enabled = true,
    this.state = 'available',
  });
  bool paired, enabled;
  final String state;
  final calls = <(String, Map<String, dynamic>)>[];
  bool failPairing = false;
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, arguments));
    switch (method) {
      case 'snapshot':
        return {
          'name': 'HiBreak',
          'peer': 'd' * 64,
          'enabled': enabled,
          'relay': 'connected',
          'network': 'Network available',
          'version': '0.1.0',
          'order': {},
          'hosts': {
            if (paired)
              peer: {
                'name': 'j-strix',
                'state': state,
                'status': state == 'available'
                    ? 'Available'
                    : 'Access denied by this host',
                'catalog': {
                  'services': [
                    {
                      'id': 'website',
                      'display_name': 'Website',
                      'kind': 'http',
                      'incarnation': incarnation,
                    },
                    {
                      'id': 'llama',
                      'display_name': 'Local model',
                      'kind': 'http',
                      'incarnation': incarnation,
                    },
                  ],
                },
              },
          },
          'telemetry': {
            'peers': {
              peer: {'path': 'direct', 'rtt_ms': 15},
            },
          },
        };
      case 'setAccess':
        enabled = arguments['enabled'] as bool;
        return {};
      case 'open':
        return {'url': 'http://website.wispnet.localhost:18300'};
      case 'settings':
        return {
          'port': 18400,
          'url': 'http://registry.wispnet.localhost:18400',
        };
      case 'pair':
        if (failPairing) {
          throw PlatformException(
            code: 'invitation_denied',
            message: 'Invalid invitation',
          );
        }
        paired = true;
        return {};
      case 'removeHost':
        paired = false;
        return {};
      default:
        return {};
    }
  }
}

class DesktopFake extends FakeBackend {
  final ownerCalls = <Map<String, dynamic>>[];
  final status = <String, dynamic>{
    'host': {
      'host': 'd' * 64,
      'revision': 7,
      'services': [
        {
          'id': 'web',
          'display_name': 'Shared web',
          'kind': 'http',
          'backend': '127.0.0.1:8080',
          'incarnation': incarnation,
          'allow': [peer],
        },
      ],
      'clients': [
        {'peer': peer, 'name': 'Existing device'},
      ],
    },
    'host_name': 'Desktop',
    'connected_peers': {},
    'invitations': [
      {
        'name': 'New device',
        'peer': peer,
        'invitation': 'request-id',
        'state': 'pending',
      },
    ],
    'public_access': {'services': []},
  };
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    if (method == 'desktopStatus') return status;
    if (method == 'desktopData') {
      return {
        'value': arguments['op'] == 'targets'
            ? [
                {'host': 'd' * 64},
              ]
            : [],
      };
    }
    if (method == 'desktopRequest') {
      ownerCalls.add(arguments['payload'] as Map<String, dynamic>);
      return {};
    }
    final reply = await super.invoke(method, arguments);
    if (method == 'snapshot') {
      return {
        ...reply,
        'desktop': true,
        'pendingRequests': 1,
        'hosts': {
          ...reply['hosts'] as Map<String, dynamic>,
          'd' * 64: {
            'name': 'HiBreak',
            'local': true,
            'state': 'available',
            'status': 'This machine',
            'catalog': {'services': status['host']['services']},
          },
        },
      };
    }
    return reply;
  }
}

Future<void> mount(
  WidgetTester tester,
  FakeBackend backend, {
  Size size = const Size(390, 844),
  double scale = 1,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = size;
  tester.platformDispatcher.accessibilityFeaturesTestValue =
      const FakeAccessibilityFeatures(disableAnimations: true);
  addTearDown(() {
    tester.view.resetPhysicalSize();
    tester.view.resetDevicePixelRatio();
    tester.platformDispatcher.clearAccessibilityFeaturesTestValue();
  });
  await tester.pumpWidget(
    RepaintBoundary(
      key: const ValueKey('capture'),
      child: MediaQuery(
        data: MediaQueryData(
          size: size,
          textScaler: TextScaler.linear(scale),
          disableAnimations: true,
        ),
        child: WispnetApp(backend: backend),
      ),
    ),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

Future<void> close(WidgetTester tester) async {
  await tester.pumpWidget(const SizedBox());
  await tester.pump();
}

Future<void> screenshot(WidgetTester tester, String name) async {
  final folder = Platform.environment['WISPNET_SCREENSHOTS'];
  if (folder == null) return;
  await tester.runAsync(() async {
    final boundary = tester.firstRenderObject<RenderObject>(
      find.byKey(const ValueKey('capture')),
    ) as RenderRepaintBoundary;
    final image = await boundary.toImage();
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    final directory = Directory(folder)..createSync(recursive: true);
    File('${directory.path}/$name.png')
        .writeAsBytesSync(bytes!.buffer.asUint8List());
    image.dispose();
  });
}

void main() {
  setUpAll(() async {
    final loader = FontLoader('Roboto');
    loader.addFont(rootBundle.load('assets/fonts/Roboto-Regular.ttf'));
    loader.addFont(rootBundle.load('assets/fonts/Roboto-Medium.ttf'));
    loader.addFont(rootBundle.load('assets/fonts/Roboto-Bold.ttf'));
    await loader.load();
    await (FontLoader(
      'MaterialIcons',
    )..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
  });
  testWidgets(
    'existing machines and service handoff keep host and incarnation',
    (tester) async {
      final backend = FakeBackend();
      await mount(tester, backend);
      expect(find.text('REMOTE SECTOR'), findsOneWidget);
      expect(find.text('ONLINE · DIRECT · 15ms'), findsOneWidget);
      await screenshot(tester, 'android-directory');
      await tester.tap(find.textContaining('Website', findRichText: true));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 1000));
      final opened = backend.calls.where((c) => c.$1 == 'open').single.$2;
      expect(opened, {
        'host': peer,
        'service': 'website',
        'incarnation': incarnation,
      });
      expect(
        backend.calls.where((c) => c.$1 == 'launch').single.$2['url'],
        contains('website.wispnet.localhost'),
      );
      await close(tester);
    },
  );
  testWidgets('settings follows the device label in the directory', (
    tester,
  ) async {
    for (final width in [320.0, 390.0, 1440.0]) {
      await mount(tester, FakeBackend(), size: Size(width, 844));
      final label = tester.getRect(
        find.byKey(const ValueKey('device-heading')),
      );
      final settings = tester.getRect(find.byTooltip('Settings'));
      expect(settings.left, closeTo(label.right, .01));
      expect(settings.width, 44);
      final button = tester.getRect(find.byType(OnlineButton));
      expect(button.height, 44);
      expect(button.center.dy, closeTo(settings.center.dy + 2, .01));
      await tester.tap(find.byTooltip('Online'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byType(OnlineButton), findsNothing);
      expect(tester.takeException(), isNull);
      await close(tester);
    }
  });
  testWidgets('Android and desktop use the same directory and status labels', (
    tester,
  ) async {
    for (final backend in [FakeBackend(), DesktopFake()]) {
      await mount(tester, backend, size: const Size(1000, 850));
      expect(find.text('REMOTE SECTOR'), findsOneWidget);
      expect(find.text('PAIRED MACHINES'), findsNothing);
      expect(find.byTooltip('Online'), findsOneWidget);
      expect(find.byTooltip('Go offline'), findsNothing);
      await tester.tap(find.byTooltip('Online'));
      await tester.pumpAndSettle();
      expect(backend.enabled, false);
      expect(find.byTooltip('Enter wisp space'), findsOneWidget);
      await close(tester);
    }
  });
  testWidgets('both launch states show a centered entry and its caption', (
    tester,
  ) async {
    for (final paired in [false, true]) {
      final backend = FakeBackend(paired: paired, enabled: false);
      await mount(tester, backend);
      final caption = find.text(
        paired ? 'hyperjump to wispnet' : 'establish your wispnet',
      );
      expect(caption, findsOneWidget);
      expect(find.byType(Text), findsOneWidget);
      expect(find.byTooltip('Settings'), findsNothing);
      expect(find.text('Manage'), findsNothing);
      final entry = find.byTooltip(
        paired ? 'Enter wisp space' : 'Establish your wisp network',
      );
      expect(tester.getCenter(entry).dx, 195);
      final group = tester
          .getRect(entry)
          .expandToInclude(tester.getRect(caption));
      expect(group.center.dy, closeTo(422, .01));
      await tester.tap(entry);
      await tester.pumpAndSettle();
      expect(
        paired ? find.text('REMOTE SECTOR') : find.text('Pair a machine'),
        findsOneWidget,
      );
      await close(tester);
    }
  });
  testWidgets('Online status caption uses natural width at phone density', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1.875;
    tester.view.physicalSize = const Size(1072, 1448);
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });
    await tester.pumpWidget(
      MaterialApp(
        home: Material(
          child: Center(child: OnlineButton(onPressed: () {})),
        ),
      ),
    );
    await tester.pump();
    final bounds = tester.getSize(find.byType(OnlineButton));
    final caption = OnlineButton.labelPainter();
    expect(caption.text!.toPlainText(), 'Online');
    expect(
      (bounds.width * 1.875).round(),
      ((caption.width + 48) * 1.875).ceil(),
    );
    caption.dispose();
    expect(find.byTooltip('Online'), findsOneWidget);
    expect((bounds.height * 1.875).round(), 83);
    final folder = Platform.environment['WISPNET_SCREENSHOTS'];
    if (folder != null) {
      await tester.runAsync(() async {
        final boundary = tester.renderObject<RenderRepaintBoundary>(
          find.byKey(const ValueKey('offline-control')),
        );
        final image = await boundary.toImage(pixelRatio: 1.875);
        final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
        Directory(folder).createSync(recursive: true);
        File('$folder/flutter-offline-toggle.png')
            .writeAsBytesSync(bytes!.buffer.asUint8List());
        image.dispose();
      });
    }
    await close(tester);
  });
  testWidgets('denied cached services never issue an open request', (
    tester,
  ) async {
    final backend = FakeBackend(state: 'host_denied');
    await mount(tester, backend);
    await tester.tap(find.textContaining('Website', findRichText: true));
    await tester.pump();
    expect(backend.calls.where((c) => c.$1 == 'open'), isEmpty);
    await close(tester);
  });
  testWidgets(
    'pairing disables blank input and retains failed code and feedback',
    (tester) async {
      final backend = FakeBackend(paired: false, enabled: false)
        ..failPairing = true;
      await mount(tester, backend);
      await screenshot(tester, 'android-standby');
      await tester.tap(find.byTooltip('Establish your wisp network'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Pairing Code'));
      await tester.pumpAndSettle();
      final pair = find.widgetWithText(FilledButton, 'Pair');
      expect(tester.widget<FilledButton>(pair).onPressed, isNull);
      await tester.enterText(find.byType(TextField), '   \n ');
      await tester.pump();
      expect(tester.widget<FilledButton>(pair).onPressed, isNull);
      await tester.enterText(find.byType(TextField), 'wispnet1:sample\ncode');
      await tester.pump();
      await tester.tap(pair);
      await tester.pumpAndSettle();
      expect(find.textContaining('This host did not accept'), findsOneWidget);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        'wispnet1:sample\ncode',
      );
      await screenshot(tester, 'android-pairing');
      await close(tester);
    },
  );
  testWidgets('dragging a service saves its order within its machine', (
    tester,
  ) async {
    final backend = FakeBackend();
    await mount(tester, backend);
    final gesture = await tester.startGesture(
      tester.getCenter(find.text('Website')),
    );
    await tester.pump(const Duration(milliseconds: 650));
    await gesture.moveTo(tester.getCenter(find.text('Local model')));
    await tester.pump();
    await gesture.up();
    await tester.pump();
    final saved = backend.calls.where((c) => c.$1 == 'saveOrder').single.$2;
    expect(saved, {
      'group': 'services/$peer',
      'ids': ['llama', 'website'],
    });
    await close(tester);
  });
  testWidgets('settings validate before changing native state', (tester) async {
    final backend = FakeBackend();
    await mount(tester, backend);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'Registry port'),
      '80',
    );
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pump();
    expect(find.text('Port must be between 1024 and 65535.'), findsOneWidget);
    expect(backend.calls.where((c) => c.$1 == 'saveSettings'), isEmpty);
    await tester.enterText(
      find.widgetWithText(TextField, 'Registry port'),
      '18401',
    );
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();
    expect(
      backend.calls.where((c) => c.$1 == 'saveSettings').single.$2['port'],
      18401,
    );
    await close(tester);
  });
  for (final size in [const Size(320, 640), const Size(1440, 900)]) {
    testWidgets('shared components fit $size', (tester) async {
      await mount(tester, FakeBackend(), size: size);
      expect(tester.takeException(), isNull);
      if (size.width > 1000) await screenshot(tester, 'desktop-components');
      await close(tester);
    });
  }
  testWidgets('phone layout supports large text and a settings sheet', (
    tester,
  ) async {
    await mount(tester, FakeBackend(), size: const Size(360, 780), scale: 2);
    expect(tester.takeException(), isNull);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets('desktop owner tools use sheets and retain service grants', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    expect(find.text('Device and browser settings'), findsOneWidget);
    await tester.tap(find.text('Edit Shared web'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'Display name'),
      'Renamed web',
    );
    await tester.ensureVisible(find.text('Save service'));
    await tester.tap(find.text('Save service'));
    await tester.pumpAndSettle();
    final operation = backend.ownerCalls.single['operation'] as Map;
    expect(operation['expected_revision'], 7);
    expect(operation['allow'], [peer]);
    expect(operation['display_name'], 'Renamed web');
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets(
    'desktop sharing defaults an empty display name to the service ID',
    (tester) async {
      final backend = DesktopFake();
      await mount(tester, backend, size: const Size(1000, 850));
      await tester.tap(find.byTooltip('Settings'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Share a service'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.widgetWithText(TextField, 'Service ID'),
        'new-service',
      );
      await tester.ensureVisible(find.text('Save service'));
      await tester.tap(find.text('Save service'));
      await tester.pumpAndSettle();
      expect(
        backend.ownerCalls.single['operation']['display_name'],
        'new-service',
      );
      expect(backend.ownerCalls.single['operation']['allow'], isEmpty);
      expect(tester.takeException(), isNull);
      await close(tester);
    },
  );
  testWidgets('local services and remote machines have separate controls', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    final local = find.byKey(const ValueKey('local-sector'));
    final remote = find.byKey(const ValueKey('remote-sector'));
    expect(
      find.descendant(of: local, matching: find.text('LOCAL SERVICES')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: local, matching: find.textContaining('HiBreak')),
      findsNothing,
    );
    expect(
      find.descendant(of: local, matching: find.textContaining('ONLINE')),
      findsNothing,
    );
    expect(
      find.descendant(of: local, matching: find.byType(WispStatusDot)),
      findsNothing,
    );
    expect(
      find.descendant(of: local, matching: find.text('Shared web')),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: remote,
        matching: find.byTooltip('Refresh remote services'),
      ),
      findsOneWidget,
    );
    expect(
      backend.calls
          .where((c) => c.$1 == 'refreshHost')
          .every((c) => c.$2['host'] == peer),
      true,
    );
    await tester.tap(find.descendant(of: local, matching: find.text('Manage')));
    await tester.pumpAndSettle();
    expect(find.text('Share a service'), findsOneWidget);
    expect(find.text('Device and browser settings'), findsNothing);
    expect(find.text('Pair with this machine'), findsNothing);
    expect(find.text('Edit Shared web'), findsOneWidget);
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
    await tester.tap(
      find.descendant(of: remote, matching: find.byType(OutlinedButton)).first,
    );
    await tester.pumpAndSettle();
    expect(find.text('Paired machines'), findsOneWidget);
    expect(find.text('HiBreak · dddddddd'), findsOneWidget);
    expect(find.text('j-strix · aaaaaaaa'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets('desktop public access requires explicit confirmation', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('Enable public access'));
    await tester.tap(find.text('Enable public access'));
    await tester.pumpAndSettle();
    expect(backend.ownerCalls, isEmpty);
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    expect(backend.ownerCalls.single['op'], 'configure_public');
    expect(backend.ownerCalls.single['incarnation'], incarnation);
    expect(backend.ownerCalls.single['enabled'], true);
    await close(tester);
  });
  testWidgets('desktop pairing approval preserves existing service grants', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(DropdownButton<String>).first);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Pairing requests (1)').last);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Approve device'));
    await tester.pumpAndSettle();
    final grant = tester.widget<CheckboxListTile>(
      find.byType(CheckboxListTile),
    );
    expect(grant.value, true);
    expect(grant.onChanged, isNull);
    await tester.tap(find.text('Approve device').last);
    await tester.pumpAndSettle();
    expect(backend.ownerCalls.single['operation'], {
      'op': 'approve_invitation',
      'invitation': 'request-id',
      'services': ['web'],
      'expected_revision': 7,
    });
    await close(tester);
  });
  test('saved order ignores missing and duplicate entries', () {
    expect(orderedIds(['a', 'b', 'c'], ['b', 'gone', 'b']), ['b', 'a', 'c']);
  });
}
