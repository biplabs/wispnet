import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:desktop_drop/desktop_drop.dart';
// Native drag events use the same test transport seam as the plugin's tests.
// ignore: implementation_imports
import 'package:desktop_drop/src/native_transport.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/file_transfer.dart';
import 'package:wispnet/ui.dart';

final _host = 'a' * 64;
final _incarnation = 'b' * 64;
final _machine = Machine(_host, {
  'name': 'Receiving desktop',
  'state': 'available',
});

class _FilesBackend implements WispnetBackend {
  final calls = <(String, Map<String, dynamic>)>[];
  final states = <String, Map<String, dynamic>>{};
  Object? sendError;
  Completer<Map<String, dynamic>>? statusWait;
  int nextId = 0;

  List<Map<String, dynamic>> get sends => calls
      .where((call) => call.$1 == 'fileSend')
      .map((call) => call.$2)
      .toList();

  Map<String, dynamic> complete(String id) => states[id] = {
    'transfer': id,
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
    calls.add((method, Map.of(arguments)));
    switch (method) {
      case 'snapshot':
        return {
          'desktop': true,
          'name': 'Sending laptop',
          'peer': 'c' * 64,
          'enabled': true,
          'relay': 'connected',
          'network': 'Network available',
          'version': '0.1.0',
          'order': {},
          'hosts': {
            _host: {
              ..._machine.data,
              'catalog': {
                'services': [
                  {
                    'id': 'files',
                    'display_name': 'Files',
                    'kind': 'files',
                    'incarnation': _incarnation,
                  },
                ],
              },
            },
          },
        };
      case 'refreshHost':
      case 'takeLink':
      case 'takeInvitation':
        return {};
      case 'fileSend':
        if (sendError case final Object error) {
          sendError = null;
          throw error;
        }
        final id = (++nextId).toRadixString(16).padLeft(64, '0');
        states[id] = {
          'transfer': id,
          'state': 'running',
          'phase': 'sending',
          'verified_bytes': 128,
          'size_bytes': 1024,
        };
        return Map.of(states[id]!);
      case 'fileStatus':
        if (statusWait != null) return statusWait!.future;
        return Map.of(states[arguments['transfer']]!);
      case 'fileCancel':
        final id = arguments['transfer'] as String;
        states[id] = {
          ...states[id]!,
          'state': 'cancelled',
          'error': {'code': 'CANCELLED', 'message': 'Transfer cancelled'},
        };
        return Map.of(states[id]!);
      default:
        throw StateError('Unexpected desktop operation $method');
    }
  }
}

Future<void> _pumpStatus(WidgetTester tester) async {
  await tester.pump(const Duration(seconds: 1));
  await tester.pump();
}

Future<void> _screenshot(WidgetTester tester, String name) async {
  final directory = Platform.environment['WISPNET_SCREENSHOTS'];
  if (directory == null) return;
  await tester.runAsync(() async {
    final boundary = tester.renderObject<RenderRepaintBoundary>(
      find.byKey(const ValueKey('file-send-capture')),
    );
    final image = await boundary.toImage();
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    Directory(directory).createSync(recursive: true);
    File('$directory/$name.png').writeAsBytesSync(bytes!.buffer.asUint8List());
    image.dispose();
  });
}

Widget _sheet(_FilesBackend backend, DesktopFileTransfers transfers) =>
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: FileSendSheet(
            backend: backend,
            machine: _machine,
            incarnation: _incarnation,
            transfers: transfers,
            initialPaths: const ['/source/weights.bin', '/source/config.json'],
          ),
        ),
      ),
    );

void main() {
  setUpAll(() async {
    final fonts = FontLoader('Roboto')
      ..addFont(rootBundle.load('assets/fonts/Roboto-Regular.ttf'))
      ..addFont(rootBundle.load('assets/fonts/Roboto-Medium.ttf'))
      ..addFont(rootBundle.load('assets/fonts/Roboto-Bold.ttf'));
    await fonts.load();
    await (FontLoader(
      'MaterialIcons',
    )..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
  });

  test(
    'upload speed measures new bytes and expires on a stalled native reply',
    () {
      var elapsed = const Duration(days: 1);
      final job = DesktopFileTransfer(
        machine: _machine,
        incarnation: _incarnation,
        workspace: '',
        path: '/source/model.bin',
        elapsed: () => elapsed,
      );
      void report(int bytes) => job.update({
        'state': 'running',
        'phase': 'transferring',
        'verified_bytes': bytes,
        'size_bytes': 1024 * 1024,
      });
      report(512 * 1024);
      expect(
        job.bytesPerSecond,
        isNull,
        reason: 'A resumed prefix is only a baseline.',
      );
      elapsed += const Duration(seconds: 3);
      expect(
        job.bytesPerSecond,
        0,
        reason: 'A stalled first reply must not measure forever.',
      );
      report(512 * 1024);
      elapsed += const Duration(seconds: 2);
      report(514 * 1024);
      expect(job.bytesPerSecond, 1024);
      report(514 * 1024);
      expect(
        job.bytesPerSecond,
        1024,
        reason: 'Same-time samples cannot divide by zero.',
      );
      elapsed += const Duration(seconds: 1);
      report(514 * 1024);
      expect(
        job.bytesPerSecond,
        0,
        reason: 'A real stall must not retain a moving rate.',
      );
      elapsed += const Duration(seconds: 1);
      report(516 * 1024);
      expect(job.bytesPerSecond, 2048);
      elapsed += const Duration(seconds: 3);
      expect(
        job.bytesPerSecond,
        0,
        reason: 'A delayed native reply expires the previous rate.',
      );
      job.error = 'Waiting for the local device';
      expect(job.bytesPerSecond, isNull);
      job.error = null;
      job.cancelling = true;
      expect(job.bytesPerSecond, isNull);
    },
  );

  test(
    'upload speed resets through preparation, reconnects and terminal states',
    () {
      var elapsed = Duration.zero;
      final job = DesktopFileTransfer(
        machine: _machine,
        incarnation: _incarnation,
        workspace: '',
        path: '/source/model.bin',
        elapsed: () => elapsed,
      );
      var bytes = 0;
      void report(String phase, {String state = 'running'}) {
        elapsed += const Duration(seconds: 1);
        job.update({
          'state': state,
          'phase': phase,
          'verified_bytes': bytes,
          'copied_bytes': 1024 * 1024,
          'size_bytes': 1024 * 1024,
        });
      }

      for (final phase in [
        'preparing',
        'hashing',
        'copying',
        'queued',
        'reconnecting',
        'verifying',
      ]) {
        report('transferring');
        bytes += 256;
        report('transferring');
        expect(job.bytesPerSecond, 256);
        report(phase);
        expect(job.bytesPerSecond, isNull, reason: phase);
        bytes += 4096;
        report('transferring');
        expect(
          job.bytesPerSecond,
          isNull,
          reason: 'A new active phase needs a fresh baseline.',
        );
      }
      for (final state in ['complete', 'failed', 'cancelled']) {
        bytes += 256;
        report('transferring');
        expect(job.bytesPerSecond, 256);
        report('transferring', state: state);
        expect(job.bytesPerSecond, isNull, reason: state);
        report('transferring');
        expect(job.bytesPerSecond, isNull);
      }
      bytes = 0;
      report('transferring');
      expect(
        job.bytesPerSecond,
        isNull,
        reason: 'Counter resets must not produce negative rates.',
      );
      bytes = 128;
      report('transferring');
      expect(job.bytesPerSecond, 128);
    },
  );

  testWidgets(
    'upload progress shows percentage, bytes and a current expiring speed',
    (tester) async {
      var elapsed = Duration.zero;
      final backend = _FilesBackend();
      final transfers = DesktopFileTransfers(backend, elapsed: () => elapsed);
      addTearDown(transfers.dispose);
      final job = transfers
          .enqueue(
            machine: _machine,
            incarnation: _incarnation,
            paths: ['/source/model.bin'],
          )
          .single;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: FileTransferList(transfers: transfers)),
        ),
      );
      await tester.pump();
      expect(find.text('Measuring upload speed…'), findsOneWidget);
      elapsed += const Duration(seconds: 1);
      backend.states[job.transfer!]!['verified_bytes'] = 640;
      await _pumpStatus(tester);
      expect(find.text('62% · 640 B / 1.0 KB'), findsOneWidget);
      expect(find.text('512 B/s'), findsOneWidget);
      final bar = tester.widget<LinearProgressIndicator>(
        find.byType(LinearProgressIndicator),
      );
      expect(bar.value, .625);
      expect(bar.minHeight, 10);
      expect(bar.semanticsValue, '62%');
      backend.statusWait = Completer<Map<String, dynamic>>();
      elapsed += const Duration(seconds: 1);
      await _pumpStatus(tester);
      elapsed += const Duration(seconds: 2);
      await tester.pump(const Duration(seconds: 2));
      expect(find.text('0 B/s'), findsOneWidget);
      expect(find.text('512 B/s'), findsNothing);
      transfers.cancel(job);
      await tester.pump();
      expect(find.text('0 B/s'), findsNothing);
      expect(job.bytesPerSecond, isNull);
      backend.statusWait!.complete(Map.of(backend.states[job.transfer!]!));
      backend.statusWait = null;
      await tester.pump();
      await _pumpStatus(tester);
    },
  );

  testWidgets('native jobs run sequentially and preserve the selected target', (
    tester,
  ) async {
    final backend = _FilesBackend();
    final transfers = DesktopFileTransfers(backend);
    addTearDown(transfers.dispose);
    final jobs = transfers.enqueue(
      machine: _machine,
      incarnation: _incarnation,
      workspace: 'training',
      paths: ['/source/model weights.bin', '/source/配置.json'],
    );
    await tester.pump();

    expect(jobs, hasLength(2));
    expect(backend.sends, [
      {
        'host': _host,
        'incarnation': _incarnation,
        'workspace': 'training',
        'local_path': '/source/model weights.bin',
        'remote_path': 'model weights.bin',
        'resume': true,
      },
    ]);
    expect(jobs.first.verifiedBytes, 128);
    expect(jobs.first.sizeBytes, 1024);
    expect(jobs.last.finished, isFalse);

    backend.complete(jobs.first.transfer!);
    await _pumpStatus(tester);
    expect(backend.sends, hasLength(2));
    expect(backend.sends.last['remote_path'], '配置.json');
    expect(jobs.first.state, 'complete');
    expect(jobs.first.verifiedBytes, 1024);

    backend.complete(jobs.last.transfer!);
    await _pumpStatus(tester);
    await Future.wait(jobs.map((job) => job.done));
    expect(transfers.jobs.every((job) => job.finished), isTrue);
  });

  testWidgets('failed sends remain visible and retry with resumable transfer', (
    tester,
  ) async {
    final backend = _FilesBackend()
      ..sendError = PlatformException(
        code: 'permission_denied',
        message: 'Files access was revoked',
      );
    final transfers = DesktopFileTransfers(backend);
    addTearDown(transfers.dispose);
    final first = transfers
        .enqueue(
          machine: _machine,
          incarnation: _incarnation,
          workspace: 'incoming',
          paths: ['/source/weights.bin'],
        )
        .single;
    await tester.pump();
    expect(first.state, 'failed');
    expect(first.error, contains('revoked'));
    await first.done;

    transfers.retry(first);
    await tester.pump();
    expect(backend.sends, hasLength(2));
    expect(backend.sends.last['resume'], isTrue);
    expect(backend.sends.last['local_path'], '/source/weights.bin');
    final running = transfers.jobs.firstWhere((job) => !job.finished);
    backend.complete(running.transfer!);
    await _pumpStatus(tester);
    expect(transfers.jobs.any((job) => job.state == 'complete'), isTrue);
  });

  testWidgets('cancelling a running transfer advances the remaining queue', (
    tester,
  ) async {
    final backend = _FilesBackend();
    final transfers = DesktopFileTransfers(backend);
    addTearDown(transfers.dispose);
    final jobs = transfers.enqueue(
      machine: _machine,
      incarnation: _incarnation,
      workspace: 'incoming',
      paths: ['/source/first.bin', '/source/second.bin'],
    );
    await tester.pump();
    final cancelledId = jobs.first.transfer;
    transfers.cancel(jobs.first);
    await _pumpStatus(tester);

    expect(backend.calls.where((call) => call.$1 == 'fileCancel').single.$2, {
      'transfer': cancelledId,
    });
    expect(jobs.first.state, 'cancelled');
    expect(backend.sends, hasLength(2));
    expect(backend.sends.last['remote_path'], 'second.bin');
    backend.complete(jobs.last.transfer!);
    await _pumpStatus(tester);
  });

  testWidgets('an expired resume token restarts as a fresh native transfer', (
    tester,
  ) async {
    final backend = _FilesBackend();
    final transfers = DesktopFileTransfers(backend);
    addTearDown(transfers.dispose);
    final expired = transfers
        .enqueue(
          machine: _machine,
          incarnation: _incarnation,
          workspace: 'incoming',
          paths: ['/source/weights.bin'],
        )
        .single;
    await tester.pump();
    backend.states[expired.transfer!] = {
      ...backend.states[expired.transfer]!,
      'state': 'failed',
      'error': {
        'code': 'TRANSFER_EXPIRED',
        'message': 'The previous partial transfer has expired',
      },
    };
    await _pumpStatus(tester);
    expect(expired.state, 'failed');
    transfers.retry(expired);
    await tester.pump();
    expect(backend.sends, hasLength(2));
    expect(backend.sends.last['resume'], isFalse);
    final restarted = transfers.jobs.last;
    backend.complete(restarted.transfer!);
    await _pumpStatus(tester);
    expect(restarted.state, 'complete');
  });

  testWidgets('send sheet submits files without discovering a destination', (
    tester,
  ) async {
    final backend = _FilesBackend();
    final transfers = DesktopFileTransfers(backend);
    addTearDown(transfers.dispose);
    await tester.pumpWidget(_sheet(backend, transfers));
    await tester.pump();
    expect(backend.sends, isEmpty);
    expect(backend.calls, isEmpty);
    expect(find.text('weights.bin'), findsOneWidget);
    expect(find.text('config.json'), findsOneWidget);
    expect(find.byType(DropdownButtonFormField<String>), findsNothing);
    expect(find.byKey(const ValueKey('file-workspace')), findsNothing);
    expect(find.byIcon(Icons.settings_outlined), findsNothing);
    await tester.tap(find.byKey(const ValueKey('file-send')));
    await tester.pump();
    expect(transfers.jobs, hasLength(2));
    expect(backend.sends.single['workspace'], '');

    // Dismissing the sheet must keep its in-flight native transfer alive.
    await tester.pumpWidget(const MaterialApp(home: SizedBox()));
    expect(backend.calls.where((call) => call.$1 == 'fileCancel'), isEmpty);
    backend.complete(transfers.jobs.first.transfer!);
    await _pumpStatus(tester);
    expect(backend.sends, hasLength(2));
    backend.complete(transfers.jobs.last.transfer!);
    await _pumpStatus(tester);
  });

  testWidgets('send panel and restored transfers hide destination details', (
    tester,
  ) async {
    final backend = _FilesBackend();
    final transfers = DesktopFileTransfers(backend);
    addTearDown(transfers.dispose);
    transfers.jobs.add(
      DesktopFileTransfer(
        machine: _machine,
        incarnation: _incarnation,
        workspace: 'private-receiving-name',
        path: '/source/old-file.bin',
      )..state = 'complete',
    );
    await tester.pumpWidget(
      RepaintBoundary(
        key: const ValueKey('file-send-capture'),
        child: MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: wispTheme(),
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () => showWispSheet<void>(
                  context,
                  title: 'Send files to Receiving desktop',
                  child: FileSendSheet(
                    backend: backend,
                    machine: _machine,
                    incarnation: _incarnation,
                    transfers: transfers,
                  ),
                ),
                child: const Text('Open send sheet'),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open send sheet'));
    await tester.pumpAndSettle();
    expect(find.text('Receiving folder'), findsNothing);
    expect(find.text('private-receiving-name'), findsNothing);
    expect(find.byType(TextField), findsNothing);
    expect(find.byIcon(Icons.settings_outlined), findsNothing);
    expect(backend.calls, isEmpty);
    await _screenshot(tester, 'file-send-private-destination');
  });

  testWidgets('a file drop keeps its access lease until processing finishes', (
    tester,
  ) async {
    final temporary = Directory.systemTemp.createTempSync('wispnet-drop-test-');
    addTearDown(() => temporary.deleteSync(recursive: true));
    final file = File('${temporary.path}/weights.bin')
      ..writeAsStringSync('test');
    late Completer<void> processed;
    late Completer<List<String>> received;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: FileDropArea(
            enabled: true,
            onFiles: (files) async {
              received.complete(files);
              await processed.future;
            },
            child: const SizedBox(width: 200, height: 100),
          ),
        ),
      ),
    );
    var released = false;
    final target = tester.widget<DropTarget>(find.byType(DropTarget));
    await tester.runAsync(() async {
      processed = Completer<void>();
      received = Completer<List<String>>();
      final drop = Future.sync(
        () => target.onDrop(
          DropDetails(
            items: [
              DropFileItem.fromPath(file.path),
              const DropStringItem(
                mimeType: 'text/uri-list',
                value: 'https://example.invalid/weights.bin',
              ),
            ],
            localPosition: Offset.zero,
            globalPosition: Offset.zero,
            operation: DropOperation.copy,
          ),
        ),
      ).whenComplete(() => released = true);
      expect(await received.future, [file.path]);
      expect(released, isFalse);
      processed.complete();
      await drop;
      expect(released, isTrue);
    });
  });

  testWidgets('directories and symlinks are rejected without queuing files', (
    tester,
  ) async {
    final temporary = Directory.systemTemp.createTempSync('wispnet-drop-test-');
    addTearDown(() => temporary.deleteSync(recursive: true));
    final source = File('${temporary.path}/source.bin')
      ..writeAsStringSync('test');
    final link = Link('${temporary.path}/shortcut.bin')
      ..createSync(source.path);
    var sends = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: FileDropArea(
            onFiles: (_) async => sends++,
            child: const SizedBox(width: 200, height: 100),
          ),
        ),
      ),
    );
    final target = tester.widget<DropTarget>(find.byType(DropTarget));
    for (final item in <DropItem>[
      DropDirectoryItem(
        directory: XDirectory.fileSystem(path: temporary.path),
        name: 'folder',
        localPath: temporary.path,
      ),
      DropFileItem.fromPath(link.path),
    ]) {
      await tester.runAsync(
        () async => target.onDrop(
          DropDetails(
            items: [item],
            localPosition: Offset.zero,
            globalPosition: Offset.zero,
            operation: DropOperation.copy,
          ),
        ),
      );
      await tester.pump();
    }
    expect(sends, 0);
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'dropping into the compact desktop pop-up selects a receiver and survives closing its sheet',
    (tester) async {
      final temporary = Directory.systemTemp.createTempSync(
        'wispnet-popup-drop-test-',
      );
      addTearDown(() => temporary.deleteSync(recursive: true));
      final file = File('${temporary.path}/model weights.bin')
        ..writeAsStringSync('native drop fixture');
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 600);
      tester.platformDispatcher.accessibilityFeaturesTestValue =
          FakeAccessibilityFeatures(disableAnimations: true);
      addTearDown(() {
        tester.view.resetDevicePixelRatio();
        tester.view.resetPhysicalSize();
        tester.platformDispatcher.clearAccessibilityFeaturesTestValue();
      });
      final backend = _FilesBackend();
      await tester.pumpWidget(WispnetApp(backend: backend));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Receiving desktop · aaaaaaaa'), findsOneWidget);

      // Send an OS-style event through the plugin's real hit-testing and event
      // delivery seam. The left margin hits the whole pop-up's parent target.
      final transport = NativeTransport.debugOverride!;
      final event = <String, Object?>{
        'view': tester.view.viewId,
        'generation': 1,
        'session': 9101,
        'request': 1,
        'phase': 1,
        'x': 8.0,
        'y': 180.0,
        'offer': {
          'allowedOperations': ['copy'],
          'isComplete': true,
          'strings': [],
          'files': [
            {'name': 'model weights.bin', 'path': file.path},
          ],
        },
      };
      final selection =
          await transport.onEvent!(MethodCall('query', event)) as Map;
      expect(selection['target'], greaterThan(0));
      expect(selection['operation'], 1);
      var released = false;
      late Future<Object?> drop;
      await tester.runAsync(() async {
        drop = transport
            .onEvent!(
              MethodCall('drop', {
                ...event,
                'target': selection['target'],
                'operation': selection['operation'],
              }),
            )
            .whenComplete(() => released = true);
      });
      for (
        var attempt = 0;
        attempt < 10 && find.byType(FileSendSheet).evaluate().isEmpty;
        attempt++
      ) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 10)),
        );
        await tester.pump();
      }
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Send files to Receiving desktop'), findsOneWidget);
      expect(find.text('model weights.bin'), findsOneWidget);
      expect(backend.sends, isEmpty);

      await tester.ensureVisible(find.byKey(const ValueKey('file-send')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('file-send')));
      await tester.pump();
      expect(backend.sends.single, {
        'host': _host,
        'incarnation': _incarnation,
        'workspace': '',
        'local_path': file.path,
        'remote_path': 'model weights.bin',
        'resume': true,
      });
      await tester.ensureVisible(find.byTooltip('Close'));
      await tester.pump();
      await tester.tap(find.byTooltip('Close'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byType(FileSendSheet), findsNothing);
      expect(find.byKey(const ValueKey('file-transfers-dock')), findsOneWidget);
      expect(released, isFalse);
      expect(backend.calls.where((call) => call.$1 == 'fileCancel'), isEmpty);

      backend.complete(backend.states.keys.single);
      await _pumpStatus(tester);
      // The OS callback runs outside Flutter's fake clock. Alternate real
      // microtasks and framework pumps so neither side waits on a frozen zone.
      for (var attempt = 0; attempt < 30 && !released; attempt++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)),
        );
        await tester.pump(const Duration(milliseconds: 50));
      }
      expect(
        released,
        isTrue,
        reason: 'The completed native job releases its drop',
      );
      final accepted = await tester.runAsync(
        () => drop.timeout(const Duration(seconds: 5)),
      );
      expect(accepted, isTrue);
      expect(released, isTrue);
      expect(find.byKey(const ValueKey('file-transfers-dock')), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
    },
  );
  testWidgets(
    'CLI examples copy only the destination prefix without file placeholders',
    (tester) async {
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
      for (final service in ['files', 'checkpoints-incoming']) {
        await tester.pumpWidget(
          MaterialApp(
            theme: wispTheme(),
            home: Scaffold(
              body: FileSendCommand(host: _host, service: service),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final prefix =
            "wispnet files put $_host ${service == 'files' ? "''" : service}";
        expect(find.text('$prefix /local/file filename'), findsOneWidget);
        await tester.tap(find.byTooltip('Copy send command'));
        await tester.pumpAndSettle();
        expect(copied, prefix);
        expect(copied, isNot(contains('/local/file')));
        await tester.pumpWidget(const SizedBox());
      }
    },
  );
}
