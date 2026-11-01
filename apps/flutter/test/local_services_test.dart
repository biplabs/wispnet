import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/desktop_settings.dart';
import 'package:wispnet/ui.dart';

import 'android_file_shares_test.dart' as fixture;
import 'folder_services_test.dart' as folders;

class ServiceListBackend extends folders.FolderBackend {
  final ordinary = <Map<String, dynamic>>[
    {
      'id': 'shell',
      'kind': 'shell',
      'backend': '127.0.0.1:0',
      'incarnation': '4' * 64,
      'allow': [fixture.laptop],
      'enabled': true,
    },
    {
      'id': 'research',
      'kind': 'http',
      'backend': '127.0.0.1:8080',
      'incarnation': '5' * 64,
      'allow': [fixture.other],
      'enabled': true,
    },
    {
      'id': 'database',
      'kind': 'tcp',
      'backend': '127.0.0.1:5432',
      'incarnation': '6' * 64,
      'allow': <String>[],
      'enabled': false,
    },
  ];

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    if (method == 'desktopStatus') {
      final response = await super.invoke(method, arguments);
      (response['host']['services'] as List).insertAll(0, ordinary);
      return response;
    }
    if (method == 'setServiceEnabled' &&
        ordinary.any((service) => service['id'] == arguments['service'])) {
      calls.add((method, Map.of(arguments)));
      ordinary.singleWhere(
        (service) => service['id'] == arguments['service'],
      )['enabled'] = arguments['enabled'];
      revision++;
      return {};
    }
    return super.invoke(method, arguments);
  }
}

Future<void> owner(WidgetTester tester, ServiceListBackend backend) =>
    fixture.mount(
      tester,
      DesktopSettings(
        backend: backend,
        servicesOnly: true,
        onAccess: () {},
        onChanged: () {},
      ),
    );

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

  testWidgets(
    'Local services lists Shell, HTTP, TCP and folders as uniform existing rows',
    (tester) async {
      final backend = ServiceListBackend();
      await owner(tester, backend);
      for (final id in [
        'shell',
        'research',
        'database',
        'training-data',
        'predictions',
        'checkpoints',
        'datasets',
      ]) {
        final row = find.byKey(ValueKey('local-service-$id'));
        expect(row, findsOneWidget);
        expect(
          find.descendant(of: row, matching: find.byType(WispToggle)),
          findsOneWidget,
        );
        expect(
          find.descendant(of: row, matching: find.byType(WispSettingsButton)),
          findsOneWidget,
        );
        expect(
          find.descendant(of: row, matching: find.byType(WispBadge)),
          findsOneWidget,
        );
      }
      expect(find.byKey(const ValueKey('local-service-files')), findsNothing);
      expect(find.text('Pair this machine'), findsNothing);
      expect(find.text('Shell access'), findsNothing);
      expect(find.byType(WispCreateButton), findsNWidgets(2));
      final lastRow = find.byKey(const ValueKey('local-service-datasets'));
      final create = find.widgetWithText(WispCreateButton, 'Add a new service');
      await tester.ensureVisible(create);
      expect(
        tester.getRect(create).top,
        greaterThan(tester.getRect(lastRow).bottom),
      );
      final button = find.descendant(
        of: create,
        matching: find.byType(FilledButton),
      );
      final style = tester.widget<FilledButton>(button).style!;
      expect(style.backgroundColor!.resolve({}), const Color(0xff123d32));
      expect(
        find.descendant(of: create, matching: find.byIcon(Icons.add)),
        findsOneWidget,
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'each service gear opens its existing settings without mutating grants',
    (tester) async {
      final backend = ServiceListBackend();
      await owner(tester, backend);
      await tester.tap(find.byTooltip('Shell access'));
      await tester.pumpAndSettle();
      expect(find.text('Shell access'), findsOneWidget);
      expect(find.widgetWithText(TextField, 'Port'), findsNothing);
      expect(find.text('Save access'), findsOneWidget);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Settings for research'));
      await tester.pumpAndSettle();
      expect(find.widgetWithText(TextField, 'Port'), findsOneWidget);
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Service ID'))
            .controller!
            .text,
        'research',
      );
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Settings for database'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Service ID'))
            .controller!
            .text,
        'database',
      );
      expect(
        backend.calls.where((call) => call.$1 == 'desktopRequest'),
        isEmpty,
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'ordinary service toggles target only the selected incarnation and keep grants',
    (tester) async {
      final backend = ServiceListBackend();
      await owner(tester, backend);
      await tester.tap(
        find.byKey(const ValueKey('local-service-toggle-shell')),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('local-service-toggle-database')),
      );
      await tester.pumpAndSettle();
      final toggles = backend.calls
          .where((call) => call.$1 == 'setServiceEnabled')
          .map((call) => call.$2)
          .toList();
      expect(toggles, [
        {'service': 'shell', 'incarnation': '4' * 64, 'enabled': false},
        {'service': 'database', 'incarnation': '6' * 64, 'enabled': true},
      ]);
      expect(backend.ordinary[0]['allow'], [fixture.laptop]);
      expect(backend.ordinary[1]['enabled'], isTrue);
      expect(backend.ordinary[1]['allow'], [fixture.other]);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'revised Local services fits desktop and phone with creation below the rows',
    (tester) async {
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      tester.view.devicePixelRatio = 1;
      for (final size in [const Size(1000, 800), const Size(360, 780)]) {
        tester.view.physicalSize = size;
        final backend = ServiceListBackend();
        backend.inboxes.removeLast();
        backend.shares.clear();
        await fixture.mount(
          tester,
          Builder(
            builder: (context) => TextButton(
              onPressed: () => showWispSheet<void>(
                context,
                title: 'Local services',
                child: DesktopSettings(
                  backend: backend,
                  servicesOnly: true,
                  onAccess: () {},
                  onChanged: () {},
                ),
              ),
              child: const Text('Open Local services'),
            ),
          ),
        );
        await tester.tap(find.text('Open Local services'));
        await tester.pumpAndSettle();
        await fixture.capture(
          tester,
          size.width > 500
              ? 'local-services-uniform-desktop'
              : 'local-services-uniform-phone',
        );
        expect(tester.takeException(), isNull);
        expect(find.text('Pair this machine'), findsNothing);
        await tester.pumpWidget(const SizedBox());
      }
    },
  );
}
