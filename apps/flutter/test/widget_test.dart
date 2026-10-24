import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/cards.dart';
import 'package:wispnet/src/header.dart';
import 'package:wispnet/src/desktop_settings.dart';
import 'package:wispnet/src/theme.dart';

final peer = 'a' * 64;
final incarnation = 'c' * 64;

class FakeBackend implements WispnetBackend {
  FakeBackend({
    this.paired = true,
    this.enabled = true,
    this.state = 'available',
    this.diagnosticReport,
    this.publicAccess,
  });
  final Map<String, dynamic>? diagnosticReport;
  final Map<String, dynamic>? publicAccess;
  bool paired, enabled;
  final String state;
  final calls = <(String, Map<String, dynamic>)>[];
  bool failPairing = false;
  bool missingRoute = false, failOpen = false;
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, arguments));
    switch (method) {
      case 'snapshot':
        return {
          'name': 'hibreak',
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
                if (publicAccess != null) 'public_access': publicAccess,
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
      case 'connectionDiagnostics':
        return {
          'network': 'Cellular',
          'vpn': false,
          'peers': [
            {
              'name': 'j-strix',
              'report':
                  diagnosticReport ??
                  {
                    'initial_path': 'relay',
                    'selected_path': 'relay',
                    'rtt_ms': 42,
                    'observation_ms': 10000,
                    'relay_status': 'connected',
                    'reason':
                        'No direct path was established within 10 seconds.',
                    'network_report': null,
                    'transport': {
                      'relays': [
                        {
                          'url': 'https://relay.wispnet.app/',
                          'connected': true,
                          'preferred': true,
                        },
                        {
                          'url': 'https://usw1-1.relay.n0.iroh.link/',
                          'connected': true,
                          'preferred': false,
                        },
                      ],
                      'discovery': [
                        {
                          'url': 'https://relay.wispnet.app/',
                          'ipv4': true,
                          'ipv6': false,
                        },
                        {
                          'url': 'https://usw1-1.relay.n0.iroh.link/',
                          'ipv4': true,
                          'ipv6': false,
                        },
                      ],
                    },
                  },
            },
          ],
        };
      case 'setAccess':
        enabled = arguments['enabled'] as bool;
        return {};
      case 'open':
        if (failOpen) {
          throw PlatformException(
            code: 'unavailable',
            message: 'Host unavailable',
          );
        }
        return {'url': 'http://website.wispnet.localhost:18300'};
      case 'settings':
        return {
          'port': 18400,
          'url': arguments['host'] == null
              ? 'http://registry.wispnet.localhost:18400'
              : missingRoute
              ? ''
              : 'http://j-strix-aaaaaaaaaaaa-${arguments['service']}.localhost:18400/',
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

class LinkFake extends FakeBackend {
  LinkFake(this.link) : super(enabled: false);
  String? link;
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    if (method == 'takeLink') {
      final value = link;
      link = null;
      return value == null ? {} : {'link': value};
    }
    if (method == 'resolveLink') {
      calls.add((method, arguments));
      return {'url': 'http://j-strix-xxx-website.localhost:7842/'};
    }
    return super.invoke(method, arguments);
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
    if (method == 'settings' && arguments['host'] == 'd' * 64) {
      calls.add((method, arguments));
      return {'port': '', 'url': 'http://shared-web.localhost:18000/'};
    }
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
      if (arguments['payload']['op'] == 'invite_qr') {
        return {
          'host': 'd' * 64,
          'code': 'wispnet1:generated-code',
          'matrix': [
            [1, 0],
            [0, 1],
          ],
          'invitation': 'issued-id',
          'expires_at': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 60,
        };
      }
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
            'name': 'hibreak',
            'local': true,
            'state': 'available',
            'status': 'This machine',
            'catalog': {
              'services': [
                for (final service in status['host']['services'])
                  {
                    ...service as Map<String, dynamic>,
                    for (final p in status['public_access']['services'])
                      if (p['service'] == service['id'] &&
                          p['incarnation'] == service['incarnation'])
                        'public_access': p,
                  },
              ],
            },
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
  bool disableAnimations = true,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = size;
  tester.platformDispatcher.accessibilityFeaturesTestValue =
      FakeAccessibilityFeatures(disableAnimations: disableAnimations);
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
          disableAnimations: disableAnimations,
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
  testWidgets(
    'service links enable access and launch the resolved HTTP URL once',
    (tester) async {
      final backend = LinkFake('wispnet://$peer/website');
      await tester.pumpWidget(WispnetApp(backend: backend));
      for (var n = 0; n < 20; n++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(
        backend.calls.where((c) => c.$1 == 'resolveLink').single.$2['link'],
        'wispnet://$peer/website',
      );
      expect(
        backend.calls.where((c) => c.$1 == 'launch').single.$2['url'],
        'http://j-strix-xxx-website.localhost:7842/',
      );
      expect(backend.enabled, true);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'malformed service links never enable access or resolve a service',
    (tester) async {
      final backend = LinkFake('wispnet://$peer/website?backend=evil');
      await tester.pumpWidget(WispnetApp(backend: backend));
      for (var n = 0; n < 20; n++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(
        backend.calls.where(
          (c) => ['setAccess', 'resolveLink', 'launch'].contains(c.$1),
        ),
        isEmpty,
      );
      expect(backend.enabled, false);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets('empty remote card is compact and names the device once', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: SizedBox(
              width: 900,
              child: MachineCard(
                machine: Machine(peer, {
                  'name': 'Pixel 10 Pro',
                  'state': 'available',
                  'catalog': {'services': []},
                }),
                order: const {},
                telemetry: 'Direct · 8ms',
                stateColor: Colors.white,
                busy: false,
                onManage: () {},
                onOpen: (_) {},
                onLinks: (_) {},
                onSettings: (_) {},
                onReorder: (_, _, _, _) {},
              ),
            ),
          ),
        ),
      ),
    );
    expect(find.text('No shared services'), findsOneWidget);
    expect(find.textContaining('Pixel 10 Pro'), findsOneWidget);
    expect(find.textContaining('PIXEL 10 PRO'), findsNothing);
    expect(tester.getSize(find.byType(MachineCard)).height, lessThan(80));
  });

  testWidgets('mouse hover highlights custom controls and clears on exit', (
    tester,
  ) async {
    final backend = FakeBackend();
    String? copied;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = call.arguments['text'] as String;
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    await mount(tester, backend, size: const Size(1000, 850));
    final mouse = await tester.createGesture(kind: ui.PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    final controls = [
      find.descendant(
        of: find.byType(OnlineButton),
        matching: find.byType(WispInkWell),
      ),
      find.ancestor(
        of: find.text('website'),
        matching: find.byType(WispInkWell),
      ),
      find.ancestor(
        of: find.textContaining('j-strix'),
        matching: find.byType(WispInkWell),
      ),
    ];
    Color overlay(Finder control) =>
        (tester
                    .widget<AnimatedContainer>(
                      find
                          .descendant(
                            of: control,
                            matching: find.byType(AnimatedContainer),
                          )
                          .first,
                    )
                    .foregroundDecoration!
                as ShapeDecoration)
            .color!;
    for (final control in controls) {
      expect(overlay(control).a, 0);
      await mouse.moveTo(tester.getCenter(control));
      await tester.pumpAndSettle();
      expect(overlay(control).a, greaterThan(.1));
      await mouse.moveTo(Offset.zero);
      await tester.pumpAndSettle();
      expect(overlay(control).a, 0);
    }
    final heading = find.textContaining('j-strix');
    expect(
      tester.getSize(controls[2]).width,
      closeTo(tester.getSize(heading).width, .01),
    );
    final status = find.textContaining('Online · Direct');
    await mouse.moveTo(tester.getCenter(status));
    await tester.pumpAndSettle();
    expect(overlay(controls[2]).a, 0);
    final service = controls[1];
    expect(find.byTooltip('Settings for website on j-strix'), findsNothing);
    final copy = find.byTooltip('Show links for website');
    expect(tester.getRect(service).contains(tester.getCenter(copy)), isTrue);
    await mouse.moveTo(tester.getCenter(copy));
    await tester.pumpAndSettle();
    expect(overlay(service).a, greaterThan(.1));
    await mouse.down(tester.getCenter(copy));
    await mouse.up();
    await tester.pumpAndSettle();
    expect(find.byTooltip('Close'), findsOneWidget);
    expect(copied, isNull);
    expect(backend.calls.where((call) => call.$1 == 'settings'), hasLength(1));
    expect(backend.calls.where((call) => call.$1 == 'open'), isEmpty);
    await mouse.removePointer();
    await close(tester);
  });

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
      expect(find.text('Online · Direct · 15ms'), findsOneWidget);
      await screenshot(tester, 'android-directory');
      await tester.tap(find.textContaining('website', findRichText: true));
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
  testWidgets('entry reveals all controls only during the end of the jump', (
    tester,
  ) async {
    await mount(tester, FakeBackend(enabled: false), disableAnimations: false);
    await tester.tap(find.byTooltip('Enter wisp space'));
    await tester.pump();
    double opacity() => tester
        .widget<FadeTransition>(find.byKey(const ValueKey('entry-reveal')))
        .opacity
        .value;
    expect(opacity(), 0);
    await tester.pump(const Duration(milliseconds: 650));
    expect(opacity(), 0);
    await tester.pump(const Duration(milliseconds: 150));
    expect(opacity(), closeTo(.5, .01));
    await tester.pump(const Duration(milliseconds: 100));
    expect(opacity(), 1);
    expect(find.text('REMOTE SECTOR'), findsOneWidget);
    await close(tester);
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
  testWidgets('desktop pairing opens the code form without a tab selector', (
    tester,
  ) async {
    await mount(tester, DesktopFake(), size: const Size(1000, 850));
    await tester.tap(
      find
          .descendant(
            of: find.byKey(const ValueKey('remote-sector')),
            matching: find.byType(OutlinedButton),
          )
          .first,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('+ Pair another machine'));
    await tester.pumpAndSettle();
    expect(find.text('Pair a machine'), findsOneWidget);
    expect(find.text('Pairing Code'), findsNothing);
    expect(find.text('QR'), findsNothing);
    expect(find.byType(TextField), findsOneWidget);
    final pair = find.widgetWithText(FilledButton, 'Pair');
    expect(tester.widget<FilledButton>(pair).onPressed, isNull);
    await tester.enterText(find.byType(TextField), 'wispnet1:samplecode');
    await tester.pump();
    expect(tester.widget<FilledButton>(pair).onPressed, isNotNull);
    await close(tester);
  });

  testWidgets('denied cached services never issue an open request', (
    tester,
  ) async {
    final backend = FakeBackend(state: 'host_denied');
    await mount(tester, backend);
    await tester.tap(find.textContaining('website', findRichText: true));
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
      tester.getCenter(find.text('website')),
    );
    await tester.pump(const Duration(milliseconds: 650));
    await gesture.moveTo(tester.getCenter(find.text('llama')));
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
  testWidgets('Diagnostic below Save opens a page and runs automatically', (
    tester,
  ) async {
    final backend = FakeBackend();
    await mount(tester, backend);
    expect(find.text('Diagnostic'), findsNothing);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Connection diagnostics'), findsNothing);
    expect(
      tester.getTopLeft(find.text('Diagnostic')).dy,
      greaterThan(tester.getBottomLeft(find.text('Save')).dy),
    );
    await tester.tap(find.text('Diagnostic'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(AppBar, 'Diagnostic'), findsOneWidget);
    expect(find.textContaining('Network: Cellular'), findsOneWidget);
    expect(find.textContaining('relay → relay'), findsOneWidget);
    expect(find.textContaining('Phone relay connections'), findsOneWidget);
    expect(
      find.textContaining('✓ https://relay.wispnet.app/ · preferred'),
      findsOneWidget,
    );
    expect(
      find.textContaining('✓ https://usw1-1.relay.n0.iroh.link/'),
      findsOneWidget,
    );
    expect(find.textContaining('Phone discovery observations'), findsOneWidget);
    expect(find.textContaining('• ? UDP response IPv4'), findsOneWidget);
    expect(
      find.textContaining('Automatic router mapping status unavailable'),
      findsOneWidget,
    );
    expect(
      backend.calls
          .where((c) => c.$1 == 'connectionDiagnostics')
          .single
          .$2['host'],
      peer,
    );
    expect(backend.calls.where((c) => c.$1 == 'saveSettings'), isEmpty);
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.widgetWithText(TextField, 'Device ID'), findsOneWidget);
    expect(find.text('Save'), findsOneWidget);
  });
  testWidgets('both ends distinguish refreshed discovery from cached reports', (
    tester,
  ) async {
    Map<String, dynamic> side(bool fresh) => {
      'relay_status': 'connected',
      'elapsed_ms': 20000,
      'peer_ip_candidates': 2,
      'network': {
        'fresh': fresh,
        'wait_ms': 20000,
        'report': {
          'udp_v4': true,
          'udp_v6': false,
          'mapping_varies': null,
          'public_ipv4_discovered': fresh,
          'public_ipv6_discovered': false,
        },
      },
      'paths': {
        'initial_path': 'relay',
        'selected_path': 'relay',
        'rtt_ms': 100,
        'direct_paths_seen': 0,
        'direct_paths_closed': 0,
        'direct_datagrams_sent': 0,
        'direct_datagrams_received': 0,
        'direct_challenges_sent': 0,
        'direct_responses_received': 0,
        'connection_challenges_sent': 3,
        'connection_responses_received': 2,
        'events_lost': 0,
      },
    };
    final backend = FakeBackend(
      diagnosticReport: {
        'initial_path': 'relay',
        'selected_path': 'relay',
        'observation_ms': 20000,
        'relay_status': 'connected',
        'reason': 'UDP discovery works; direct peer connectivity failed during this test.',
        'local': side(true),
        'remote': side(false),
        'router_mapping': {
          'local': {
            'enabled': true,
            'active': true,
            'external_port': 54321,
            'public_address': true,
            'upnp': true,
            'pcp': false,
            'nat_pmp': null,
          },
          'remote': {
            'enabled': true,
            'active': false,
            'external_port': null,
            'public_address': null,
            'upnp': true,
            'pcp': false,
            'nat_pmp': null,
          },
        },
      },
    );
    await mount(tester, backend);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Diagnostic'));
    await tester.pumpAndSettle();
    final text = tester
        .widget<SelectableText>(find.byType(SelectableText))
        .data!;
    expect(text, contains('✓ Automatic router mapping enabled'));
    expect(text, contains('UPnP ✓ · PCP ✗ · NAT-PMP ?'));
    expect(text, contains('✓ Router mapping granted · UDP 54321'));
    expect(text, contains('✗ Router mapping granted'));
    expect(text, contains('A granted mapping does not prove reachability'));

    expect(text, contains('Phone observations'));
    expect(text, contains('Target computer observations'));
    expect(text, contains('network report updated during this test'));
    expect(text, contains('cached; no new report observed during this test'));
    expect(text, contains('across all paths (includes relay)'));
    expect(text, contains('Known peer IP addresses in transport cache: 2'));
    expect(text, contains('cache count does not measure address exchange'));
    expect(text, contains('Discovery responded, but no public IP was found'));
    expect(
      text,
      contains('Zero observed paths does not prove no attempts were made'),
    );
  });
  testWidgets('settings validate before changing native state', (tester) async {
    final backend = FakeBackend();
    await mount(tester, backend);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'Wispnet port'),
      '80',
    );
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pump();
    expect(find.text('Port must be between 1024 and 65535.'), findsOneWidget);
    expect(backend.calls.where((c) => c.$1 == 'saveSettings'), isEmpty);
    await tester.enterText(
      find.widgetWithText(TextField, 'Wispnet port'),
      '18401',
    );
    for (final invalid in ['has spaces', 'Uppercase', '-prefix', 'a' * 65]) {
      await tester.enterText(
        find.widgetWithText(TextField, 'Device ID'),
        invalid,
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Save'));
      await tester.pump();
      expect(find.textContaining('Use 1–64 lowercase'), findsOneWidget);
      expect(backend.calls.where((c) => c.$1 == 'saveSettings'), isEmpty);
    }
    await tester.enterText(
      find.widgetWithText(TextField, 'Device ID'),
      'my-phone',
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
  testWidgets('paired machines show one readable ID and their host identity', (
    tester,
  ) async {
    final backend = FakeBackend();
    await mount(tester, backend);
    await tester.tap(find.textContaining('j-strix'));
    await tester.pumpAndSettle();
    expect(find.text('Device ID\nj-strix\n\nHost ID\n$peer'), findsOneWidget);
    expect(find.widgetWithText(TextField, 'Display name'), findsNothing);
    expect(find.text('Save name'), findsNothing);
    expect(backend.calls.where((call) => call.$1 == 'renameHost'), isEmpty);
    await close(tester);
  });
  testWidgets('machine settings controls have consistent spacing', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.textContaining('j-strix'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(OutlinedButton, 'Refresh'), findsNothing);
    expect(find.byTooltip('Refresh remote services'), findsOneWidget);
    final remove = find.widgetWithText(OutlinedButton, 'Remove');
    expect(
      tester.widget<OutlinedButton>(remove).style!.foregroundColor!.resolve({}),
      WispColors.error,
    );
    await tester.tap(remove);
    await tester.pumpAndSettle();
    expect(find.text('Remove machine?'), findsOneWidget);
    expect(
      find.textContaining(
        'forget the pairing and close its local service routes',
      ),
      findsOneWidget,
    );
    expect(backend.calls.where((call) => call.$1 == 'removeHost'), isEmpty);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(backend.calls.where((call) => call.$1 == 'removeHost'), isEmpty);
    await tester.tap(remove);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Remove machine'));
    await tester.pumpAndSettle();
    expect(
      backend.calls.where((call) => call.$1 == 'removeHost').single.$2['host'],
      peer,
    );
    expect(find.widgetWithText(TextField, 'Display name'), findsNothing);
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets('desktop device gear opens name and registry settings directly', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(TextField, 'Device ID'), findsOneWidget);
    expect(find.widgetWithText(TextField, 'Wispnet port'), findsOneWidget);
    expect(find.text('Device and browser settings'), findsNothing);
    expect(find.text('Share a service'), findsNothing);
    expect(find.byType(DropdownButton<String>), findsNothing);
    expect(
      find.text('http://registry.wispnet.localhost:18400/v1/services'),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const ValueKey('settings-address')));
    await tester.pumpAndSettle();
    expect(
      backend.calls.where((call) => call.$1 == 'launch').single.$2['url'],
      'http://registry.wispnet.localhost:18400/v1/services',
    );
    expect(find.byTooltip('Close'), findsOneWidget);
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Close'), findsNothing);
    await close(tester);
  });
  testWidgets('new service accepts one explicit ID and saves a numeric port', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(
      find.descendant(
        of: find.byKey(const ValueKey('local-sector')),
        matching: find.widgetWithText(OutlinedButton, 'Manage'),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add a new service'));
    await tester.pumpAndSettle();
    final id = find.widgetWithText(TextField, 'Service ID');
    final port = find.widgetWithText(TextField, 'Port');
    expect(find.widgetWithText(TextField, 'Service name'), findsNothing);
    expect(tester.getRect(id).top, lessThan(tester.getRect(port).top));
    expect(tester.widget<TextField>(port).controller!.text, '8080');
    for (final invalid in ['', 'UpperCase', 'has spaces', 'a' * 65]) {
      await tester.enterText(id, invalid);
      await tester.ensureVisible(find.text('Save service'));
      await tester.tap(find.text('Save service'));
      await tester.pumpAndSettle();
      expect(find.text('Enter a valid service ID.'), findsOneWidget);
      expect(backend.ownerCalls, isEmpty);
    }
    await tester.enterText(id, 'local_model_api');
    for (final invalid in ['0', '65536']) {
      await tester.enterText(port, invalid);
      await tester.ensureVisible(find.text('Save service'));
      await tester.tap(find.text('Save service'));
      await tester.pumpAndSettle();
      expect(find.text('Enter a port between 1 and 65535.'), findsOneWidget);
      expect(backend.ownerCalls, isEmpty);
    }
    expect(
      tester
          .widget<DropdownButtonFormField<String>>(
            find.byType(DropdownButtonFormField<String>),
          )
          .decoration
          .labelText,
      'Protocol',
    );
    await tester.tap(find.byType(DropdownButtonFormField<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('TCP').last);
    await tester.pumpAndSettle();
    await tester.enterText(port, '8201');
    await tester.tap(find.text('Save service'));
    await tester.pumpAndSettle();
    final operation = backend.ownerCalls.single['operation'];
    expect(operation['id'], 'local_model_api');
    expect(operation.containsKey('display_name'), isFalse);
    expect(operation['backend'], '127.0.0.1:8201');
    expect(operation['kind'], 'tcp');
    await close(tester);
  });

  testWidgets('each local service gear edits only its own service', (
    tester,
  ) async {
    final backend = DesktopFake();
    (backend.status['host']['services'] as List).add({
      'id': 'other',
      'display_name': 'Other service',
      'kind': 'http',
      'backend': '[::1]:8090',
      'incarnation': incarnation,
      'allow': [],
    });
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.byTooltip('Settings for other on hibreak'));
    await tester.pumpAndSettle();
    expect(find.text('Edit'), findsOneWidget);
    expect(find.text('Browser settings'), findsNothing);
    expect(find.text('Add a new service'), findsNothing);
    await tester.tap(find.text('Edit'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<TextField>(find.widgetWithText(TextField, 'Port'))
          .controller!
          .text,
      '8090',
    );
    expect(
      tester
          .widget<TextField>(find.widgetWithText(TextField, 'Service ID'))
          .enabled,
      false,
    );
    await tester.ensureVisible(find.text('Save service'));
    await tester.tap(find.text('Save service'));
    await tester.pumpAndSettle();
    expect(backend.ownerCalls.single['operation']['id'], 'other');
    expect(backend.ownerCalls.single['operation']['backend'], '[::1]:8090');
    expect(backend.ownerCalls.single['operation']['allow'], isEmpty);
    await close(tester);
  });
  testWidgets('desktop owner tools use sheets and retain service grants', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.byTooltip('Settings for web on hibreak'));
    await tester.pumpAndSettle();
    expect(find.text('Device and browser settings'), findsNothing);
    expect(find.text('Add a new service'), findsNothing);
    expect(find.text('Pair this machine'), findsNothing);
    expect(find.text('Browser settings'), findsNothing);
    await tester.tap(find.text('Edit'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(TextField, 'Service name'), findsNothing);
    await tester.ensureVisible(find.text('Save service'));
    await tester.tap(find.text('Save service'));
    await tester.pumpAndSettle();
    final operation = backend.ownerCalls.single['operation'] as Map;
    expect(operation['expected_revision'], 7);
    expect(operation['id'], 'web');
    expect(operation['backend'], '127.0.0.1:8080');
    expect(operation['allow'], [peer]);
    expect(operation.containsKey('display_name'), isFalse);
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets('desktop sharing saves only the service ID', (tester) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(
      find
          .descendant(
            of: find.byKey(const ValueKey('local-sector')),
            matching: find.byType(OutlinedButton),
          )
          .first,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add a new service'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'Service ID'),
      'new-service',
    );
    await tester.ensureVisible(find.text('Save service'));
    await tester.tap(find.text('Save service'));
    await tester.pumpAndSettle();
    expect(backend.ownerCalls.single['operation']['id'], 'new-service');
    expect(backend.ownerCalls.single['operation']['allow'], isEmpty);
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets(
    'section Manage buttons follow their titles at phone and desktop widths',
    (tester) async {
      for (final width in [390.0, 1000.0]) {
        await mount(tester, DesktopFake(), size: Size(width, 850));
        for (final section in ['local', 'remote']) {
          final container = find.byKey(ValueKey('$section-sector'));
          final title = find.descendant(
            of: container,
            matching: find.text(
              section == 'local' ? 'LOCAL SERVICES' : 'REMOTE SECTOR',
            ),
          );
          final manage = find
              .descendant(of: container, matching: find.byType(OutlinedButton))
              .first;
          expect(
            tester.getRect(manage).left,
            closeTo(tester.getRect(title).right + 12, .01),
          );
        }
        final remote = find.byKey(const ValueKey('remote-sector'));
        final refresh = find.ancestor(
          of: find.byTooltip('Refresh remote services'),
          matching: find.byType(IconButton),
        );
        expect(
          tester.getRect(refresh).right,
          closeTo(tester.getRect(remote).right, .01),
        );
        expect(
          tester.widget<Text>(find.text('REMOTE SECTOR')).style!.fontSize,
          16,
        );
        expect(tester.takeException(), isNull);
        await close(tester);
      }
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
      find.descendant(of: local, matching: find.textContaining('hibreak')),
      findsNothing,
    );
    expect(
      find.descendant(of: local, matching: find.textContaining('Online')),
      findsNothing,
    );
    expect(
      find.descendant(of: local, matching: find.byType(WispStatusDot)),
      findsNothing,
    );
    expect(
      find.descendant(of: local, matching: find.text('web')),
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
    expect(find.text('Add a new service'), findsOneWidget);
    expect(find.text('Pair this machine'), findsOneWidget);
    expect(find.text('Device and browser settings'), findsNothing);
    expect(find.text('Pair with this machine'), findsNothing);
    expect(find.text('Edit'), findsNothing);
    expect(find.text('Remove service'), findsNothing);
    expect(find.text('Enable public access'), findsNothing);
    await tester.tap(find.text('Pair this machine'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('pairing-qr')), findsOneWidget);
    expect(find.text('Copy pairing code'), findsNothing);
    await tester.tap(find.text('Pairing code'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('pairing-code')), findsOneWidget);
    expect(find.text('wispnet1:generated-code'), findsOneWidget);
    expect(
      backend.ownerCalls.where((call) => call['op'] == 'invite_qr'),
      hasLength(1),
    );
    expect(find.text('Copy pairing code'), findsOneWidget);
    await tester.tap(find.text('QR code'));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('pairing-qr')), findsOneWidget);
    expect(find.text('Copy pairing code'), findsNothing);
    await tester.tap(find.byTooltip('Close').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
    await tester.tap(
      find.descendant(of: remote, matching: find.byType(OutlinedButton)).first,
    );
    await tester.pumpAndSettle();
    expect(find.text('Paired machines'), findsOneWidget);
    expect(find.text('hibreak · dddddddd'), findsOneWidget);
    expect(find.text('j-strix · aaaaaaaa'), findsNWidgets(2));
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets(
    'public service links, status and copy action reflect publication state',
    (tester) async {
      final backend = DesktopFake();
      final publication = {
        'service': 'web',
        'incarnation': incarnation,
        'enabled': true,
        'state': 'online',
        'url': 'https://shared-web.wispnet.test/',
      };
      backend.status['public_access']['services'] = [publication];
      String? copied;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'Clipboard.setData') {
            copied = call.arguments['text'] as String;
          }
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      await mount(tester, backend, size: const Size(1000, 850));
      expect(find.text('Public · Online'), findsOneWidget);
      expect(find.text('Public URL'), findsNothing);
      expect(find.byTooltip('Copy public URL for web'), findsNothing);
      final links = find.byTooltip('Show links for web');
      final gear = find.byTooltip('Settings for web on hibreak');
      expect(
        tester.getRect(links).right,
        lessThanOrEqualTo(tester.getRect(gear).left),
      );
      await tester.tap(links);
      await tester.pumpAndSettle();
      expect(copied, isNull);
      await tester.tap(find.byTooltip('Copy public URL'));
      await tester.pumpAndSettle();
      expect(copied, publication['url']);
      expect(backend.calls.where((call) => call.$1 == 'open'), isEmpty);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      publication['state'] = 'offline';
      await tester.pump(const Duration(seconds: 2));
      await tester.pumpAndSettle();
      expect(find.text('Public · Offline'), findsOneWidget);
      await tester.tap(gear);
      await tester.pumpAndSettle();
      final local = find.text('http://shared-web.localhost:18000/');
      final public = find.text(publication['url'] as String);
      expect(tester.getRect(local).top, lessThan(tester.getRect(public).top));
      expect(
        tester.getRect(public).bottom,
        lessThan(tester.getRect(find.text('Edit')).top),
      );
      expect(find.textContaining('127.0.0.1:8080'), findsNothing);
      expect(
        find.descendant(
          of: find.byType(DesktopSettings),
          matching: find.textContaining('Public ·'),
        ),
        findsNothing,
      );
      await tester.tap(local);
      await tester.pumpAndSettle();
      expect(
        backend.calls.where((call) => call.$1 == 'launch').single.$2['url'],
        'http://shared-web.localhost:18000/',
      );
      await tester.tap(public);
      await tester.pumpAndSettle();
      expect(
        backend.calls
            .where((call) => call.$1 == 'launchPublic')
            .single
            .$2['service'],
        'web',
      );
      expect(backend.ownerCalls, isEmpty);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      backend.status['public_access']['services'] = [];
      await tester.pump(const Duration(seconds: 2));
      await tester.pumpAndSettle();
      expect(find.text('Public · Online'), findsNothing);
      expect(find.text('Public · Offline'), findsNothing);
      expect(find.byTooltip('Copy public URL for web'), findsNothing);
      await close(tester);
    },
  );
  testWidgets('public status and copy controls fit a narrow service tile', (
    tester,
  ) async {
    final backend = DesktopFake();
    backend.status['public_access']['services'] = [
      {
        'service': 'web',
        'incarnation': incarnation,
        'enabled': true,
        'state': 'online',
        'url': 'https://shared-web.wispnet.test/',
      },
    ];
    await mount(tester, backend, size: const Size(360, 844));
    expect(find.text('Public · Online'), findsOneWidget);
    expect(find.byTooltip('Copy public URL for web'), findsNothing);
    expect(find.byTooltip('Show links for web'), findsOneWidget);
    expect(tester.takeException(), isNull);
    final card = tester.widget<MachineCard>(find.byType(MachineCard).first);
    await tester.pumpWidget(
      MaterialApp(
        theme: wispTheme(),
        home: Scaffold(
          body: MediaQuery(
            data: const MediaQueryData(textScaler: TextScaler.linear(2)),
            child: SizedBox(width: 280, child: card),
          ),
        ),
      ),
    );
    await tester.pump();
    expect(find.text('Public URL'), findsNothing);
    expect(find.byTooltip('Show links for web'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await close(tester);
  });
  testWidgets(
    'phone public links use matching host metadata and copy without opening a service',
    (tester) async {
      final publication = {
        'service': 'website',
        'incarnation': incarnation,
        'url': 'https://shared-website.wispnet.test/',
        'state': 'online',
      };
      final backend = FakeBackend(
        publicAccess: {
          'version': 1,
          'host': peer,
          'services': [publication],
        },
      );
      String? copied;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'Clipboard.setData') {
            copied = call.arguments['text'] as String;
          }
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      await mount(tester, backend, size: const Size(360, 844));
      expect(find.text('Public URL'), findsNothing);
      expect(find.text('Public · Online'), findsOneWidget);
      expect(find.byTooltip('Copy public URL for website'), findsNothing);
      final links = find.byTooltip('Show links for website');
      expect(
        find.descendant(of: links, matching: find.byType(IconButton)),
        findsOneWidget,
      );
      expect(
        find.descendant(of: links, matching: find.byIcon(Icons.link)),
        findsOneWidget,
      );
      await tester.tap(links);
      await tester.pumpAndSettle();
      expect(copied, isNull);
      expect(find.text('Links for website'), findsOneWidget);
      expect(find.text('Local URL'), findsOneWidget);
      expect(find.text('Wispnet URL'), findsOneWidget);
      expect(
        tester.getRect(find.text('Local URL')).top,
        lessThan(tester.getRect(find.text('Wispnet URL')).top),
      );
      expect(
        tester.getRect(find.text('Wispnet URL')).top,
        lessThan(tester.getRect(find.text('Public URL').last).top),
      );
      await tester.tap(find.byTooltip('Copy local URL'));
      await tester.pumpAndSettle();
      expect(copied, 'http://j-strix-aaaaaaaaaaaa-website.localhost:18400/');
      await tester.tap(find.byTooltip('Copy Wispnet link'));
      await tester.pumpAndSettle();
      expect(copied, 'wispnet://$peer/website');
      await tester.tap(find.byTooltip('Copy public URL'));
      await tester.pumpAndSettle();
      expect(copied, publication['url']);
      expect(
        backend.calls.where((c) => ['open', 'saveSettings'].contains(c.$1)),
        isEmpty,
      );
      expect(backend.calls.where((c) => c.$1 == 'launch'), isEmpty);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      backend.publicAccess!['host'] = 'e' * 64;
      await tester.pump(const Duration(seconds: 2));
      await tester.pumpAndSettle();
      expect(find.text('Public · Online'), findsNothing);
      await tester.tap(find.byTooltip('Show links for website'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Copy public URL'), findsNothing);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      backend.publicAccess!['host'] = peer;
      publication['incarnation'] = 'e' * 64;
      await tester.pump(const Duration(seconds: 2));
      await tester.pumpAndSettle();
      expect(find.text('Public · Online'), findsNothing);
      await tester.tap(find.byTooltip('Show links for website'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Copy public URL'), findsNothing);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      await close(tester);
      publication['incarnation'] = incarnation;
      await mount(
        tester,
        FakeBackend(state: 'unreachable', publicAccess: backend.publicAccess),
      );
      expect(find.text('Public · Cached'), findsOneWidget);
      expect(find.text('Public · Online'), findsNothing);
      expect(find.text('Public URL'), findsNothing);
      await tester.tap(find.byTooltip('Show links for website'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Copy public URL'), findsOneWidget);
      await close(tester);
    },
  );
  testWidgets(
    'desktop link popup shows local and public URLs without an Android option',
    (tester) async {
      final backend = DesktopFake();
      backend.status['public_access']['services'] = [
        {
          'service': 'web',
          'incarnation': incarnation,
          'enabled': true,
          'state': 'online',
          'url': 'https://shared-web.wispnet.test/',
        },
      ];
      await mount(tester, backend, size: const Size(1000, 850));
      await tester.tap(find.byTooltip('Show links for web'));
      await tester.pumpAndSettle();
      expect(find.text('http://shared-web.localhost:18000/'), findsOneWidget);
      expect(find.text('https://shared-web.wispnet.test/'), findsOneWidget);
      expect(find.byTooltip('Copy Wispnet link'), findsNothing);
      expect(
        backend.calls.where((c) => ['open', 'launch'].contains(c.$1)),
        isEmpty,
      );
      await close(tester);
    },
  );
  testWidgets(
    'link popup retains portable addresses when local resolution fails and can retry',
    (tester) async {
      final backend =
          FakeBackend(
              publicAccess: {
                'version': 1,
                'host': peer,
                'services': [
                  {
                    'service': 'website',
                    'incarnation': incarnation,
                    'state': 'online',
                    'url': 'https://shared-web.wispnet.test/',
                  },
                ],
              },
            )
            ..missingRoute = true
            ..failOpen = true;
      await mount(tester, backend);
      await tester.tap(find.byTooltip('Show links for website'));
      await tester.pumpAndSettle();
      expect(find.text('Host unavailable'), findsOneWidget);
      expect(find.byTooltip('Copy Wispnet link'), findsOneWidget);
      expect(find.byTooltip('Copy public URL'), findsOneWidget);
      backend.failOpen = false;
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      expect(
        find.text('http://website.wispnet.localhost:18300'),
        findsOneWidget,
      );
      expect(
        backend.calls.where((c) => c.$1 == 'open').last.$2['incarnation'],
        incarnation,
      );
      expect(backend.calls.where((c) => c.$1 == 'launch'), isEmpty);
      await close(tester);
    },
  );
  testWidgets('desktop public access requires explicit confirmation', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(find.byTooltip('Settings for web on hibreak'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('Enable public access'));
    await tester.tap(find.text('Enable public access'));
    await tester.pumpAndSettle();
    expect(backend.ownerCalls, isEmpty);
    expect(
      find.textContaining('unprotected to the public internet'),
      findsOneWidget,
    );
    expect(
      find.textContaining(
        'without pairing, device approval, or Wispnet authentication',
      ),
      findsOneWidget,
    );
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(backend.ownerCalls, isEmpty);
    await tester.tap(find.text('Enable public access'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Enable public access'));
    await tester.pumpAndSettle();
    expect(backend.ownerCalls.single['op'], 'configure_public');
    expect(backend.ownerCalls.single['incarnation'], incarnation);
    expect(backend.ownerCalls.single['enabled'], true);
    await close(tester);
  });
  testWidgets(
    'local service danger actions have spacing and removal requires confirmation',
    (tester) async {
      final backend = DesktopFake();
      await mount(tester, backend, size: const Size(1000, 850));
      await tester.tap(find.byTooltip('Settings for web on hibreak'));
      await tester.pumpAndSettle();
      final edit = find.widgetWithText(OutlinedButton, 'Edit');
      final remove = find.widgetWithText(OutlinedButton, 'Remove service');
      final publish = find.widgetWithText(
        OutlinedButton,
        'Enable public access',
      );
      expect(
        tester.getRect(remove).top - tester.getRect(edit).bottom,
        greaterThanOrEqualTo(8),
      );
      expect(
        tester.getRect(publish).top - tester.getRect(remove).bottom,
        greaterThanOrEqualTo(8),
      );
      for (final button in [remove, publish]) {
        expect(
          tester
              .widget<OutlinedButton>(button)
              .style!
              .foregroundColor!
              .resolve({}),
          WispColors.error,
        );
      }
      await tester.tap(remove);
      await tester.pumpAndSettle();
      expect(backend.ownerCalls, isEmpty);
      expect(
        find.textContaining('Paired devices will lose Wispnet access'),
        findsOneWidget,
      );
      expect(
        find.textContaining(
          'does not stop or delete the underlying application',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(backend.ownerCalls, isEmpty);
      await tester.tap(remove);
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Remove service'));
      await tester.pumpAndSettle();
      expect(backend.ownerCalls.single['op'], 'mutate');
      expect((backend.ownerCalls.single['operation'] as Map)['op'], 'remove');
      await close(tester);
    },
  );
  testWidgets('desktop pairing approval preserves existing service grants', (
    tester,
  ) async {
    final backend = DesktopFake();
    await mount(tester, backend, size: const Size(1000, 850));
    await tester.tap(
      find
          .descendant(
            of: find.byKey(const ValueKey('remote-sector')),
            matching: find.byType(OutlinedButton),
          )
          .first,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Review 1 pairing requests'));
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
