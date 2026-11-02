import 'backend.dart';

/// Translates the shared owner settings operations to each native backend.
class ServiceSettingsAdapter {
  ServiceSettingsAdapter({
    required this.backend,
    this.android = false,
    this.host = '',
  });

  final WispnetBackend backend;
  final bool android;
  final String host;
  int? _revision;
  Map<String, Map<String, dynamic>> _services = {};

  Future<Map<String, dynamic>> status() async {
    if (!android) return backend.invoke('desktopStatus');
    final result = await backend.invoke('folderServicesStatus');
    final inboxes = [
      for (final inbox in result['inboxes'] as List? ?? [])
        {
          ...(inbox as Map).cast<String, dynamic>(),
          'kind': inbox['kind'] ?? 'file_inbox',
        },
    ];
    final shares = [
      for (final share in result['shares'] as List? ?? [])
        {...(share as Map).cast<String, dynamic>(), 'kind': 'file_share'},
    ];
    final services = [...inboxes, ...shares];
    _revision = result['revision'] as int;
    _services = {
      for (final service in services)
        service['id'] as String: Map<String, dynamic>.of(service),
    };
    return {
      'host': {
        'host': host,
        'revision': _revision,
        'services': services,
        'clients': result['clients'] ?? <Map<String, dynamic>>[],
        'file_inboxes': {
          for (final inbox in inboxes) inbox['id'] as String: inbox['folder'],
        },
        'file_shares': {
          for (final share in shares) share['id'] as String: share['folder'],
        },
        'file_exports': {
          for (final inbox in inboxes)
            if (inbox['kind'] == 'files')
              inbox['id'] as String: inbox['folder'],
        },
      },
      'invitations': <Map<String, dynamic>>[],
      'connected_peers': <String, dynamic>{},
      'recent_peers': <String, dynamic>{},
    };
  }

  Future<void> mutate(Map<String, dynamic> operation, int revision) async {
    if (!android) {
      await backend.invoke('desktopRequest', {
        'payload': {
          'op': 'mutate',
          'operation': {...operation, 'expected_revision': revision},
        },
      });
      return;
    }
    if (_revision == null) {
      throw const FormatException(
        'Refresh local services before changing them.',
      );
    }
    if (_revision != revision) {
      throw const FormatException(
        'Service settings changed. Refresh and try again.',
      );
    }
    final op = operation['op'];
    if (!const [
      'file_inbox',
      'file_share',
      'edit_folder',
      'remove',
      'set_service_enabled',
    ].contains(op)) {
      throw const FormatException(
        'This service operation is unavailable on this device.',
      );
    }
    final id = operation['id'] as String;
    var kind = op;
    if (op == 'file_inbox' || op == 'file_share') {
      if (_services.containsKey(id)) {
        throw const FormatException(
          'This service ID already exists. Choose a different ID.',
        );
      }
    } else {
      final service = _services[id];
      if (service == null || !isFolderService(service)) {
        throw const FormatException('This service is no longer shared.');
      }
      kind = service['kind'];
    }
    if (op == 'edit_folder' && (operation['new_id'] ?? id) != id) {
      throw const FormatException(
        'Renaming folders is unavailable on this device.',
      );
    }
    switch (op) {
      case 'file_inbox':
      case 'file_share':
      case 'edit_folder':
        await backend.invoke(kind == 'file_share' ? 'saveShare' : 'saveInbox', {
          'expected_revision': revision,
          'id': id,
          'folder': operation['root'],
          'allow': operation['allow'],
        });
      case 'remove':
        await backend.invoke(
          kind == 'file_share' ? 'removeShare' : 'removeInbox',
          {'expected_revision': revision, 'id': id},
        );
      case 'set_service_enabled':
        await backend.invoke('setServiceEnabled', {
          'service': id,
          'incarnation': operation['incarnation'],
          'enabled': operation['enabled'],
        });
    }
  }
}
