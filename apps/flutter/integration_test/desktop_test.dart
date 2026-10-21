import 'dart:io';
import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/cards.dart';
import 'package:wispnet/src/desktop_backend.dart';
import 'package:wispnet/src/header.dart';
import 'package:wispnet/src/theme.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets(
    'desktop uses shared directory, owner sheets and real browser handoff',
    (tester) async {
      final native = DesktopBackend();
      final backend = _ObservedBackend(native);
      final before = await native.invoke('snapshot');
      expect(
        before['enabled'],
        true,
        reason: 'Run with the existing node online',
      );
      final key = GlobalKey();
      Future<void> waitFor(Finder finder) async {
        for (var n = 0; n < 150 && finder.evaluate().isEmpty; n++) {
          await tester.pump(const Duration(milliseconds: 200));
        }
        expect(finder, findsWidgets);
      }

      Future<void> capture(String name) async {
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 300));
        final image =
            await (key.currentContext!.findRenderObject()!
                    as RenderRepaintBoundary)
                .toImage();
        final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
        final directory = Directory('/tmp/wispnet-flutter-desktop-review')
          ..createSync(recursive: true);
        await File('${directory.path}/$name.png')
            .writeAsBytes(bytes!.buffer.asUint8List());
        image.dispose();
      }

      try {
        await tester.pumpWidget(
          RepaintBoundary(
            key: key,
            child: WispnetApp(backend: backend),
          ),
        );
        await waitFor(find.text('REMOTE SECTOR'));
        await tester.pump(const Duration(seconds: 3));
        await capture('directory');
        final mouse = await tester.createGesture(
          kind: ui.PointerDeviceKind.mouse,
        );
        await mouse.addPointer(location: Offset.zero);
        await mouse.moveTo(tester.getCenter(find.byType(OnlineButton)));
        await capture('hover-online');
        await mouse.moveTo(tester.getCenter(find.text('Manage').first));
        await capture('hover-manage');
        final serviceTile = find
            .descendant(
              of: find.byKey(const ValueKey('local-sector')),
              matching: find.byType(WispInkWell),
            )
            .first;
        await mouse.moveTo(tester.getCenter(serviceTile));
        await capture('hover-service');
        final machineName = find
            .descendant(
              of: find
                  .byWidgetPredicate(
                    (widget) => widget is MachineCard && widget.showHeader,
                  )
                  .first,
              matching: find.byType(WispInkWell),
            )
            .first;
        await mouse.moveTo(tester.getCenter(machineName));
        await capture('hover-machine-name');
        await mouse.moveTo(Offset.zero);
        await mouse.removePointer();
        await tester.tap(machineName);
        await waitFor(find.widgetWithText(TextField, 'Private display name'));
        await capture('machine-settings');
        expect(find.widgetWithText(OutlinedButton, 'Refresh'), findsNothing);
        await tester.tap(find.widgetWithText(OutlinedButton, 'Remove'));
        await waitFor(find.text('Remove machine?'));
        await capture('remove-machine-confirmation');
        await tester.tap(find.text('Cancel'));
        await tester.pump(const Duration(milliseconds: 500));
        await tester.tap(find.byTooltip('Close'));
        await tester.pump(const Duration(milliseconds: 500));
        await tester.tap(
          find
              .descendant(
                of: find.byKey(const ValueKey('remote-sector')),
                matching: find.byType(OutlinedButton),
              )
              .first,
        );
        await waitFor(find.text('+ Pair another machine'));
        await tester.tap(find.text('+ Pair another machine'));
        await waitFor(find.text('Pair a machine'));
        expect(find.text('Pairing Code'), findsNothing);
        expect(find.text('QR'), findsNothing);
        await capture('pairing');
        await tester.tap(find.byTooltip('Close'));
        await tester.pump(const Duration(milliseconds: 500));
        await tester.tap(
          find.descendant(
            of: find.byKey(const ValueKey('local-sector')),
            matching: find.text('Manage'),
          ),
        );
        await waitFor(find.text('Add a new service'));
        expect(find.text('Pair this machine'), findsOneWidget);
        expect(find.text('Remove service'), findsNothing);
        expect(find.text('Enable public access'), findsNothing);
        await capture('local-services');
        await tester.tap(find.text('Pair this machine'));
        await waitFor(find.byKey(const ValueKey('pairing-qr')));
        await capture('pair-this-machine-qr');
        await tester.tap(find.text('Pairing code'));
        await waitFor(find.byKey(const ValueKey('pairing-code')));
        expect(
          tester
              .widget<SelectableText>(
                find.byKey(const ValueKey('pairing-code')),
              )
              .data,
          startsWith('wispnet1:'),
        );
        await capture('pair-this-machine-code');
        await tester.tap(find.byTooltip('Close').last);
        await tester.pump(const Duration(milliseconds: 500));
        await tester.tap(find.byTooltip('Close'));
        await tester.pump(const Duration(milliseconds: 500));
        await tester.tap(
          find
              .descendant(
                of: find.byKey(const ValueKey('local-sector')),
                matching: find.byType(IconButton),
              )
              .first,
        );
        await waitFor(find.text('Remove service'));
        expect(find.text('Add a new service'), findsNothing);
        await capture('local-service-controls');
        await tester.ensureVisible(find.text('Remove service').first);
        await tester.tap(find.text('Remove service').first);
        await waitFor(find.text('Remove service?'));
        await capture('remove-service-confirmation');
        await tester.tap(find.text('Cancel'));
        await tester.pump(const Duration(milliseconds: 500));
        await tester.ensureVisible(find.text('Enable public access').first);
        await tester.tap(find.text('Enable public access').first);
        await waitFor(find.text('Expose service to the internet?'));
        expect(
          find.textContaining('unprotected to the public internet'),
          findsOneWidget,
        );
        await capture('public-service-confirmation');
        await tester.tap(find.text('Cancel'));
        await tester.pump(const Duration(milliseconds: 500));
        expect(find.text('Device and browser settings'), findsNothing);
        await tester.tap(find.byTooltip('Close'));
        for (
          var i = 0;
          i < 15 && find.byTooltip('Close').evaluate().isNotEmpty;
          i++
        ) {
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.tap(find.byTooltip('Settings'));
        await waitFor(find.widgetWithText(TextField, 'Registry port'));
        expect(find.text('Device and browser settings'), findsNothing);
        expect(find.byTooltip('Close'), findsOneWidget);
        await capture('settings');
        await tester.pump(const Duration(seconds: 1));
        expect(
          tester
              .widget<TextField>(
                find.widgetWithText(TextField, 'Registry port'),
              )
              .controller!
              .text,
          isNotEmpty,
        );
        await capture('browser-settings');
        await tester.tap(find.byKey(const ValueKey('settings-address')));
        for (var i = 0; i < 100 && backend.launched == null; i++) {
          await tester.pump(const Duration(milliseconds: 100));
          await tester.runAsync(
            () => Future<void>.delayed(const Duration(milliseconds: 100)),
          );
        }
        expect(backend.launched, endsWith('/v1/services'));
        await tester.runAsync(() async {
          final client = HttpClient();
          try {
            final uri = Uri.parse(backend.launched!);
            // The local registry listens on IPv4; this hostname also resolves to ::1.
            final request = await client.getUrl(uri.replace(host: '127.0.0.1'));
            request.headers.set(HttpHeaders.hostHeader, uri.authority);
            request.headers.set('Sec-Fetch-Site', 'none');
            request.headers.set('Sec-Fetch-Mode', 'navigate');
            request.headers.set('Sec-Fetch-Dest', 'document');
            final response = await request.close();
            expect(response.statusCode, 200);
            expect(response.headers.contentType?.mimeType, 'application/json');
            final json =
                jsonDecode(await utf8.decoder.bind(response).join()) as Map;
            expect(json['api_version'], 1);
            expect(json['services'], isA<List>());
            for (final service in json['services'] as List) {
              expect(service['url'], isA<String>(), reason: service.toString());
            }
          } finally {
            client.close(force: true);
          }
        });
        backend.launched = null;

        Future<void> closeSheet(int remaining) async {
          await tester.tap(find.byTooltip('Close').last);
          for (var i = 0; i < 15; i++) {
            await tester.pump(const Duration(milliseconds: 100));
            if (find.byTooltip('Close').evaluate().length == remaining) break;
          }
          expect(find.byTooltip('Close').evaluate().length, remaining);
        }

        await closeSheet(0);
        await tester.tap(find.byTooltip('Online'));
        await waitFor(find.byTooltip('Enter wisp space'));
        expect((await native.invoke('snapshot'))['enabled'], false);
        expect(find.text('hyperjump to wispnet'), findsOneWidget);
        expect(find.byTooltip('Settings'), findsNothing);
        await capture('launch');
        await tester.tap(find.byTooltip('Enter wisp space'));
        await waitFor(find.text('REMOTE SECTOR'));
        expect((await native.invoke('snapshot'))['peer'], before['peer']);
        for (
          var n = 0;
          n < 100 &&
              tester
                      .widget<OnlineButton>(find.byType(OnlineButton))
                      .onPressed ==
                  null;
          n++
        ) {
          await tester.pump(const Duration(milliseconds: 100));
          await tester.runAsync(
            () => Future<void>.delayed(const Duration(milliseconds: 20)),
          );
        }
        expect(
          tester.widget<OnlineButton>(find.byType(OnlineButton)).onPressed,
          isNotNull,
        );
        final local = (before['hosts'] as Map)[before['peer']] as Map;
        final services = (local['catalog']['services'] as List)
            .where((s) => s['kind'] == 'http')
            .toList();
        expect(
          services,
          isNotEmpty,
          reason: 'A running HTTP service is required for live handoff',
        );
        tester.binding.handleAppLifecycleStateChanged(
          AppLifecycleState.resumed,
        );
        await tester.tap(
          find
              .textContaining(
                services.first['display_name'] as String,
                findRichText: true,
              )
              .first,
        );
        for (var i = 0; i < 100 && backend.launched == null; i++) {
          await tester.pump(const Duration(milliseconds: 100));
          await tester.runAsync(
            () => Future<void>.delayed(const Duration(milliseconds: 100)),
          );
        }
        expect(backend.launched, isNotNull, reason: backend.calls.join(', '));
        await tester.runAsync(() async {
          final client = HttpClient();
          try {
            final response = await (await client.getUrl(
              Uri.parse(backend.launched!),
            )).close();
            expect(response.statusCode, lessThan(500));
            await response.drain<void>();
          } finally {
            client.close(force: true);
          }
        });
        final after = await native.invoke('snapshot');
        expect(after['peer'], before['peer']);
        expect(
          (after['hosts'] as Map).keys.toSet(),
          (before['hosts'] as Map).keys.toSet(),
        );
        expect(tester.takeException(), isNull);
      } finally {
        await tester.pumpWidget(const SizedBox());
        if ((await native.invoke('snapshot'))['enabled'] != before['enabled']) {
          await native.invoke('setAccess', {'enabled': before['enabled']});
        }
        await native.close();
      }
    },
  );
}

class _ObservedBackend implements WispnetBackend {
  _ObservedBackend(this.native);
  final DesktopBackend native;
  String? launched;
  final calls = <String>[];
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add(method);
    final result = await native.invoke(method, arguments);
    if (method == 'launch') launched = arguments['url'] as String;
    return result;
  }
}
