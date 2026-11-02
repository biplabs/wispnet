import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/service_settings.dart';

import 'widget_test.dart' as fixture;

class OwnerFeaturesBackend extends fixture.DesktopFake {
  OwnerFeaturesBackend(this.android) {
    if (android) {
      (status['host']['services'] as List).removeWhere(
        (service) => service['kind'] == 'shell',
      );
    }
  }
  final bool android;

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    final result = await super.invoke(method, arguments);
    if (method == 'snapshot') return {...result, 'desktop': !android};
    return result;
  }
}

Future<void> mountOwner(WidgetTester tester, OwnerFeaturesBackend backend) =>
    fixture.mount(
      tester,
      backend,
      size: backend.android ? const Size(390, 844) : const Size(1000, 850),
    );

Future<void> openOwnerTools(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Settings'));
  await tester.pumpAndSettle();
  final tools = find.text('Device access and pairing');
  await tester.ensureVisible(tools);
  await tester.tap(tools);
  await tester.pumpAndSettle();
}

void main() {
  setUpAll(() async {
    await (FontLoader('Roboto')
          ..addFont(rootBundle.load('assets/fonts/Roboto-Regular.ttf'))
          ..addFont(rootBundle.load('assets/fonts/Roboto-Medium.ttf'))
          ..addFont(rootBundle.load('assets/fonts/Roboto-Bold.ttf')))
        .load();
    await (FontLoader(
      'MaterialIcons',
    )..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
  });
  for (final android in [false, true]) {
    final platform = android ? 'Android' : 'desktop';
    testWidgets(
      '$platform owner entry opens the shared Devices page and issues a pairing QR',
      (tester) async {
        final backend = OwnerFeaturesBackend(android);
        await mountOwner(tester, backend);
        await openOwnerTools(tester);
        final settings = tester.widget<ServiceSettings>(
          find.byType(ServiceSettings),
        );
        expect(settings.android, android);
        expect(settings.initialSection, 'Devices');
        expect(
          tester
              .widget<DropdownButton<String>>(
                find.byType(DropdownButton<String>),
              )
              .value,
          'Devices',
        );
        expect(find.text('Existing device'), findsOneWidget);
        await tester.ensureVisible(find.text('Pair this machine'));
        await tester.tap(find.text('Pair this machine'));
        await tester.pumpAndSettle();
        expect(find.byKey(const ValueKey('pairing-qr')), findsOneWidget);
        expect(
          backend.ownerCalls.where((call) => call['op'] == 'invite_qr'),
          hasLength(1),
        );
        await tester.tap(find.text('Pairing code'));
        await tester.pumpAndSettle();
        expect(find.text('wispnet1:generated-code'), findsOneWidget);
        await fixture.close(tester);
      },
    );

    for (final kind in ['http', 'tcp']) {
      testWidgets(
        '$platform creates a shared $kind service with an explicit port and device grant',
        (tester) async {
          final backend = OwnerFeaturesBackend(android);
          await mountOwner(tester, backend);
          await tester.tap(find.byTooltip('Settings'));
          await tester.pumpAndSettle();
          await tester.ensureVisible(find.text('Local services'));
          await tester.tap(find.text('Local services'));
          await tester.pumpAndSettle();
          await tester.ensureVisible(
            find.byKey(const ValueKey('share-service')),
          );
          await tester.tap(find.byKey(const ValueKey('share-service')));
          await tester.pumpAndSettle();
          expect(find.widgetWithText(Tab, 'Custom'), findsOneWidget);
          await tester.tap(find.widgetWithText(Tab, 'Custom'));
          await tester.pumpAndSettle();
          await tester.enterText(
            find.widgetWithText(TextField, 'Port'),
            '8201',
          );
          await tester.enterText(
            find.widgetWithText(TextField, 'Service ID'),
            'shared-$kind',
          );
          if (kind == 'tcp') {
            await tester.tap(find.byType(DropdownButtonFormField<String>));
            await tester.pumpAndSettle();
            await tester.tap(find.text('TCP').last);
            await tester.pumpAndSettle();
          }
          await tester.ensureVisible(find.text('Existing device'));
          await tester.tap(find.text('Existing device'));
          await tester.ensureVisible(find.widgetWithText(FilledButton, 'Save'));
          await tester.tap(find.widgetWithText(FilledButton, 'Save'));
          await tester.pumpAndSettle();
          expect(backend.ownerCalls.single, {
            'op': 'mutate',
            'operation': {
              'op': 'service',
              'id': 'shared-$kind',
              'backend': '127.0.0.1:8201',
              'kind': kind,
              'allow': [fixture.peer],
              'expected_revision': 7,
            },
          });
          expect(tester.takeException(), isNull);
          await fixture.close(tester);
        },
      );
    }

    testWidgets(
      '$platform public HTTP access requires the shared explicit confirmation',
      (tester) async {
        final backend = OwnerFeaturesBackend(android);
        await mountOwner(tester, backend);
        await tester.tap(find.byTooltip('Settings for web on hibreak'));
        await tester.pumpAndSettle();
        final enable = find.widgetWithText(
          OutlinedButton,
          'Enable public access',
        );
        await tester.ensureVisible(enable);
        await tester.tap(enable);
        await tester.pumpAndSettle();
        expect(find.text('Expose service to the internet?'), findsOneWidget);
        expect(
          find.textContaining('unprotected to the public internet'),
          findsOneWidget,
        );
        expect(backend.ownerCalls, isEmpty);
        await tester.ensureVisible(find.text('Cancel').last);
        await tester.tap(find.text('Cancel').last);
        await tester.pumpAndSettle();
        expect(backend.ownerCalls, isEmpty);
        await tester.ensureVisible(enable);
        await tester.tap(enable);
        await tester.pumpAndSettle();
        final confirm = find.widgetWithText(
          FilledButton,
          'Enable public access',
        );
        await tester.ensureVisible(confirm);
        await tester.tap(confirm);
        await tester.pumpAndSettle();
        expect(backend.ownerCalls.single, {
          'op': 'configure_public',
          'service': 'web',
          'incarnation': fixture.incarnation,
          'enabled': true,
          'name': 'Desktop',
        });
        await fixture.close(tester);
      },
    );

    testWidgets(
      '$platform reviews and approves pairing requests through the shared owner page',
      (tester) async {
        final backend = OwnerFeaturesBackend(android);
        await mountOwner(tester, backend);
        await openOwnerTools(tester);
        await tester.tap(find.byType(DropdownButton<String>));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Pairing requests (1)').last);
        await tester.pumpAndSettle();
        final approve = find.widgetWithText(OutlinedButton, 'Approve device');
        await tester.ensureVisible(approve);
        await tester.tap(approve);
        await tester.pumpAndSettle();
        expect(backend.ownerCalls, isEmpty);
        final confirm = find.widgetWithText(FilledButton, 'Approve device');
        await tester.ensureVisible(confirm);
        await tester.tap(confirm);
        await tester.pumpAndSettle();
        expect(backend.ownerCalls.single, {
          'op': 'mutate',
          'operation': {
            'op': 'approve_invitation',
            'invitation': 'request-id',
            'services': ['web'],
            'expected_revision': 7,
          },
        });
        await fixture.close(tester);
      },
    );
  }
}
