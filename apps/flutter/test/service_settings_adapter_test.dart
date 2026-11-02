import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/service_settings_adapter.dart';

import 'android_file_shares_test.dart' show ownerStatusFromFolders;

class SettingsBackend implements WispnetBackend {
  bool embedded = true;
  final calls = <(String, Map<String, dynamic>)>[];
  Object? mutationFailure;
  final desktopStatus = <String, dynamic>{
    'host': {'host': 'desktop-host', 'revision': 12},
    'desktop-only': true,
  };
  final folders = <String, dynamic>{
    'revision': 7,
    'clients': [
      {'peer': 'laptop', 'name': 'Laptop'},
    ],
    'inboxes': [
      {
        'id': 'incoming',
        'folder': 'opaque-incoming',
        'label': 'Incoming files',
        'available': true,
        'enabled': false,
        'incarnation': 'incoming-version',
        'allow': ['laptop'],
      },
      {
        'id': 'files',
        'kind': 'files',
        'folder': 'opaque-legacy',
        'label': 'Legacy folder',
        'available': true,
        'enabled': true,
        'incarnation': 'legacy-version',
        'allow': ['laptop'],
      },
    ],
    'shares': [
      {
        'id': 'checkpoints',
        'folder': 'opaque-checkpoints',
        'label': 'Training checkpoints',
        'available': false,
        'enabled': false,
        'incarnation': 'checkpoints-version',
        'allow': <String>[],
      },
    ],
  };

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, Map.of(arguments)));
    if (method == 'ownerStatus') {
      return embedded
          ? ownerStatusFromFolders(folders, 'phone-host')
          : desktopStatus;
    }
    if (method == 'folderServicesStatus') return folders;
    if (mutationFailure case final Object error) throw error;
    return {};
  }
}

Matcher nativeCall(String method, Map<String, dynamic> arguments) =>
    isA<(String, Map<String, dynamic>)>()
        .having((call) => call.$1, 'method', method)
        .having((call) => call.$2, 'arguments', arguments);

void main() {
  test(
    'desktop status and mutations retain the native owner protocol',
    () async {
      final backend = SettingsBackend()..embedded = false;
      final adapter = ServiceSettingsAdapter(backend: backend);
      expect(await adapter.status(), same(backend.desktopStatus));
      await adapter.mutate({
        'op': 'http',
        'id': 'website',
        'backend': '127.0.0.1:8080',
        'allow': ['laptop'],
        'expected_revision': -1,
      }, 12);
      expect(backend.calls, [
        nativeCall('ownerStatus', {}),
        nativeCall('ownerRequest', {
          'payload': {
            'op': 'mutate',
            'operation': {
              'op': 'http',
              'id': 'website',
              'backend': '127.0.0.1:8080',
              'allow': ['laptop'],
              'expected_revision': 12,
            },
          },
        }),
      ]);
    },
  );

  test(
    'Android folder status has the shared owner shape and native metadata',
    () async {
      final backend = SettingsBackend();
      final adapter = ServiceSettingsAdapter(
        backend: backend,
        android: true,
        host: 'phone-host',
      );
      final status = await adapter.status();
      final host = status['host'] as Map;
      expect(host['host'], 'phone-host');
      expect(host['revision'], 7);
      expect(host['clients'], backend.folders['clients']);
      expect(host['services'], [
        {
          ...(backend.folders['inboxes'] as List)[0] as Map,
          'kind': 'file_inbox',
        },
        (backend.folders['inboxes'] as List)[1],
        {
          ...(backend.folders['shares'] as List)[0] as Map,
          'kind': 'file_share',
        },
      ]);
      expect(host['file_inboxes'], {
        'incoming': 'opaque-incoming',
        'files': 'opaque-legacy',
      });
      expect(host['file_shares'], {'checkpoints': 'opaque-checkpoints'});
      expect(host['file_exports'], {'incoming': 'opaque-legacy'});
      expect(status['invitations'], isEmpty);
      expect(status['connected_peers'], isEmpty);
      expect(status['recent_peers'], isEmpty);
      expect(backend.calls, [nativeCall('ownerStatus', {})]);
      expect(
        (backend.folders['inboxes'] as List).first.containsKey('kind'),
        isFalse,
      );
    },
  );

  for (final (operation, method) in [
    ('file_inbox', 'saveInbox'),
    ('file_share', 'saveShare'),
  ]) {
    test(
      'Android $operation creation preserves the revision and grants',
      () async {
        final backend = SettingsBackend();
        final adapter = ServiceSettingsAdapter(backend: backend, android: true);
        await adapter.status();
        await adapter.mutate({
          'op': operation,
          'id': 'new-folder',
          'root': 'selected-capability',
          'allow': ['laptop'],
        }, 7);
        expect(
          backend.calls.last,
          nativeCall(method, {
            'expected_revision': 7,
            'id': 'new-folder',
            'folder': 'selected-capability',
            'allow': ['laptop'],
          }),
        );
        expect(
          backend.calls.where((call) => call.$1 == 'ownerStatus'),
          hasLength(1),
        );
      },
    );
  }

  for (final (id, method) in [
    ('incoming', 'saveInbox'),
    ('files', 'saveInbox'),
    ('checkpoints', 'saveShare'),
  ]) {
    test('Android editing $id uses its existing folder mode', () async {
      final backend = SettingsBackend();
      final adapter = ServiceSettingsAdapter(backend: backend, android: true);
      await adapter.status();
      await adapter.mutate({
        'op': 'edit_folder',
        'id': id,
        'new_id': id,
        'root': 'replacement-capability',
        'allow': <String>[],
      }, 7);
      expect(
        backend.calls.last,
        nativeCall(method, {
          'expected_revision': 7,
          'id': id,
          'new_id': id,
          'folder': 'replacement-capability',
          'allow': <String>[],
        }),
      );
    });
  }

  for (final (id, method) in [
    ('incoming', 'saveInbox'),
    ('checkpoints', 'saveShare'),
  ]) {
    test('Android renames $id atomically using the original ID', () async {
      final backend = SettingsBackend();
      final adapter = ServiceSettingsAdapter(backend: backend, android: true);
      await adapter.status();
      await adapter.mutate({
        'op': 'edit_folder',
        'id': id,
        'new_id': '$id-renamed',
        'root': 'unchanged-capability',
        'allow': ['laptop'],
      }, 7);
      expect(
        backend.calls.last,
        nativeCall(method, {
          'expected_revision': 7,
          'id': id,
          'new_id': '$id-renamed',
          'folder': 'unchanged-capability',
          'allow': ['laptop'],
        }),
      );
    });
  }

  test('Android legacy Files saves complete exports and clears grants with the last folder', () async {
    final backend = SettingsBackend();
    final adapter = ServiceSettingsAdapter(backend: backend, android: true);
    await adapter.status();
    final exports = {'incoming': 'opaque-legacy', 'archive': 'opaque-archive'};
    await adapter.mutate({
      'op': 'files',
      'exports': exports,
      'allow': ['laptop'],
    }, 7);
    expect(
      backend.calls.last,
      nativeCall('saveReceiving', {
        'expected_revision': 7,
        'exports': exports,
        'allow': ['laptop'],
      }),
    );
    await adapter.mutate({
      'op': 'files',
      'exports': <String, String>{},
      'allow': ['laptop'],
    }, 7);
    expect(
      backend.calls.last,
      nativeCall('saveReceiving', {
        'expected_revision': 7,
        'exports': <String, String>{},
        'allow': <String>[],
      }),
    );
  });

  for (final (id, method) in [
    ('incoming', 'removeInbox'),
    ('files', 'removeInbox'),
    ('checkpoints', 'removeShare'),
  ]) {
    test(
      'Android explicit folder removal of $id targets only its configuration',
      () async {
        final backend = SettingsBackend();
        final adapter = ServiceSettingsAdapter(backend: backend, android: true);
        await adapter.status();
        await adapter.mutate({'op': 'remove', 'id': id}, 7);
        expect(
          backend.calls.last,
          nativeCall(method, {'expected_revision': 7, 'id': id}),
        );
      },
    );
  }

  test(
    'Android disabling the built-in inbox preserves its folder configuration',
    () async {
      final backend = SettingsBackend();
      final adapter = ServiceSettingsAdapter(backend: backend, android: true);
      await adapter.status();
      await adapter.mutate({
        'op': 'set_service_enabled',
        'id': 'files',
        'incarnation': 'legacy-version',
        'enabled': false,
      }, 7);
      expect(
        backend.calls.last,
        nativeCall('setServiceEnabled', {
          'service': 'files',
          'incarnation': 'legacy-version',
          'enabled': false,
        }),
      );
      expect(backend.calls.where((call) => call.$1 == 'removeInbox'), isEmpty);
    },
  );

  test(
    'Android rejects unsupported operations, duplicate IDs and missing targets',
    () async {
      final backend = SettingsBackend();
      final adapter = ServiceSettingsAdapter(backend: backend, android: true);
      await adapter.status();
      for (final operation in <Map<String, dynamic>>[
        {
          'op': 'shell',
          'allow': ['laptop'],
        },
        {'op': 'http', 'id': 'web'},
        {'op': 'edit_folder', 'id': 'incoming', 'new_id': 'checkpoints'},
        {'op': 'file_share', 'id': 'incoming'},
        {'op': 'file_inbox', 'id': 'checkpoints'},
        {'op': 'remove', 'id': 'missing'},
        {'op': 'edit_folder', 'id': 'missing'},
      ]) {
        await expectLater(adapter.mutate(operation, 7), throwsFormatException);
      }
      expect(backend.calls, [nativeCall('ownerStatus', {})]);
    },
  );

  test('Android mutation requires status and does not refresh away an editor revision', () async {
    final backend = SettingsBackend();
    final adapter = ServiceSettingsAdapter(backend: backend, android: true);
    await expectLater(
      adapter.mutate({'op': 'remove', 'id': 'incoming'}, 7),
      throwsFormatException,
    );
    expect(backend.calls, isEmpty);
    await adapter.status();
    backend.folders['revision'] = 8;
    await adapter.status();
    await expectLater(
      adapter.mutate({'op': 'remove', 'id': 'incoming'}, 7),
      throwsFormatException,
    );
    expect(backend.calls, [
      nativeCall('ownerStatus', {}),
      nativeCall('ownerStatus', {}),
    ]);
  });

  test('Android propagates native stale-revision rejection with the captured revision', () async {
    final backend = SettingsBackend();
    final adapter = ServiceSettingsAdapter(backend: backend, android: true);
    await adapter.status();
    final failure = PlatformException(
      code: 'stale_revision',
      message: 'Refresh',
    );
    backend.folders['revision'] = 8;
    backend.mutationFailure = failure;
    await expectLater(
      adapter.mutate({'op': 'remove', 'id': 'incoming'}, 7),
      throwsA(same(failure)),
    );
    expect(
      backend.calls.last,
      nativeCall('removeInbox', {'expected_revision': 7, 'id': 'incoming'}),
    );
    expect(
      backend.calls.where((call) => call.$1 == 'ownerStatus'),
      hasLength(1),
    );
  });
}
