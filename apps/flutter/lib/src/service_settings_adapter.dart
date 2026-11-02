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
    final result = await backend.invoke('ownerStatus');
    if (android) {
      _revision = result['host']['revision'] as int;
      _services = {
        for (final service in result['host']['services'] as List)
          service['id'] as String: Map<String, dynamic>.from(service as Map),
      };
    }
    return result;
  }

  Future<void> mutate(Map<String, dynamic> operation, int revision) async {
    if (!android) {
      await backend.invoke('ownerRequest', {
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
      'files',
      'edit_folder',
      'remove',
      'set_service_enabled',
    ].contains(op)) {
      if (!const [
        'approve',
        'revoke',
        'approve_invitation',
        'reject_invitation',
        'grant_service',
        'revoke_service',
        'service',
        'edit_service',
      ].contains(op)) {
        throw const FormatException(
          'This owner operation is unavailable on this device.',
        );
      }
      await backend.invoke('ownerRequest', {
        'payload': {
          'op': 'mutate',
          'operation': {...operation, 'expected_revision': revision},
        },
      });
      return;
    }
    if (op == 'files') {
      final exports = Map<String, String>.from(operation['exports'] as Map);
      await backend.invoke('saveReceiving', {
        'expected_revision': revision,
        'exports': exports,
        'allow': exports.isEmpty ? <String>[] : operation['allow'],
      });
      return;
    }
    final id = operation['id'] as String;
    final selected = _services[id];
    if ((op == 'remove' || op == 'set_service_enabled') &&
        selected != null &&
        !isFolderService(selected)) {
      await backend.invoke('ownerRequest', {
        'payload': {
          'op': 'mutate',
          'operation': {...operation, 'expected_revision': revision},
        },
      });
      return;
    }
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
    final newId = operation['new_id'] as String? ?? id;
    if (op == 'edit_folder' && newId != id && _services.containsKey(newId)) {
      throw const FormatException(
        'This service ID already exists. Choose a different ID.',
      );
    }
    switch (op) {
      case 'file_inbox':
      case 'file_share':
      case 'edit_folder':
        await backend.invoke(kind == 'file_share' ? 'saveShare' : 'saveInbox', {
          'expected_revision': revision,
          'id': id,
          if (op == 'edit_folder') 'new_id': newId,
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
