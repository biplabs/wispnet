import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend.dart';
import 'sheets.dart';
import 'theme.dart';

/// Owner workflows share the same sheets as Android; no permanent navigation rail.
class DesktopSettings extends StatefulWidget {
  const DesktopSettings({
    super.key,
    required this.backend,
    required this.onAccess,
    required this.onChanged,
    this.initialSection = 'My services',
    this.servicesOnly = false,
    this.serviceId,
  });
  final String initialSection;
  final bool servicesOnly;
  final String? serviceId;
  final WispnetBackend backend;
  final VoidCallback onAccess, onChanged;
  @override
  State<DesktopSettings> createState() => _DesktopSettingsState();
}

class _DesktopSettingsState extends State<DesktopSettings> {
  Map<String, dynamic>? status;
  List<dynamic> operations = [];
  late String section = widget.initialSection;
  String? error;
  String localUrl = '';
  bool busy = false, openingAddress = false;
  Timer? timer;
  List<Map<String, dynamic>> get services =>
      (status?['host']['services'] as List? ?? []).cast<Map<String, dynamic>>();
  List<Map<String, dynamic>> get clients =>
      (status?['host']['clients'] as List? ?? []).cast<Map<String, dynamic>>();
  List<Map<String, dynamic>> get requests =>
      (status?['invitations'] as List? ?? [])
          .cast<Map<String, dynamic>>()
          .where((i) => i['state'] == 'pending')
          .toList();
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
      final next = await widget.backend.invoke('desktopStatus');
      final journal = await widget.backend.invoke('desktopData', {
        'op': 'operations',
      });
      var address = '';
      String? addressError;
      final selected = (next['host']['services'] as List)
          .cast<Map<String, dynamic>>()
          .where((s) => s['id'] == widget.serviceId && s['kind'] == 'http')
          .firstOrNull;
      if (selected != null) {
        try {
          final args = {
            'host': next['host']['host'],
            'service': selected['id'],
          };
          final route = await widget.backend.invoke('settings', args);
          address = route['url'] as String? ?? '';
          if (address.isEmpty) {
            final opened = await widget.backend.invoke('open', {
              ...args,
              'incarnation': selected['incarnation'],
            });
            address = opened['url'] as String? ?? '';
          }
        } catch (e) {
          addressError = failureMessage(e);
        }
      }
      if (mounted) {
        setState(() {
          localUrl = address;
          status = next;
          operations = journal['value'] as List;
          error = addressError;
        });
      }
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    }
  }

  Future<void> run(Future<void> Function() action) async {
    if (busy) return;
    setState(() {
      busy = true;
      error = null;
    });
    try {
      await action();
      widget.onChanged();
      await load();
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> request(Map<String, dynamic> payload) async {
    await widget.backend.invoke('desktopRequest', {'payload': payload});
  }

  Future<void> mutate(Map<String, dynamic> operation, int revision) => request({
    'op': 'mutate',
    'operation': {...operation, 'expected_revision': revision},
  });
  Future<void> confirm(
    String title,
    String detail,
    Future<void> Function() action, {
    bool danger = false,
    String confirmLabel = 'Confirm',
  }) async {
    await showWispSheet<void>(
      context,
      title: title,
      child: _ActionForm(
        builder: (context, submit, working, failure) => Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SelectableText(detail),
            const SizedBox(height: 16),
            if (failure != null)
              Text(failure, style: const TextStyle(color: WispColors.error)),
            FilledButton(
              onPressed: working ? null : () => submit(action),
              style: danger ? dangerStyle : null,
              child: Text(confirmLabel),
            ),
            const SizedBox(height: 8),
            TextButton(
              onPressed: working ? null : () => Navigator.pop(context),
              child: const Text('Cancel'),
            ),
          ],
        ),
      ),
    );
    widget.onChanged();
    await load();
  }

  Future<void> editShell(Map<String, dynamic> service) async {
    final revision = status!['host']['revision'] as int;
    final allowed = (service['allow'] as List? ?? []).cast<String>().toSet();
    final devices = List<Map<String, dynamic>>.of(clients);
    await showWispSheet<void>(
      context,
      title: 'Shell access',
      child: _ActionForm(
        builder: (context, submit, working, failure) => StatefulBuilder(
          builder: (context, update) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Text(
                'Shell is built into Wispnet and is always available on this computer. Select which paired devices may open a remote shell as your OS user.',
              ),
              const SizedBox(height: 12),
              const Text(
                'Leave all devices unchecked to allow access only from this computer.',
              ),
              for (final c in devices)
                CheckboxListTile(
                  contentPadding: EdgeInsets.zero,
                  title: Text(c['name'] as String),
                  subtitle: Text(c['peer'] as String),
                  value: allowed.contains(c['peer']),
                  onChanged: working
                      ? null
                      : (value) => update(() {
                          if (value!) {
                            allowed.add(c['peer'] as String);
                          } else {
                            allowed.remove(c['peer']);
                          }
                        }),
                ),
              if (failure != null)
                Text(failure, style: const TextStyle(color: WispColors.error)),
              FilledButton(
                onPressed: working
                    ? null
                    : () => submit(
                        () => mutate({
                          'op': 'shell',
                          'allow': allowed.toList(),
                        }, revision),
                      ),
                child: const Text('Save access'),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Map<String, String> parseFileExports(String text) {
    final roots = <String, String>{};
    for (final raw in text.split('\n')) {
      final line = raw.trim();
      if (line.isEmpty) continue;
      final split = line.indexOf('=');
      final name = split < 0 ? '' : line.substring(0, split);
      final root = split < 0 ? '' : line.substring(split + 1);
      if (!RegExp(r'^[a-z0-9][a-z0-9._-]{0,63}$').hasMatch(name) ||
          !root.startsWith('/') ||
          roots.containsKey(name)) {
        throw Exception('Use a unique name=/absolute/folder on each line.');
      }
      roots[name] = root;
    }
    return roots;
  }

  Future<void> editFiles(Map<String, dynamic> service) async {
    final revision = status!['host']['revision'] as int;
    final allowed = (service['allow'] as List? ?? []).cast<String>().toSet();
    final devices = List<Map<String, dynamic>>.of(clients);
    final currentExports = (status!['host']['file_exports'] as Map? ?? {});
    final exports = TextEditingController(
      text: currentExports.entries.map((e) => '${e.key}=${e.value}').join('\n'),
    );
    await showWispSheet<void>(
      context,
      title: 'Files access',
      child: _ActionForm(
        controllers: [exports],
        builder: (context, submit, working, failure) => StatefulBuilder(
          builder: (context, update) {
            var hasFolders = false;
            try {
              hasFolders = parseFileExports(exports.text).isNotEmpty;
            } catch (_) {
              // The form reports invalid folder entries when saved.
            }
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Text(
                  'Choose a folder before sharing Files. Selected devices can upload and download files in the shared folders without confirming each transfer. Transfers can resume after disconnects.',
                ),
                const SizedBox(height: 12),
                TextField(
                  controller: exports,
                  enabled: !working,
                  onChanged: (_) => update(() {}),
                  minLines: 2,
                  maxLines: 6,
                  decoration: const InputDecoration(
                    labelText: 'Exported folders',
                    hintText: 'training=/absolute/path/to/training',
                    helperText: 'One name=/absolute/folder per line. Folders must already exist. Removing every folder revokes Files access.',
                    helperMaxLines: 3,
                  ),
                ),
                const SizedBox(height: 12),
                Text(
                  hasFolders
                      ? 'Select paired devices allowed to use these folders. Leave all unchecked for local access only.'
                      : 'Set a shared folder before granting Files access.',
                ),
                for (final c in devices)
                  CheckboxListTile(
                    contentPadding: EdgeInsets.zero,
                    title: Text(c['name'] as String),
                    subtitle: Text(c['peer'] as String),
                    value: hasFolders && allowed.contains(c['peer']),
                    onChanged: working || !hasFolders
                        ? null
                        : (value) => update(() {
                            if (value!) {
                              allowed.add(c['peer'] as String);
                            } else {
                              allowed.remove(c['peer']);
                            }
                          }),
                  ),
                if (failure != null)
                  Text(
                    failure,
                    style: const TextStyle(color: WispColors.error),
                  ),
                FilledButton(
                  onPressed: working
                      ? null
                      : () => submit(() async {
                          final roots = parseFileExports(exports.text);
                          await mutate({
                            'op': 'files',
                            'allow': roots.isEmpty
                                ? <String>[]
                                : allowed.toList(),
                            'exports': roots,
                          }, revision);
                        }),
                  child: const Text('Save folders and access'),
                ),
              ],
            );
          },
        ),
      ),
    );
  }

  Future<void> editService([Map<String, dynamic>? service]) async {
    final current = status!;
    final originalId = service?['id'] as String?;
    final id = TextEditingController(text: originalId);
    final endpoint = Uri.parse(
      'tcp://${service?['backend'] ?? '127.0.0.1:8080'}',
    );
    final portField = TextEditingController(text: endpoint.port.toString());
    var kind = service?['kind'] as String? ?? 'http';
    var renamed = false;
    final allowed = (service?['allow'] as List? ?? []).cast<String>().toSet();
    final devices = List<Map<String, dynamic>>.of(clients);
    await showWispSheet<void>(
      context,
      title: service == null ? 'Share a service' : 'Edit service',
      child: _ActionForm(
        controllers: [id, portField],
        builder: (context, submit, working, failure) => StatefulBuilder(
          builder: (context, update) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TextField(
                controller: id,
                enabled: !working,
                decoration: InputDecoration(
                  labelText: 'Service ID',
                  helperMaxLines: 3,
                  helperText: service == null
                      ? '1–64 lowercase letters, numbers, dots, underscores or hyphens'
                      : 'Renaming keeps device permissions. Update saved URLs afterward.',
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: portField,
                enabled: !working,
                keyboardType: TextInputType.number,
                inputFormatters: [FilteringTextInputFormatter.digitsOnly],
                decoration: const InputDecoration(labelText: 'Port'),
              ),
              const SizedBox(height: 12),
              DropdownButtonFormField<String>(
                initialValue: kind,
                decoration: const InputDecoration(
                  labelText: 'Protocol',
                  helperText: 'HTTP: websites and APIs with browser links. TCP: raw connections for native apps. UDP services are not supported yet.',
                  helperMaxLines: 5,
                ),
                items: ['http', 'tcp']
                    .map(
                      (k) => DropdownMenuItem(
                        value: k,
                        child: Text(k.toUpperCase()),
                      ),
                    )
                    .toList(),
                onChanged: working ? null : (v) => update(() => kind = v!),
              ),
              const SizedBox(height: 12),
              const Text(
                'Only selected devices can access this service. Saving replaces the complete allowlist.',
              ),
              for (final c in devices)
                CheckboxListTile(
                  contentPadding: EdgeInsets.zero,
                  title: Text(c['name'] as String),
                  subtitle: Text(c['peer'] as String),
                  value: allowed.contains(c['peer']),
                  onChanged: working
                      ? null
                      : (v) => update(() {
                          if (v!) {
                            allowed.add(c['peer'] as String);
                          } else {
                            allowed.remove(c['peer']);
                          }
                        }),
                ),
              if (failure != null)
                Text(failure, style: const TextStyle(color: WispColors.error)),
              FilledButton(
                onPressed: working
                    ? null
                    : () => submit(() async {
                        final serviceId = id.text.trim();
                        if (serviceId == 'shell' || serviceId == 'files') {
                          throw const FormatException(
                            '“shell” and “files” are reserved for built-in services. Use their access settings to manage permissions.',
                          );
                        }
                        if (!RegExp(r'^[a-z0-9][a-z0-9._-]{0,63}$')
                            .hasMatch(serviceId)) {
                          throw const FormatException(
                            'Enter a valid service ID.',
                          );
                        }
                        if ((current['host']['services'] as List).any(
                          (s) => s['id'] == serviceId && s['id'] != originalId,
                        )) {
                          throw const FormatException(
                            'This ID exists. Edit that service instead.',
                          );
                        }
                        final port = int.tryParse(portField.text.trim());
                        if (port == null || port < 1 || port > 65535) {
                          throw const FormatException(
                            'Enter a port between 1 and 65535.',
                          );
                        }
                        final host = endpoint.host
                            .replaceAll('[', '')
                            .replaceAll(']', '');
                        final address =
                            '${host.contains(':') ? '[$host]' : host}:$port';
                        await mutate({
                          'op': service == null ? 'service' : 'edit_service',
                          'id': originalId ?? serviceId,
                          if (service != null) 'new_id': serviceId,
                          'backend': address,
                          'kind': kind,
                          'allow': allowed.toList(),
                        }, current['host']['revision'] as int);
                        renamed = service != null && serviceId != originalId;
                      }),
                child: const Text('Save service'),
              ),
            ],
          ),
        ),
      ),
    );
    widget.onChanged();
    if (renamed && widget.serviceId != null && mounted) {
      // Return to the refreshed directory rather than leaving settings pointed
      // at the old service ID.
      Navigator.pop(context);
      return;
    }
    await load();
  }

  Future<void> approve(Map<String, dynamic> invitation) async {
    final revision = status!['host']['revision'] as int;
    final hasFileFolders =
        (status!['host']['file_exports'] as Map? ?? {}).isNotEmpty;
    final existing = services
        .where(
          (s) =>
              (s['allow'] as List).contains(invitation['peer']) &&
              (s['kind'] != 'files' || hasFileFolders),
        )
        .map((s) => s['id'] as String)
        .toSet();
    final allowed = {...existing};
    final available = List<Map<String, dynamic>>.of(services);
    await showWispSheet<void>(
      context,
      title: 'Approve ${invitation['name'] ?? 'device'}',
      child: _ActionForm(
        builder: (context, submit, working, failure) => StatefulBuilder(
          builder: (context, update) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SelectableText(invitation['peer'] as String),
              const SizedBox(height: 12),
              const Text(
                'Compare the full device identity. Choose services to grant; existing grants are preserved. No selection pairs without adding access.',
              ),
              for (final s in available)
                CheckboxListTile(
                  title: Text(s['id'] as String),
                  subtitle: s['kind'] == 'files'
                      ? Text(
                          hasFileFolders
                              ? 'Upload and download without confirming each transfer.'
                              : 'Set a shared folder before granting Files access.',
                        )
                      : null,
                  value:
                      (s['kind'] != 'files' || hasFileFolders) &&
                      allowed.contains(s['id']),
                  onChanged:
                      working ||
                          existing.contains(s['id']) ||
                          (s['kind'] == 'files' && !hasFileFolders)
                      ? null
                      : (v) => update(() {
                          if (v!) {
                            allowed.add(s['id'] as String);
                          } else {
                            allowed.remove(s['id']);
                          }
                        }),
                ),
              if (failure != null)
                Text(failure, style: const TextStyle(color: WispColors.error)),
              FilledButton(
                onPressed: working
                    ? null
                    : () => submit(
                        () => mutate({
                          'op': 'approve_invitation',
                          'invitation': invitation['invitation'],
                          'services': allowed.toList(),
                        }, revision),
                      ),
                child: const Text('Approve device'),
              ),
            ],
          ),
        ),
      ),
    );
    widget.onChanged();
    await load();
  }

  Future<void> invite() async {
    await showWispSheet<void>(
      context,
      title: 'Pair this machine',
      child: _Invitation(backend: widget.backend),
    );
    await load();
  }

  Future<void> combined(String op) async {
    final response = await widget.backend.invoke('desktopData', {
      'op': 'targets',
    });
    final targets = (response['value'] as List).cast<Map<String, dynamic>>();
    final selected = <String>{};
    final peer = TextEditingController(), name = TextEditingController();
    Map<String, dynamic>? plan;
    if (!mounted) return;
    await showWispSheet<void>(
      context,
      title: op == 'approve'
          ? 'Add client across hosts'
          : 'Revoke across hosts',
      child: _ActionForm(
        controllers: [peer, name],
        builder: (context, submit, working, failure) => StatefulBuilder(
          builder: (context, update) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Text(
                'Each selected host applies independently using configured SSH owner access. Adding a client grants no services automatically. Offline hosts remain pending.',
              ),
              TextField(
                controller: peer,
                enabled: plan == null && !working,
                decoration: const InputDecoration(labelText: 'Host ID'),
              ),
              if (op == 'approve')
                TextField(
                  controller: name,
                  enabled: plan == null && !working,
                  decoration: const InputDecoration(labelText: 'Device ID'),
                ),
              for (final target in targets)
                CheckboxListTile(
                  title: Text(
                    '${target['ssh'] ?? 'This host'} · ${target['host']}',
                  ),
                  value: selected.contains(target['host']),
                  onChanged: plan != null || working
                      ? null
                      : (v) => update(() {
                          if (v!) {
                            selected.add(target['host'] as String);
                          } else {
                            selected.remove(target['host']);
                          }
                        }),
                ),
              if (failure != null)
                Text(failure, style: const TextStyle(color: WispColors.error)),
              if (plan != null)
                SelectableText(
                  'Apply $op for ${peer.text.trim()} on ${selected.length} selected hosts?',
                ),
              FilledButton(
                onPressed: working
                    ? null
                    : () {
                        if (plan == null) {
                          if (!RegExp(r'^[0-9a-f]{64}$')
                                  .hasMatch(peer.text.trim()) ||
                              selected.isEmpty ||
                              op == 'approve' &&
                                  !RegExp(r'^[a-z0-9][a-z0-9._-]{0,63}$')
                                      .hasMatch(name.text.trim())) {
                            ScaffoldMessenger.of(context).showSnackBar(
                              const SnackBar(
                                content: Text(
                                  'Enter a full host ID, a device ID for approval, and select hosts.',
                                ),
                              ),
                            );
                            return;
                          }
                          update(
                            () => plan = {
                              'action': {
                                'op': op,
                                'peer': peer.text.trim(),
                                if (op == 'approve') 'name': name.text.trim(),
                              },
                              'targets': targets
                                  .where((t) => selected.contains(t['host']))
                                  .toList(),
                            },
                          );
                        } else {
                          submit(() async {
                            await widget.backend.invoke('desktopData', {
                              'op': 'start',
                              'plan': plan,
                            });
                          });
                        }
                      },
                child: Text(
                  plan == null ? 'Review operation' : 'Confirm operation',
                ),
              ),
            ],
          ),
        ),
      ),
    );
    await load();
    widget.onChanged();
  }

  Map<String, dynamic>? publication(Map<String, dynamic> service) =>
      ((status?['public_access']?['services'] as List?) ?? [])
          .cast<Map<String, dynamic>>()
          .where(
            (p) =>
                p['service'] == service['id'] &&
                p['incarnation'] == service['incarnation'],
          )
          .firstOrNull;

  Future<void> openAddress(String url, String? publicService) async {
    if (busy || openingAddress) return;
    setState(() {
      openingAddress = true;
      error = null;
    });
    try {
      await widget.backend.invoke(
        publicService == null ? 'launch' : 'launchPublic',
        publicService == null ? {'url': url} : {'service': publicService},
      );
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    } finally {
      if (mounted) setState(() => openingAddress = false);
    }
  }

  Widget addressLink(String url, {String? publicService}) => Padding(
    padding: const EdgeInsets.only(bottom: 8),
    child: Align(
      alignment: Alignment.centerLeft,
      child: Semantics(
        link: true,
        child: Tooltip(
          message: publicService == null
              ? 'Open local service'
              : 'Open public service',
          child: WispInkWell(
            onTap: busy || openingAddress
                ? null
                : () => openAddress(url, publicService),
            child: Text(
              url,
              style: const TextStyle(
                color: WispColors.cyan,
                decoration: TextDecoration.underline,
                decorationColor: WispColors.cyan,
              ),
            ),
          ),
        ),
      ),
    ),
  );

  ButtonStyle get dangerStyle => OutlinedButton.styleFrom(
    foregroundColor: WispColors.error,
    side: const BorderSide(color: WispColors.error),
  ).copyWith(backgroundColor: wispButtonBackground(const Color(0xff401e2a)));

  Widget action(String label, VoidCallback callback, {bool danger = false}) =>
      Padding(
        padding: const EdgeInsets.only(bottom: 8),
        child: OutlinedButton(
          onPressed: busy ? null : callback,
          style: danger ? dangerStyle : null,
          child: Text(label),
        ),
      );
  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      if (!widget.servicesOnly) ...[
        action('Device and browser settings', widget.onAccess),
        action('Pair this machine', () => run(invite)),
        const SizedBox(height: 12),
        DropdownButton<String>(
          isExpanded: true,
          value: section,
          items:
              ['My services', 'Devices', 'Pairing requests', 'Owner operations']
                  .map(
                    (s) => DropdownMenuItem(
                      value: s,
                      child: Text(
                        s == 'Pairing requests' ? '$s (${requests.length})' : s,
                      ),
                    ),
                  )
                  .toList(),
          onChanged: busy ? null : (v) => setState(() => section = v!),
        ),
      ],
      if (error != null)
        Text(error!, style: const TextStyle(color: WispColors.error)),
      if (status == null)
        const LinearProgressIndicator()
      else if (section == 'My services') ...[
        if (widget.serviceId == null) ...[
          action('Add a new service', () => run(() => editService())),
          for (final s in services.where(
            (s) => s['id'] == 'shell' && s['kind'] == 'shell',
          ))
            action('Shell access', () => run(() => editShell(s))),
          for (final s in services.where(
            (s) => s['id'] == 'files' && s['kind'] == 'files',
          ))
            action('Files access', () => run(() => editFiles(s))),
          if (widget.servicesOnly)
            action('Pair this machine', () => run(invite)),
        ] else if (!services.any((s) => s['id'] == widget.serviceId))
          const Text('This service is no longer shared.'),
        for (final s in services.where((s) => s['id'] == widget.serviceId))
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: WispPanel(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    s['id'] as String,
                    style: const TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                  Text(s['kind'].toString().toUpperCase()),
                  const SizedBox(height: 8),
                  if (localUrl.isNotEmpty) addressLink(localUrl),
                  if (publication(s)?['enabled'] == true &&
                      publication(s)?['url'] is String)
                    addressLink(
                      publication(s)!['url'] as String,
                      publicService: s['id'] as String,
                    ),
                  const SizedBox(height: 10),
                  if (s['kind'] == 'shell') ...[
                    const Text('Built-in service · available on this computer'),
                    const SizedBox(height: 8),
                    action('Manage access', () => run(() => editShell(s))),
                  ] else if (s['kind'] == 'files') ...[
                    const Text('Built-in service · resumable file transfers'),
                    const SizedBox(height: 8),
                    action(
                      'Manage folders and access',
                      () => run(() => editFiles(s)),
                    ),
                  ] else ...[
                    action('Edit', () => run(() => editService(s))),
                    action(
                      'Remove service',
                      () => run(
                        () => confirm(
                          'Remove service?',
                          'Remove “${s['id']}” from this machine’s shared services? Paired devices will lose Wispnet access, and its public URL will stop working. This does not stop or delete the underlying application.',
                          () => mutate({
                            'op': 'remove',
                            'id': s['id'],
                          }, status!['host']['revision'] as int),
                          danger: true,
                          confirmLabel: 'Remove service',
                        ),
                      ),
                      danger: true,
                    ),
                  ],
                  if (s['kind'] == 'http') ..._public(s),
                ],
              ),
            ),
          ),
      ] else if (section == 'Devices') ...[
        Text(
          '${clients.length} approved devices · Active means connected or seen within the last 60 seconds.',
        ),
        for (final c in clients)
          WispPanel(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(c['name'] as String),
                SelectableText(c['peer'] as String),
                Text(
                  (status!['connected_peers'] as Map).containsKey(c['peer'])
                      ? 'Connected'
                      : (status!['recent_peers'] as Map?)?[c['peer']] != null
                      ? 'Seen ${((status!['recent_peers'] as Map)[c['peer']] as num).floor()} seconds ago'
                      : 'Idle',
                ),
                action(
                  'Revoke ${c['name']}',
                  () => run(
                    () => confirm(
                      'Revoke device?',
                      '${c['peer']}\nRemove access to services on this machine?',
                      () => mutate({
                        'op': 'revoke',
                        'peer': c['peer'],
                      }, status!['host']['revision'] as int),
                    ),
                  ),
                ),
              ],
            ),
          ),
        SelectableText('This machine’s identity\n${status!['host']['host']}'),
      ] else if (section == 'Pairing requests') ...[
        if (requests.isEmpty) const Text('No pending requests.'),
        for (final i in requests)
          WispPanel(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(i['name'] as String? ?? 'New device'),
                SelectableText(i['peer'] as String),
                action('Approve device', () => run(() => approve(i))),
                action(
                  'Reject request',
                  () => run(
                    () => confirm(
                      'Reject request?',
                      i['peer'] as String,
                      () => mutate({
                        'op': 'reject_invitation',
                        'invitation': i['invitation'],
                      }, status!['host']['revision'] as int),
                    ),
                  ),
                ),
              ],
            ),
          ),
      ] else ...[
        action('Add client across hosts', () => run(() => combined('approve'))),
        action('Revoke across hosts', () => run(() => combined('revoke'))),
        if (operations.isEmpty) const Text('No saved owner operations.'),
        for (final op in operations)
          WispPanel(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text('${op['action']['op']} · ${op['action']['peer']}'),
                SelectableText(op['id'] as String),
                for (final t in op['targets'] as List)
                  Text(
                    '${t['ssh'] ?? 'This host'} · ${t['host']} · ${t['state']} · ${t['reason'] ?? ''}${t['revision'] == null ? '' : ' · revision ${t['revision']}'}',
                  ),
                if ((op['targets'] as List).any((t) => t['state'] == 'pending'))
                  action(
                    'Retry pending hosts',
                    () => run(() async {
                      await widget.backend.invoke('desktopData', {
                        'op': 'retry',
                        'id': op['id'],
                      });
                    }),
                  ),
              ],
            ),
          ),
      ],
      if (!widget.servicesOnly) action('Refresh', () => run(load)),
    ],
  );
  List<Widget> _public(Map<String, dynamic> service) {
    final enabled = publication(service)?['enabled'] == true;
    return [
      action(
        enabled ? 'Disable public access' : 'Enable public access',
        () => run(
          () => confirm(
            enabled
                ? 'Disable public access?'
                : 'Expose service to the internet?',
            enabled
                ? 'Stop public internet access to “${service['id']}”? Its public URL will stop working. Approved paired devices will keep their Wispnet access.'
                : 'This exposes “${service['id']}” unprotected to the public internet. Anyone with the public URL can connect without pairing, device approval, or Wispnet authentication. Wispnet adds no password or access protection; any login must be provided by the service itself.',
            () => request({
              'op': 'configure_public',
              'service': service['id'],
              'incarnation': service['incarnation'],
              'enabled': !enabled,
              'name': status!['host_name'],
            }),
            danger: true,
            confirmLabel: enabled
                ? 'Disable public access'
                : 'Enable public access',
          ),
        ),
        danger: true,
      ),
    ];
  }
}

typedef _FormBuilder = Widget Function(
  BuildContext context,
  void Function(Future<void> Function()) submit,
  bool busy,
  String? error,
);

class _ActionForm extends StatefulWidget {
  const _ActionForm({required this.builder, this.controllers = const []});
  final List<TextEditingController> controllers;
  final _FormBuilder builder;
  @override
  State<_ActionForm> createState() => _ActionFormState();
}

class _ActionFormState extends State<_ActionForm> {
  @override
  void dispose() {
    for (final controller in widget.controllers) {
      controller.dispose();
    }
    super.dispose();
  }

  bool busy = false;
  String? error;
  Future<void> submit(Future<void> Function() action) async {
    setState(() {
      busy = true;
      error = null;
    });
    try {
      await action();
      if (mounted) Navigator.pop(context);
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: !busy,
    child: Material(
      color: Colors.transparent,
      child: widget.builder(context, submit, busy, error),
    ),
  );
}

class _Invitation extends StatefulWidget {
  const _Invitation({required this.backend});
  final WispnetBackend backend;
  @override
  State<_Invitation> createState() => _InvitationState();
}

class _InvitationState extends State<_Invitation> {
  Map<String, dynamic>? qr;
  String? error;
  bool expired = false, showQr = true;
  Timer? timer;
  @override
  void initState() {
    super.initState();
    unawaited(issue());
  }

  Future<void> issue() async {
    try {
      final next = await widget.backend.invoke('desktopRequest', {
        'payload': {'op': 'invite_qr'},
      });
      if (!mounted) return;
      setState(() => qr = next);
      timer = Timer.periodic(
        const Duration(seconds: 1),
        (_) => unawaited(check()),
      );
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    }
  }

  Future<void> check() async {
    if (expired) return;
    try {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final status = await widget.backend.invoke('desktopStatus');
      final received = (status['invitations'] as List).any(
        (i) =>
            i['invitation'] == qr!['invitation'] &&
            !['issued', 'challenged'].contains(i['state']),
      );
      if (mounted) {
        setState(() {
          expired = now >= (qr!['expires_at'] as num) || received;
        });
      }
      if (expired) timer?.cancel();
    } catch (_) {
      if (mounted) setState(() => expired = true);
      timer?.cancel();
    }
  }

  @override
  void dispose() {
    timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (error != null) return Text(error!);
    if (qr == null) return const CircularProgressIndicator();
    if (expired) {
      return const Text(
        'Code expired or request received. Close to review pairing requests.',
      );
    }
    final matrix = (qr!['matrix'] as List)
        .map((r) => (r as List).map((v) => v == true || v == 1).toList())
        .toList();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Text(
          'Scan or copy the code, then approve the request here and choose allowed services.',
        ),
        SelectableText(qr!['host'] as String),
        const SizedBox(height: 16),
        WispPanel(
          radius: 14,
          padding: const EdgeInsets.all(4),
          child: Row(
            children: [
              for (final entry in {
                true: 'QR code',
                false: 'Pairing code',
              }.entries)
                Expanded(
                  child: TextButton(
                    onPressed: () => setState(() => showQr = entry.key),
                    style: TextButton.styleFrom(
                      backgroundColor: showQr == entry.key
                          ? const Color(0xff1f3666)
                          : Colors.transparent,
                      foregroundColor: showQr == entry.key
                          ? WispColors.text
                          : WispColors.muted,
                      minimumSize: const Size(0, 44),
                    ),
                    child: Text(entry.value),
                  ),
                ),
            ],
          ),
        ),
        const SizedBox(height: 16),
        if (showQr)
          Center(
            child: SizedBox(
              key: const ValueKey('pairing-qr'),
              width: 240,
              height: 240,
              child: CustomPaint(painter: _QrPainter(matrix)),
            ),
          )
        else
          SelectableText(
            qr!['code'] as String,
            key: const ValueKey('pairing-code'),
          ),
        const SizedBox(height: 16),
        Text(
          'Expires in ${((qr!['expires_at'] as num) - DateTime.now().millisecondsSinceEpoch / 1000).ceil()}s',
        ),
        if (!showQr)
          OutlinedButton(
            onPressed: () async {
              await check();
              if (!expired) {
                await Clipboard.setData(
                  ClipboardData(text: qr!['code'] as String),
                );
              }
            },
            child: const Text('Copy pairing code'),
          ),
      ],
    );
  }
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.matrix);
  final List<List<bool>> matrix;
  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = Colors.white);
    if (matrix.isEmpty ||
        matrix.length > 256 ||
        matrix.any((r) => r.length != matrix.length)) {
      return;
    }
    final step = size.width / (matrix.length + 8);
    final paint = Paint()
      ..color = Colors.black
      ..isAntiAlias = false;
    for (var y = 0; y < matrix.length; y++) {
      for (var x = 0; x < matrix.length; x++) {
        if (matrix[y][x]) {
          canvas.drawRect(
            Rect.fromLTWH((x + 4) * step, (y + 4) * step, step, step),
            paint,
          );
        }
      }
    }
  }

  @override
  bool shouldRepaint(_QrPainter oldDelegate) => oldDelegate.matrix != matrix;
}
