import 'dart:convert';

import 'package:flutter/services.dart';

/// UI-facing operations. Platform adapters own identity, persistence and access.
abstract interface class WispnetBackend {
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]);
}

class PlatformBackend implements WispnetBackend {
  static const channel = MethodChannel('app.wispnet/ui');
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    final reply = await channel.invokeMethod<String>(
      method,
      jsonEncode(arguments),
    );
    return jsonDecode(reply ?? '{}') as Map<String, dynamic>;
  }
}

class Machine {
  Machine(this.id, this.data);
  final String id;
  final Map<String, dynamic> data;
  String get name => data['name'] as String? ?? 'Host ${id.substring(0, 8)}';
  String get state => data['state'] as String? ?? 'unreachable';
  String get status => data['status'] as String? ?? 'Unavailable';
  bool get online => state == 'available';
  List<Map<String, dynamic>> get services =>
      ((data['catalog'] as Map?)?['services'] as List? ?? [])
          .cast<Map<String, dynamic>>();
  bool canOpen(Map<String, dynamic> service) =>
      service['kind'] == 'http' &&
      RegExp(r'^[0-9a-f]{64}$')
          .hasMatch(service['incarnation'] as String? ?? '') &&
      ![
        'host_denied',
        'approval_pending',
        'state_changed',
        'unverified',
      ].contains(state);
}

List<String> orderedIds(Iterable<String> ids, List<dynamic>? saved) {
  final available = ids.toSet();
  return {
    ...?saved?.whereType<String>().where(available.contains),
    ...ids,
  }.toList();
}
