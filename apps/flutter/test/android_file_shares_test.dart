import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/android_file_shares.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/sheets.dart';
import 'package:wispnet/ui.dart';

final own = 'a' * 64;
final laptop = 'b' * 64;
final other = 'c' * 64;
final incarnation = 'd' * 64;

class AndroidSharesBackend implements WispnetBackend {
  final calls = <(String, Map<String, dynamic>)>[];
  int revision = 3;
  Object? pickerError;
  Map<String, dynamic> selection = {
    'folder': 'opaque-selected-folder',
    'label': 'Selected checkpoints',
  };
  final shares = <Map<String, dynamic>>[
    {
      'id': 'checkpoints',
      'folder': 'opaque-checkpoints',
      'label': 'Training checkpoints',
      'allow': [laptop],
      'enabled': true,
      'available': true,
      'incarnation': incarnation,
    },
    {
      'id': 'datasets',
      'folder': 'opaque-datasets',
      'label': 'Datasets folder',
      'allow': [other],
      'enabled': true,
      'available': true,
      'incarnation': 'e' * 64,
    },
  ];

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, Map.of(arguments)));
    switch (method) {
      case 'sharesStatus':
        return {
          'revision': revision,
          'clients': [
            {'peer': laptop, 'name': 'My laptop'},
            {'peer': other, 'name': 'Other device'},
          ],
          'shares': [for (final share in shares) Map.of(share)],
        };
      case 'pickShareFolder':
        if (pickerError case final Object error) throw error;
        return selection;
      case 'saveShare':
        expect(arguments['expected_revision'], revision);
        final id = arguments['id'];
        final existing = shares.where((share) => share['id'] == id).firstOrNull;
        final updated = {
          ...?existing,
          'id': id,
          'folder': arguments['folder'],
          'allow': List.of(arguments['allow'] as List),
          'available': true,
          'enabled': true,
          'incarnation': incarnation,
          'label': arguments['folder'] == selection['folder']
              ? selection['label']
              : existing?['label'] ?? 'Shared folder',
        };
        shares.removeWhere((share) => share['id'] == id);
        shares.add(updated);
        revision++;
        return {'revision': revision};
      case 'removeShare':
        expect(arguments['expected_revision'], revision);
        shares.removeWhere((share) => share['id'] == arguments['id']);
        return {'revision': ++revision};
      case 'setServiceEnabled':
        shares.singleWhere(
          (share) => share['id'] == arguments['service'],
        )['enabled'] = arguments['enabled'];
        revision++;
        return {};
      case 'snapshot':
        return {
          'peer': own,
          'name': 'Android phone',
          'enabled': true,
          'network': 'Network available',
          'order': {},
          'hosts': {
            own: {
              'name': 'Android phone',
              'local': true,
              'state': 'available',
              'catalog': {
                'host': own,
                'services': [
                  {
                    'id': 'files',
                    'kind': 'files',
                    'incarnation': 'f' * 64,
                    'enabled': true,
                  },
                  for (final share in shares) {...share, 'kind': 'file_share'},
                ],
              },
            },
          },
        };
      case 'fileList':
        return {'transfers': []};
      case 'takeLink':
      case 'takeInvitation':
      case 'refreshHost':
        return {};
      default:
        throw StateError('Unexpected Android operation $method');
    }
  }
}

Future<void> mount(WidgetTester tester, Widget child) async {
  await tester.pumpWidget(
    RepaintBoundary(
      key: const ValueKey('android-share-preview'),
      child: MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: wispTheme(),
        home: Scaffold(
          body: SingleChildScrollView(
            padding: const EdgeInsets.all(20),
            child: child,
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

Future<void> capture(WidgetTester tester, String name) async {
  final folder = Platform.environment['WISPNET_SHARE_SCREENSHOTS'];
  if (folder == null) return;
  await tester.runAsync(() async {
    final boundary = tester.firstRenderObject<RenderRepaintBoundary>(
      find.byKey(const ValueKey('android-share-preview')),
    );
    final image = await boundary.toImage();
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    Directory(folder).createSync(recursive: true);
    File('$folder/$name.png').writeAsBytesSync(bytes!.buffer.asUint8List());
    image.dispose();
  });
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

  testWidgets(
    'Android creates a picker-selected folder share with its own device list',
    (tester) async {
      final backend = AndroidSharesBackend();
      await mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      await tester.tap(find.byKey(const ValueKey('add-android-share')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('share-root')), findsNothing);
      await tester.enterText(
        find.byKey(const ValueKey('share-name')),
        'results',
      );
      expect(
        tester
            .widget<FilledButton>(
              find.widgetWithText(FilledButton, 'Save read-only share'),
            )
            .onPressed,
        isNull,
      );
      await tester.ensureVisible(find.text('Choose shared folder'));
      await tester.tap(find.text('Choose shared folder'));
      await tester.pumpAndSettle();
      expect(find.text('Selected checkpoints'), findsOneWidget);
      expect(find.textContaining('opaque-selected-folder'), findsNothing);
      await tester.ensureVisible(find.text('My laptop'));
      await tester.tap(find.text('My laptop'));
      await tester.ensureVisible(find.text('Save read-only share'));
      await tester.tap(find.text('Save read-only share'));
      await tester.pumpAndSettle();
      expect(backend.calls.where((call) => call.$1 == 'saveShare').single.$2, {
        'expected_revision': 3,
        'id': 'results',
        'folder': 'opaque-selected-folder',
        'allow': [laptop],
      });
      expect(
        backend.shares.singleWhere(
          (share) => share['id'] == 'datasets',
        )['allow'],
        [other],
      );
      expect(backend.calls.any((call) => call.$1 == 'saveReceiving'), isFalse);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'editing one Android share changes neither inbox nor another share',
    (tester) async {
      final backend = AndroidSharesBackend();
      await mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      await tester.tap(find.byTooltip('Folder and access for checkpoints'));
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('share-selected-folder')),
          matching: find.text('Training checkpoints'),
        ),
        findsOneWidget,
      );
      expect(find.textContaining('opaque-checkpoints'), findsNothing);
      expect(find.byKey(const ValueKey('share-root')), findsNothing);
      await tester.ensureVisible(find.text('My laptop'));
      await tester.tap(find.text('My laptop'));
      await tester.ensureVisible(find.text('Other device'));
      await tester.tap(find.text('Other device'));
      await tester.ensureVisible(find.text('Save read-only share'));
      await tester.tap(find.text('Save read-only share'));
      await tester.pumpAndSettle();
      expect(backend.calls.where((call) => call.$1 == 'saveShare').single.$2, {
        'expected_revision': 3,
        'id': 'checkpoints',
        'folder': 'opaque-checkpoints',
        'allow': [other],
      });
      expect(
        backend.shares.singleWhere(
          (share) => share['id'] == 'datasets',
        )['folder'],
        'opaque-datasets',
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'expired Android folder access requires reselection and displays folder errors accurately',
    (tester) async {
      final backend = AndroidSharesBackend();
      backend.shares.first['available'] = false;
      backend.shares.first['enabled'] = false;
      await mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      final row = find.byKey(const ValueKey('android-share-checkpoints'));
      expect(tester.widget<WispServiceItem>(row).onToggle, isNull);
      await tester.tap(find.byTooltip('Folder and access for checkpoints'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Folder access expired'), findsWidgets);
      expect(
        tester
            .widget<FilledButton>(
              find.widgetWithText(FilledButton, 'Save read-only share'),
            )
            .onPressed,
        isNull,
      );
      backend.selection = {};
      await tester.ensureVisible(find.text('Choose shared folder'));
      await tester.tap(find.text('Choose shared folder'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<FilledButton>(
              find.widgetWithText(FilledButton, 'Save read-only share'),
            )
            .onPressed,
        isNull,
      );
      backend.pickerError = PlatformException(
        code: 'folder',
        message: 'Folder access expired. Select the folder again.',
      );
      await tester.tap(find.text('Choose shared folder'));
      await tester.pumpAndSettle();
      expect(
        find.text('Folder access expired. Select the folder again.'),
        findsOneWidget,
      );
      expect(find.textContaining('Pairing code expired'), findsNothing);
      backend.pickerError = null;
      backend.selection = {
        'folder': 'opaque-new-folder',
        'label': 'Reselected checkpoints',
      };
      await tester.tap(find.text('Choose shared folder'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Save read-only share'));
      await tester.tap(find.text('Save read-only share'));
      await tester.pumpAndSettle();
      expect(
        backend.shares.singleWhere(
          (share) => share['id'] == 'checkpoints',
        )['folder'],
        'opaque-new-folder',
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Android share removal confirms and affects only the chosen service',
    (tester) async {
      final backend = AndroidSharesBackend();
      await mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      await tester.tap(find.byTooltip('Folder and access for checkpoints'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Remove share'));
      await tester.tap(find.text('Remove share'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Keep share'));
      await tester.pumpAndSettle();
      expect(backend.calls.where((call) => call.$1 == 'removeShare'), isEmpty);
      await tester.ensureVisible(find.text('Remove share'));
      await tester.tap(find.text('Remove share'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('confirm-remove-share')));
      await tester.pumpAndSettle();
      expect(
        backend.calls.where((call) => call.$1 == 'removeShare').single.$2,
        {'expected_revision': 3, 'id': 'checkpoints'},
      );
      expect(backend.shares.single['id'], 'datasets');
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Android source enable control targets its exact service incarnation',
    (tester) async {
      final backend = AndroidSharesBackend();
      await mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      await tester.tap(find.text('checkpoints'));
      await tester.pumpAndSettle();
      expect(
        backend.calls.where((call) => call.$1 == 'setServiceEnabled').single.$2,
        {
          'service': 'checkpoints',
          'incarnation': incarnation,
          'enabled': false,
        },
      );
      expect(backend.shares.last['enabled'], isTrue);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Android local share gear opens share management instead of inbox settings',
    (tester) async {
      final backend = AndroidSharesBackend();
      tester.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(disableAnimations: true);
      addTearDown(
        tester.platformDispatcher.clearAccessibilityFeaturesTestValue,
      );
      await tester.pumpWidget(
        MediaQuery(
          data: const MediaQueryData(
            size: Size(800, 600),
            disableAnimations: true,
          ),
          child: WispnetApp(backend: backend),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.byTooltip('Settings for checkpoints on Android phone'),
      );
      await tester.pumpAndSettle();
      expect(find.byType(AndroidFileShares), findsOneWidget);
      expect(find.byType(AndroidReceivingSettings), findsNothing);
      expect(find.text('Datasets folder'), findsNothing);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Android owner folder shares fit a phone and show no private paths',
    (tester) async {
      tester.view.physicalSize = const Size(360, 780);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final backend = AndroidSharesBackend();
      await mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      await capture(tester, 'android-owner-folder-shares');
      await tester.tap(find.byTooltip('Folder and access for checkpoints'));
      await tester.pumpAndSettle();
      await capture(tester, 'android-owner-folder-editor');
      expect(find.byKey(const ValueKey('share-root')), findsNothing);
      expect(find.textContaining('opaque-'), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
