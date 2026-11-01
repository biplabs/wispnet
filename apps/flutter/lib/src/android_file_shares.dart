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

/// Android's owner folder services use system-selected capabilities. Marker
/// paths and provider URIs never appear in the form or remote catalog.
class AndroidFileShares extends StatefulWidget {
  const AndroidFileShares({
    super.key,
    required this.backend,
    required this.onChanged,
    this.serviceId,
    this.addFolder = false,
  });
  final WispnetBackend backend;
  final VoidCallback onChanged;
  final String? serviceId;
  final bool addFolder;
  @override
  State<AndroidFileShares> createState() => _AndroidFileSharesState();
}

class _AndroidFileSharesState extends State<AndroidFileShares> {
  Map<String, dynamic>? status;
  String? failure;
  bool busy = false;
  bool openedFolder = false;
  Timer? timer;
  List<Map<String, dynamic>> get shares => [
    for (final inbox in status?['inboxes'] as List? ?? [])
      {
        ...(inbox as Map).cast<String, dynamic>(),
        'kind': inbox['kind'] ?? 'file_inbox',
      },
    for (final share in status?['shares'] as List? ?? [])
      {...(share as Map).cast<String, dynamic>(), 'kind': 'file_share'},
  ];
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
      final result = await widget.backend.invoke('folderServicesStatus');
      if (mounted) {
        setState(() {
          status = result;
          failure = null;
        });
        if (widget.addFolder && !openedFolder) {
          openedFolder = true;
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) unawaited(edit());
          });
        }
      }
    } catch (error) {
      if (mounted) setState(() => failure = _failure(error));
    }
  }

  Future<bool> remove(Map<String, dynamic> share, int revision) async {
    final confirmed = await showWispSheet<bool>(
      context,
      title: 'Remove folder?',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Remove “${share['id']}” from Wispnet? Files in the selected folder will be kept.',
          ),
          const SizedBox(height: 16),
          FilledButton(
            key: const ValueKey('confirm-remove-share'),
            onPressed: () => Navigator.pop(context, true),
            style: wispDangerButtonStyle(),
            child: const Text('Remove folder'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Keep folder'),
          ),
        ],
      ),
    );
    if (confirmed != true) return false;
    await widget.backend.invoke(
      share['kind'] == 'file_share' ? 'removeShare' : 'removeInbox',
      {'expected_revision': revision, 'id': share['id']},
    );
    return true;
  }

  Future<void> edit([Map<String, dynamic>? share]) async {
    if (busy || status == null) return;
    setState(() => busy = true);
    final revision = status!['revision'] as int? ?? 0;
    final existingId = share?['id'] as String? ?? '';
    final mode = share == null || isInboxService(share) ? 'inbox' : 'share';
    Future<void> save(
      String selectedMode,
      String id,
      String folder,
      List<String> allow,
    ) async {
      if (shares.any((s) => s['id'] == id && s['id'] != existingId)) {
        throw const FormatException(
          'This service ID already exists. Choose a different ID.',
        );
      }
      await widget.backend.invoke(
        selectedMode == 'inbox' ? 'saveInbox' : 'saveShare',
        {
          'expected_revision': revision,
          'id': id,
          'folder': folder,
          'allow': allow,
        },
      );
    }

    try {
      await showWispSheet<void>(
        context,
        title: existingId.isEmpty ? 'Add folder' : 'Edit $existingId folder',
        child: FileShareEditor(
          backend: widget.backend,
          clients: List.of(clients),
          android: true,
          folderService: true,
          chooseMode: share == null,
          mode: mode,
          id: existingId,
          root: share?['folder'] as String? ?? '',
          folderLabel: share?['label'] as String? ?? '',
          folderAvailable: share?['available'] != false,
          allow: (share?['allow'] as List? ?? []).cast<String>(),
          onSave: (id, folder, allow) => save(mode, id, folder, allow),
          onSaveFolder: save,
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
          const Text('Folders', style: TextStyle(fontWeight: FontWeight.bold)),
          const Text(
            'Give each folder a name and choose whether devices may send files to it or browse and download from it.',
          ),
          const SizedBox(height: 8),
        ],
        if (status == null && failure == null) const LinearProgressIndicator(),
        if (status != null && visible.isEmpty)
          Text(
            widget.serviceId == null
                ? 'No folders configured.'
                : 'This folder is no longer available.',
          ),
        for (final share in visible)
          WispServiceItem(
            key: ValueKey('android-share-${share['id']}'),
            title: share['id'] as String,
            kind: share['kind'] as String,
            enabled: share['enabled'] != false,
            busy: busy,
            contentPadding: EdgeInsets.zero,
            subtitle: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (widget.serviceId == null) ...[
                  WispBadge(
                    label: isInboxService(share)
                        ? 'Receive files'
                        : 'Share files',
                    color: WispColors.cyan,
                  ),
                  const SizedBox(height: 4),
                ],
                Text(
                  share['available'] == false
                      ? 'Folder access expired or is unavailable. Choose the folder again.'
                      : share['label'] as String? ?? 'Folder',
                ),
              ],
            ),
            onToggle: share['available'] == false && share['enabled'] == false
                ? null
                : (enabled) => toggle(share, enabled),
            onSettings: () => edit(share),
            settingsTooltip: 'Folder and access for ${share['id']}',
          ),
        if (failure != null)
          Text(failure!, style: const TextStyle(color: WispColors.error)),
        if (widget.serviceId == null) ...[
          const SizedBox(height: 12),
          const Divider(color: WispColors.border),
          const SizedBox(height: 8),
          WispCreateButton(
            key: const ValueKey('add-android-share'),
            label: 'Add folder',
            onPressed: busy || status == null ? null : () => edit(),
          ),
        ],
      ],
    );
  }
}
