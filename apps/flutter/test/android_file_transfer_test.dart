import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/app.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/file_transfer.dart';

final _host = 'a' * 64;
final _incarnation = 'b' * 64;
final _machine = Machine(_host, {
  'name': 'Training desktop',
  'state': 'available',
});
const _sources = [
  {
    'path': 'content://documents/document/primary%3A123',
    'name': 'model weights.bin',
    'size_bytes': 2048,
  },
  {
    'path': 'content://documents/document/primary%3A456',
    'name': '配置.json',
    'size_bytes': 1024,
  },
  {
    'path': 'content://documents/document/primary%3A789',
    'name': 'training.csv',
    'size_bytes': 4096,
  },
];

class _AndroidFilesBackend implements WispnetBackend {
  final calls = <(String, Map<String, dynamic>)>[];
  final states = <String, Map<String, dynamic>>{};
  List<Map<String, Object>> picked = _sources;
  bool enabled = true;
  Object? pickerError;
  int nextId = 0;

  List<Map<String, dynamic>> get sends => calls
      .where((call) => call.$1 == 'fileSend')
      .map((call) => call.$2)
      .toList();

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, Map.of(arguments)));
    switch (method) {
      case 'snapshot':
        return {
          'name': 'Phone',
          'peer': 'c' * 64,
          'enabled': enabled,
          'network': 'Network available',
          'relay': 'connected',
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
      case 'pickFiles':
        if (pickerError != null) throw pickerError!;
        return {'files': picked};
      case 'fileList':
        return {'transfers': states.values.map(Map.of).toList()};
      case 'fileSend':
        if (!enabled) {
          throw PlatformException(
            code: 'offline',
            message: 'Go online before sending files',
          );
        }
        final id = (++nextId).toRadixString(16).padLeft(64, '0');
        states[id] = {
          ...arguments,
          'transfer': id,
          'name': arguments['remote_path'],
          'state': 'running',
          'phase': nextId > 2 ? 'queued' : 'transferring',
          'verified_bytes': 128,
          'size_bytes': 1024,
        };
        return Map.of(states[id]!);
      case 'fileStatus':
        return Map.of(states[arguments['transfer']]!);
      case 'fileCancel':
        final id = arguments['transfer'] as String;
        states[id] = {
          ...states[id]!,
          'state': 'cancelled',
          'error': {'code': 'CANCELLED', 'message': 'Transfer cancelled'},
        };
        return Map.of(states[id]!);
      case 'refreshHost':
      case 'takeInvitation':
      case 'takeLink':
        return {};
      default:
        throw StateError('Unexpected Android operation $method');
    }
  }
}

Widget _sheet(_AndroidFilesBackend backend, DesktopFileTransfers transfers) =>
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: FileSendSheet(
            backend: backend,
            machine: _machine,
            incarnation: _incarnation,
            transfers: transfers,
            desktop: false,
          ),
        ),
      ),
    );

void main() {
  testWidgets(
    'Android chooses multiple documents with their real names and cancels one transfer',
    (tester) async {
      final backend = _AndroidFilesBackend();
      final transfers = DesktopFileTransfers(backend)..nativeQueue = true;
      addTearDown(transfers.dispose);
      await tester.pumpWidget(_sheet(backend, transfers));
      await tester.pump();
      expect(find.byType(FileDropArea), findsNothing);
      expect(find.byType(DropdownButtonFormField<String>), findsNothing);
      expect(find.byKey(const ValueKey('file-workspace')), findsNothing);
      expect(backend.calls, isEmpty);
      await tester.tap(find.byKey(const ValueKey('file-pick')));
      await tester.pump();
      expect(find.text('model weights.bin'), findsOneWidget);
      expect(find.text('配置.json'), findsOneWidget);
      expect(find.text('training.csv'), findsOneWidget);
      await tester.ensureVisible(find.byKey(const ValueKey('file-send')));
      await tester.tap(find.byKey(const ValueKey('file-send')));
      await tester.pump();
      expect(backend.sends, hasLength(3));
      expect(backend.sends.map((send) => send['remote_path']), [
        'model weights.bin',
        '配置.json',
        'training.csv',
      ]);
      expect(
        backend.sends.map((send) => send['local_path']),
        _sources.map((file) => file['path']),
      );
      expect(backend.sends.every((send) => send['workspace'] == ''), isTrue);
      expect(
        transfers.jobs.every((job) => job.transfer != null),
        isTrue,
        reason: 'Every selection is handed to the foreground service before the Activity can be recreated.',
      );
      expect(find.byType(LinearProgressIndicator), findsNWidgets(3));
      final first = transfers.jobs.first;
      await tester.ensureVisible(find.byTooltip('Cancel model weights.bin'));
      await tester.tap(find.byTooltip('Cancel model weights.bin'));
      await tester.pump();
      expect(first.state, 'cancelled');
      expect(transfers.jobs.skip(1).every((job) => !job.finished), isTrue);
      expect(backend.calls.where((call) => call.$1 == 'fileCancel').single.$2, {
        'transfer': first.transfer,
      });
      for (final job in transfers.jobs.where((job) => !job.finished)) {
        backend.states[job.transfer!] = {
          ...backend.states[job.transfer]!,
          'state': 'complete',
        };
      }
      await tester.pump(const Duration(seconds: 1));
      await tester.pump();
    },
  );

  testWidgets('Android picker cancellation and errors never queue a send', (
    tester,
  ) async {
    final backend = _AndroidFilesBackend()..picked = [];
    final transfers = DesktopFileTransfers(backend)..nativeQueue = true;
    addTearDown(transfers.dispose);
    await tester.pumpWidget(_sheet(backend, transfers));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('file-pick')));
    await tester.pump();
    expect(
      tester
          .widget<FilledButton>(find.byKey(const ValueKey('file-send')))
          .onPressed,
      isNull,
    );
    expect(backend.sends, isEmpty);
    backend.pickerError = PlatformException(
      code: 'picker_unavailable',
      message: 'Document picker unavailable',
    );
    await tester.tap(find.byKey(const ValueKey('file-pick')));
    await tester.pump();
    expect(find.text('Document picker unavailable'), findsOneWidget);
    expect(backend.sends, isEmpty);
  });

  testWidgets('Android adds files without duplicating already selected URIs', (
    tester,
  ) async {
    final backend = _AndroidFilesBackend()..picked = [_sources.first];
    final transfers = DesktopFileTransfers(backend)..nativeQueue = true;
    addTearDown(transfers.dispose);
    await tester.pumpWidget(_sheet(backend, transfers));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('file-pick')));
    await tester.pump();
    backend.picked = _sources;
    await tester.tap(find.byKey(const ValueKey('file-pick')));
    await tester.pump();
    expect(find.text('model weights.bin'), findsOneWidget);
    expect(find.text('Send 3 files'), findsOneWidget);
  });

  testWidgets('Android restores each native job after the UI is recreated', (
    tester,
  ) async {
    final backend = _AndroidFilesBackend();
    final original = DesktopFileTransfers(backend)..nativeQueue = true;
    original.enqueue(
      machine: _machine,
      incarnation: _incarnation,
      workspace: 'training',
      paths: _sources.map((source) => source['path']! as String).toList(),
      names: {
        for (final source in _sources)
          source['path']! as String: source['name']! as String,
      },
    );
    await tester.pump();
    original.dispose();
    for (final state in backend.states.values) {
      state['verified_bytes'] = 1024 * 1024;
      state['size_bytes'] = 2 * 1024 * 1024;
    }
    var elapsed = const Duration(days: 1);
    final restored = DesktopFileTransfers(backend, elapsed: () => elapsed)
      ..nativeQueue = true;
    addTearDown(restored.dispose);
    await restored.restore({_host: _machine});
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: FileTransferList(transfers: restored)),
      ),
    );
    await tester.pump();
    expect(restored.jobs, hasLength(3));
    expect(restored.jobs.every((job) => job.bytesPerSecond == null), isTrue);
    expect(find.text('Measuring upload speed…'), findsNWidgets(2));
    elapsed += const Duration(seconds: 1);
    backend.states[restored.jobs.first.transfer!]!['verified_bytes'] =
        1024 * 1024 + 1024;
    await tester.pump(const Duration(seconds: 1));
    await tester.pump();
    expect(restored.jobs.first.bytesPerSecond, 1024);
    expect(restored.jobs[1].bytesPerSecond, 0);
    expect(
      restored.jobs.last.bytesPerSecond,
      isNull,
      reason: 'Native queued jobs have no upload rate.',
    );
    expect(find.text('1.0 KB/s'), findsOneWidget);
    expect(restored.jobs.map((job) => job.name), [
      'model weights.bin',
      '配置.json',
      'training.csv',
    ]);
    expect(
      backend.sends,
      hasLength(3),
      reason: 'Restoring must not duplicate native transfers.',
    );
    expect(find.byTooltip('Cancel 配置.json'), findsOneWidget);
    restored.cancel(restored.jobs[1]);
    await tester.pump();
    expect(restored.jobs[1].state, 'cancelled');
    expect(
      backend.calls.where((call) => call.$1 == 'fileCancel'),
      hasLength(1),
    );
    for (final job in restored.jobs.where((job) => !job.finished)) {
      backend.states[job.transfer!] = {
        ...backend.states[job.transfer]!,
        'state': 'complete',
      };
    }
    await tester.pump(const Duration(seconds: 1));
    await tester.pump();
  });

  testWidgets(
    'Android Files opens a native-picker sheet instead of CLI instructions',
    (tester) async {
      final backend = _AndroidFilesBackend();
      await tester.pumpWidget(WispnetApp(backend: backend));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      await tester.tap(find.text('files'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byKey(const ValueKey('file-pick')), findsOneWidget);
      expect(find.textContaining('wispnet files put'), findsNothing);
      expect(find.byType(FileDropArea), findsNothing);
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
    },
  );

  testWidgets('Android Files never starts a picker while offline', (
    tester,
  ) async {
    final backend = _AndroidFilesBackend()..enabled = false;
    await tester.pumpWidget(WispnetApp(backend: backend));
    await tester.pump();
    expect(find.byKey(const ValueKey('file-pick')), findsNothing);
    expect(
      backend.calls.where(
        (call) =>
            call.$1 == 'pickFiles' ||
            call.$1 == 'fileSend' ||
            call.$1 == 'fileList',
      ),
      isEmpty,
    );
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });
}
