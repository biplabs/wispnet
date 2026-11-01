import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/android_file_shares.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/desktop_settings.dart';
import 'package:wispnet/src/file_transfer.dart';
import 'package:wispnet/ui.dart';

import 'android_file_shares_test.dart' as fixture;

class FolderBackend extends fixture.AndroidSharesBackend {
  bool desktop = false, remote = false;
  String pickedDesktopFolder = '/chosen/inbox';
  final inboxes = <Map<String, dynamic>>[
    {
      'id': 'training-data',
      'folder': 'opaque-training',
      'label': 'Training data',
      'allow': [fixture.laptop],
      'enabled': true,
      'available': true,
      'incarnation': '1' * 64,
      'kind': 'file_inbox',
    },
    {
      'id': 'predictions',
      'folder': 'opaque-predictions',
      'label': 'Predictions',
      'allow': [fixture.other],
      'enabled': true,
      'available': true,
      'incarnation': '2' * 64,
      'kind': 'file_inbox',
    },
  ];
  List<Map<String, dynamic>> restoredTransfers = [];
  final transferStatus = {
    'transfer': '9' * 64,
    'state': 'complete',
    'phase': 'complete',
    'verified_bytes': 1024,
    'size_bytes': 1024,
  };

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    switch (method) {
      case 'folderServicesStatus':
        final response = await super.invoke(method, arguments);
        return {
          ...response,
          'inboxes': inboxes.map(Map<String, dynamic>.of).toList(),
        };
      case 'pickReceivingFolder':
        calls.add((method, Map.of(arguments)));
        return {'folder': 'opaque-receive', 'label': 'Receiving folder'};
      case 'saveInbox':
        calls.add((method, Map.of(arguments)));
        expect(arguments['expected_revision'], revision);
        final existing = inboxes
            .where((inbox) => inbox['id'] == arguments['id'])
            .firstOrNull;
        inboxes.removeWhere((inbox) => inbox['id'] == arguments['id']);
        inboxes.add({
          ...?existing,
          'id': arguments['id'],
          'kind': 'file_inbox',
          'folder': arguments['folder'],
          'label': existing?['label'] ?? 'Receiving folder',
          'allow': arguments['allow'],
          'enabled': true,
          'available': true,
          'incarnation': '3' * 64,
        });
        return {'revision': ++revision};
      case 'removeInbox':
        calls.add((method, Map.of(arguments)));
        expect(arguments['expected_revision'], revision);
        inboxes.removeWhere((inbox) => inbox['id'] == arguments['id']);
        return {'revision': ++revision};
      case 'setServiceEnabled':
        if (inboxes.any((inbox) => inbox['id'] == arguments['service'])) {
          calls.add((method, Map.of(arguments)));
          inboxes.singleWhere(
            (inbox) => inbox['id'] == arguments['service'],
          )['enabled'] = arguments['enabled'];
          revision++;
          return {};
        }
        return super.invoke(method, arguments);
      case 'desktopStatus':
        calls.add((method, Map.of(arguments)));
        return {
          'host': {
            'host': fixture.own,
            'revision': revision,
            'services': [
              {
                'id': 'files',
                'kind': 'files',
                'allow': [],
                'incarnation': 'f' * 64,
              },
              ...inboxes,
              for (final share in shares) {...share, 'kind': 'file_share'},
            ],
            'clients': [
              {'peer': fixture.laptop, 'name': 'My laptop'},
              {'peer': fixture.other, 'name': 'Other device'},
            ],
            'file_exports': {},
            'file_inboxes': {
              for (final inbox in inboxes) inbox['id']: '/srv/${inbox['id']}',
            },
            'file_shares': {
              for (final share in shares) share['id']: '/srv/${share['id']}',
            },
          },
          'invitations': [],
        };
      case 'desktopData':
        calls.add((method, Map.of(arguments)));
        return {'value': []};
      case 'desktopRequest':
        calls.add((method, Map.of(arguments)));
        final operation = (arguments['payload'] as Map)['operation'] as Map;
        expect(operation['expected_revision'], revision);
        return {'revision': ++revision};
      case 'pickFolder':
        calls.add((method, Map.of(arguments)));
        return {'path': pickedDesktopFolder};
      case 'snapshot':
        final snapshot = await super.invoke(method, arguments);
        return {
          ...snapshot,
          'desktop': desktop,
          'hosts': {
            if (!remote)
              fixture.own: {
                'name': 'Android phone',
                'local': true,
                'state': 'available',
                'catalog': {
                  'host': fixture.own,
                  'services': [
                    ...inboxes,
                    for (final share in shares)
                      {...share, 'kind': 'file_share'},
                  ],
                },
              },
            if (remote)
              fixture.own: {
                'name': 'Training host',
                'state': 'available',
                'catalog': {
                  'host': fixture.own,
                  'services': [
                    ...inboxes,
                    for (final share in shares)
                      {...share, 'kind': 'file_share'},
                  ],
                },
              },
          },
        };
      case 'fileSend':
      case 'fileStatus':
      case 'fileCancel':
        calls.add((method, Map.of(arguments)));
        return Map.of(transferStatus);
      case 'fileList':
        calls.add((method, Map.of(arguments)));
        return {'transfers': restoredTransfers};
      case 'pickFiles':
        calls.add((method, Map.of(arguments)));
        return {
          'files': [
            {
              'path': 'content://documents/checkpoint',
              'name': 'checkpoint.bin',
              'size_bytes': 1024,
            },
          ],
        };
      default:
        return super.invoke(method, arguments);
    }
  }
}

Future<void> chooseMode(WidgetTester tester, String label) async {
  await tester.ensureVisible(find.byKey(const ValueKey('folder-mode')));
  await tester.tap(find.byKey(const ValueKey('folder-mode')));
  await tester.pumpAndSettle();
  await tester.tap(find.text(label).last);
  await tester.pumpAndSettle();
}

Future<void> app(WidgetTester tester, FolderBackend backend) async {
  tester.platformDispatcher.accessibilityFeaturesTestValue =
      const FakeAccessibilityFeatures(disableAnimations: true);
  addTearDown(tester.platformDispatcher.clearAccessibilityFeaturesTestValue);
  await tester.pumpWidget(
    MediaQuery(
      data: const MediaQueryData(size: Size(800, 600), disableAnimations: true),
      child: WispnetApp(backend: backend),
    ),
  );
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

  testWidgets(
    'desktop Receive files tab creates a named Inbox with explicit per-folder grants',
    (tester) async {
      final backend = FolderBackend();
      await fixture.mount(
        tester,
        DesktopSettings(
          backend: backend,
          servicesOnly: true,
          onAccess: () {},
          onChanged: () {},
        ),
      );
      expect(find.widgetWithText(Tab, 'Receive files'), findsNothing);
      expect(find.text('Add read-only file share'), findsNothing);
      expect(find.text('Receive files'), findsNWidgets(2));
      await tester.tap(find.byKey(const ValueKey('share-service')));
      await tester.pumpAndSettle();
      expect(find.widgetWithText(Tab, 'Receive files'), findsOneWidget);
      await tester.enterText(
        find.byKey(const ValueKey('share-name')),
        'checkpoints-incoming',
      );
      backend.pickedDesktopFolder = '/srv/checkpoints-incoming';
      await tester.tap(find.text('Choose folder'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Other device').last);
      await tester.tap(find.text('Other device').last);
      await tester.ensureVisible(find.text('Save'));
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      final operation = backend.calls
          .where((call) => call.$1 == 'desktopRequest')
          .single
          .$2['payload']['operation'];
      expect(operation, {
        'op': 'file_inbox',
        'id': 'checkpoints-incoming',
        'root': '/srv/checkpoints-incoming',
        'allow': [fixture.other],
        'expected_revision': 3,
      });
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Android switching folder mode clears the previous picker capability',
    (tester) async {
      final backend = FolderBackend();
      await fixture.mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      await tester.tap(find.byKey(const ValueKey('add-android-share')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('share-name')),
        'results',
      );
      await tester.tap(find.text('Choose folder'));
      await tester.pumpAndSettle();
      expect(find.text('Receiving folder'), findsOneWidget);
      await tester.ensureVisible(find.text('My laptop'));
      await tester.tap(find.text('My laptop'));
      await chooseMode(tester, 'Share files');
      expect(find.text('Receiving folder'), findsNothing);
      expect(find.byKey(const ValueKey('share-selected-folder')), findsNothing);
      expect(
        tester
            .widget<WispToggleListTile>(
              find.byKey(ValueKey('share-allow-${fixture.laptop}')),
            )
            .value,
        isFalse,
      );
      expect(
        tester
            .widget<FilledButton>(find.widgetWithText(FilledButton, 'Save'))
            .onPressed,
        isNull,
      );
      await tester.tap(find.text('Choose folder'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Save'));
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(
        backend.calls.where((call) => call.$1 == 'pickReceivingFolder'),
        hasLength(1),
      );
      expect(
        backend.calls.where((call) => call.$1 == 'pickShareFolder'),
        hasLength(1),
      );
      expect(backend.calls.where((call) => call.$1 == 'saveInbox'), isEmpty);
      expect(
        backend.calls
            .where((call) => call.$1 == 'saveShare')
            .single
            .$2['folder'],
        'opaque-selected-folder',
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'editing an Android inbox keeps its mode and other inbox permissions intact',
    (tester) async {
      final backend = FolderBackend();
      await fixture.mount(
        tester,
        AndroidFileShares(backend: backend, onChanged: () {}),
      );
      await tester.tap(find.byTooltip('Folder and access for predictions'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('folder-mode')), findsNothing);
      expect(find.text('Permission'), findsOneWidget);
      await tester.ensureVisible(find.text('Other device'));
      await tester.tap(find.text('Other device'));
      await tester.ensureVisible(find.text('My laptop'));
      await tester.tap(find.text('My laptop'));
      await tester.ensureVisible(find.text('Save'));
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(backend.calls.where((call) => call.$1 == 'saveInbox').single.$2, {
        'expected_revision': 3,
        'id': 'predictions',
        'folder': 'opaque-predictions',
        'allow': [fixture.laptop],
      });
      expect(
        backend.inboxes.singleWhere(
          (inbox) => inbox['id'] == 'training-data',
        )['allow'],
        [fixture.laptop],
      );
      expect(
        backend.calls.where(
          (call) => call.$1 == 'saveShare' || call.$1 == 'saveReceiving',
        ),
        isEmpty,
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('an explicitly configured legacy inbox remains editable', (
    tester,
  ) async {
    final backend = FolderBackend();
    backend.inboxes.first['id'] = 'files';
    backend.inboxes.first['kind'] = 'files';
    await fixture.mount(
      tester,
      AndroidFileShares(backend: backend, onChanged: () {}),
    );
    await tester.tap(find.byTooltip('Folder and access for files'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('Save'));
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(
      backend.calls.where((call) => call.$1 == 'saveInbox').single.$2['id'],
      'files',
    );
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('remote Inbox card sends files to the selected named service', (
    tester,
  ) async {
    final backend = FolderBackend()..remote = true;
    await app(tester, backend);
    expect(find.text('Receive files'), findsNWidgets(2));
    await tester.tap(find.text('predictions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose files'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('Send files'));
    await tester.tap(find.text('Send files'));
    await tester.pumpAndSettle();
    final send = backend.calls.where((call) => call.$1 == 'fileSend').single.$2;
    expect(send['service'], 'predictions');
    expect(send['incarnation'], '2' * 64);
    expect(send['workspace'], '');
    expect(
      backend.calls.any(
        (call) => call.$1 == 'saveInbox' || call.$1 == 'saveShare',
      ),
      isFalse,
    );
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets(
    'desktop drop prompts for an inbox instead of selecting the first folder',
    (tester) async {
      final backend = FolderBackend()
        ..remote = true
        ..desktop = true;
      await app(tester, backend);
      final drop = tester
          .widget<FileDropArea>(find.byType(FileDropArea).first)
          .onFiles(['/source/model.bin']);
      unawaited(drop);
      await tester.pumpAndSettle();
      expect(find.text('Choose inbox on Training host'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('choose-inbox-predictions')));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Send files'));
      await tester.tap(find.text('Send files'));
      await tester.pumpAndSettle();
      expect(
        backend.calls
            .where((call) => call.$1 == 'fileSend')
            .single
            .$2['service'],
        'predictions',
      );
      await tester.ensureVisible(find.byTooltip('Close'));
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      await drop;
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('restored and retried uploads retain their named inbox service', (
    tester,
  ) async {
    final backend = FolderBackend();
    backend.restoredTransfers = [
      {
        'transfer': '8' * 64,
        'host': fixture.own,
        'service': 'predictions',
        'incarnation': '2' * 64,
        'workspace': '',
        'local_path': 'content://file',
        'remote_path': 'checkpoint.bin',
        'state': 'failed',
        'size_bytes': 1024,
      },
    ];
    final transfers = DesktopFileTransfers(backend);
    await transfers.restore({
      fixture.own: Machine(fixture.own, {'name': 'Training host'}),
    });
    expect(transfers.jobs.single.service, 'predictions');
    transfers.retry(transfers.jobs.single);
    await tester.pump();
    expect(
      backend.calls.where((call) => call.$1 == 'fileSend').single.$2['service'],
      'predictions',
    );
    transfers.dispose();
  });

  testWidgets('unified named folders and setup fit phone width', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(360, 780);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final backend = FolderBackend();
    await fixture.mount(
      tester,
      AndroidFileShares(backend: backend, onChanged: () {}),
    );
    await fixture.capture(tester, 'unified-folder-services');
    await tester.tap(find.byKey(const ValueKey('add-android-share')));
    await tester.pumpAndSettle();
    await fixture.capture(tester, 'unified-add-folder');
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
    tester.platformDispatcher.accessibilityFeaturesTestValue =
        const FakeAccessibilityFeatures(disableAnimations: true);
    addTearDown(tester.platformDispatcher.clearAccessibilityFeaturesTestValue);
    final remote = FolderBackend()..remote = true;
    await tester.pumpWidget(
      RepaintBoundary(
        key: const ValueKey('android-share-preview'),
        child: MediaQuery(
          data: const MediaQueryData(
            size: Size(360, 780),
            disableAnimations: true,
          ),
          child: WispnetApp(backend: remote),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await fixture.capture(tester, 'unified-remote-folders');
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}
