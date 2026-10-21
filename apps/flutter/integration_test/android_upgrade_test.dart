import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets(
    'installed identity, saved machines, settings and online lifecycle',
    (tester) async {
      final backend = PlatformBackend();
      final before = await backend.invoke('snapshot');
      final originalEnabled = before['enabled'] == true;
      final key = GlobalKey();
      Future<void> waitFor(Finder finder) async {
        for (var n = 0; n < 100 && finder.evaluate().isEmpty; n++) {
          await tester.pump(const Duration(milliseconds: 200));
        }
        expect(finder, findsWidgets);
      }

      Future<void> capture(String name) async {
        await tester.pump(const Duration(milliseconds: 300));
        final boundary =
            key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
        final image = await boundary.toImage();
        final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
        await File('/data/data/app.wispnet.android/cache/$name.png')
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
        await waitFor(find.byTooltip('Settings'));
        for (
          var n = 0;
          n < 100 &&
              find.textContaining(before['name'] as String).evaluate().isEmpty;
          n++
        ) {
          await tester.pump(const Duration(milliseconds: 200));
        }
        expect(find.textContaining(before['name'] as String), findsWidgets);
        await tester.tap(find.byTooltip('Settings'));
        await waitFor(find.widgetWithText(TextField, 'Registry port'));
        await waitFor(find.widgetWithText(FilledButton, 'Save'));
        expect(
          tester
              .widget<FilledButton>(find.widgetWithText(FilledButton, 'Save'))
              .onPressed,
          isNotNull,
        );
        await capture('flutter-device-settings');
        expect(tester.takeException(), isNull);
        await tester.tap(find.byTooltip('Close'));
        await tester.pump(const Duration(milliseconds: 500));
        if (!originalEnabled && (before['hosts'] as Map).isNotEmpty) {
          await tester.tap(find.byTooltip('Enter wisp space'));
        }
        if ((before['hosts'] as Map).isNotEmpty) {
          await waitFor(find.text('PAIRED MACHINES'));
          await tester.pump(const Duration(seconds: 2));
          await capture('flutter-device-directory');
          await tester.tap(find.text('Manage'));
          await waitFor(find.text('Paired machines'));
          await capture('flutter-device-machines');
          await tester.tap(find.byTooltip('Close'));
          await tester.pump(const Duration(milliseconds: 500));
        } else {
          await tester.tap(find.byTooltip('Establish your wisp network'));
          await waitFor(find.text('Pair a machine'));
          await capture('flutter-device-pairing');
          await tester.tap(find.byTooltip('Close'));
          await tester.pump(const Duration(milliseconds: 500));
        }
        if ((before['hosts'] as Map).isNotEmpty) {
          await tester.tap(find.text('GO OFFLINE'));
          await waitFor(find.byTooltip('Enter wisp space'));
          expect((await backend.invoke('snapshot'))['enabled'], false);
          await tester.tap(find.byTooltip('Enter wisp space'));
          await waitFor(find.text('PAIRED MACHINES'));
          await tester.pump(const Duration(seconds: 2));
          final current = await backend.invoke('snapshot');
          Map<String, dynamic>? selected;
          for (final record in (current['hosts'] as Map).values) {
            if (record['state'] != 'available') continue;
            for (final service in record['catalog']['services'] as List) {
              if (service['kind'] == 'http') {
                selected = (service as Map).cast<String, dynamic>();
                break;
              }
            }
            if (selected != null) break;
          }
          expect(
            selected,
            isNotNull,
            reason:
                'A live HTTP service is required for the device handoff check',
          );
          await tester.tap(
            find.text(selected!['display_name'] as String).first,
          );
          await tester.runAsync(() async {
            for (
              var n = 0;
              n < 100 &&
                  WidgetsBinding.instance.lifecycleState !=
                      AppLifecycleState.paused;
              n++
            ) {
              await Future<void>.delayed(const Duration(milliseconds: 200));
            }
          });
          expect(
            WidgetsBinding.instance.lifecycleState,
            AppLifecycleState.paused,
            reason: 'Opening a service must hand off to the installed browser',
          );
        }
        final after = await backend.invoke('snapshot');
        expect(after['peer'], before['peer']);
        expect(after['name'], before['name']);
        expect(
          (after['hosts'] as Map).keys.toSet(),
          (before['hosts'] as Map).keys.toSet(),
        );
        expect(tester.takeException(), isNull);
      } finally {
        await backend.invoke('setAccess', {'enabled': originalEnabled});
      }
    },
  );
}
