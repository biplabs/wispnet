import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend.dart';
import 'file_shares.dart';
import '../ui.dart';

String _failure(Object error) => error is FormatException
    ? error.message
    : error is PlatformException
    ? error.message ?? error.code
    : error.toString();

/// Android's owner shares use system-selected folder capabilities. Marker
/// paths and provider URIs never appear in the form or remote catalog.
class AndroidFileShares extends StatefulWidget {
  const AndroidFileShares({
    super.key,
    required this.backend,
    required this.onChanged,
    this.serviceId,
  });
  final WispnetBackend backend;
  final VoidCallback onChanged;
  final String? serviceId;
  @override
  State<AndroidFileShares> createState() => _AndroidFileSharesState();
}

class _AndroidFileSharesState extends State<AndroidFileShares> {
  Map<String, dynamic>? status;
  String? failure;
  bool busy = false;
  Timer? timer;
  List<Map<String, dynamic>> get shares =>
      (status?['shares'] as List? ?? []).cast<Map<String, dynamic>>();
  List<Map<String, dynamic>> get clients =>
      (status?['clients'] as List? ?? []).cast<Map<String, dynamic>>();

  @override
  void initState() {
    super.initState();
    unawaited(load());
    timer = Timer.periodic(const Duration(seconds: 3), (_) {
      if (!busy) unawaited(load());
    });
  }

  @override
  void dispose() {
    timer?.cancel();
    super.dispose();
  }

  Future<void> load() async {
    try {
      final result = await widget.backend.invoke('sharesStatus');
      if (mounted) {
        setState(() {
          status = result;
          failure = null;
        });
      }
    } catch (error) {
      if (mounted) setState(() => failure = _failure(error));
    }
  }

  Future<bool> remove(Map<String, dynamic> share, int revision) async {
    final confirmed = await showWispSheet<bool>(
      context,
      title: 'Remove share?',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Stop sharing “${share['id']}” with all devices? Files in the selected folder will be kept.',
          ),
          const SizedBox(height: 16),
          FilledButton(
            key: const ValueKey('confirm-remove-share'),
            onPressed: () => Navigator.pop(context, true),
            style: wispDangerButtonStyle(),
            child: const Text('Remove share'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Keep share'),
          ),
        ],
      ),
    );
    if (confirmed != true) return false;
    await widget.backend.invoke('removeShare', {
      'expected_revision': revision,
      'id': share['id'],
    });
    return true;
  }

  Future<void> edit([Map<String, dynamic>? share]) async {
    if (busy || status == null) return;
    setState(() => busy = true);
    final revision = status!['revision'] as int? ?? 0;
    final existingId = share?['id'] as String? ?? '';
    try {
      await showWispSheet<void>(
        context,
        title: existingId.isEmpty
            ? 'Add read-only folder share'
            : 'Edit $existingId share',
        child: FileShareEditor(
          backend: widget.backend,
          clients: List.of(clients),
          android: true,
          id: existingId,
          root: share?['folder'] as String? ?? '',
          folderLabel: share?['label'] as String? ?? '',
          folderAvailable: share?['available'] != false,
          allow: (share?['allow'] as List? ?? []).cast<String>(),
          onSave: (id, folder, allow) async {
            if (shares.any((s) => s['id'] == id && s['id'] != existingId)) {
              throw const FormatException(
                'This share name already exists. Choose a different name.',
              );
            }
            await widget.backend.invoke('saveShare', {
              'expected_revision': revision,
              'id': id,
              'folder': folder,
              'allow': allow,
            });
          },
          onRemove: share == null ? null : () => remove(share, revision),
        ),
      );
      widget.onChanged();
      await load();
    } catch (error) {
      if (mounted) setState(() => failure = _failure(error));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> toggle(Map<String, dynamic> share, bool enabled) async {
    if (busy) return;
    setState(() {
      busy = true;
      failure = null;
    });
    try {
      await widget.backend.invoke('setServiceEnabled', {
        'service': share['id'],
        'incarnation': share['incarnation'],
        'enabled': enabled,
      });
      widget.onChanged();
      await load();
    } catch (error) {
      if (mounted) setState(() => failure = _failure(error));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final visible = shares.where(
      (s) => widget.serviceId == null || s['id'] == widget.serviceId,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (widget.serviceId == null) ...[
          const SizedBox(height: 16),
          const Text(
            'Read-only folder shares',
            style: TextStyle(fontWeight: FontWeight.bold),
          ),
          const Text(
            'Choose a folder and the devices allowed to browse and download it. File inbox access is separate.',
          ),
          const SizedBox(height: 8),
          OutlinedButton.icon(
            key: const ValueKey('add-android-share'),
            onPressed: busy || status == null ? null : () => edit(),
            icon: const Icon(Icons.create_new_folder_outlined),
            label: const Text('Add read-only folder share'),
          ),
        ],
        if (status == null && failure == null) const LinearProgressIndicator(),
        if (status != null && visible.isEmpty)
          Text(
            widget.serviceId == null
                ? 'No folders shared.'
                : 'This folder share is no longer available.',
          ),
        for (final share in visible)
          WispServiceItem(
            key: ValueKey('android-share-${share['id']}'),
            title: share['id'] as String,
            kind: 'file_share',
            enabled: share['enabled'] != false,
            busy: busy,
            contentPadding: EdgeInsets.zero,
            subtitle: Text(
              share['available'] == false
                  ? 'Folder access expired or is unavailable. Choose the folder again.'
                  : share['label'] as String? ?? 'Shared folder',
            ),
            onToggle: share['available'] == false && share['enabled'] == false
                ? null
                : (enabled) => toggle(share, enabled),
            onSettings: () => edit(share),
            settingsTooltip: 'Folder and access for ${share['id']}',
          ),
        if (failure != null)
          Text(failure!, style: const TextStyle(color: WispColors.error)),
      ],
    );
  }
}
