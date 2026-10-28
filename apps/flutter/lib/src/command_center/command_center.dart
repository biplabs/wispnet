import 'dart:async';

import 'package:flutter/material.dart';

import '../backend.dart';
import '../theme.dart';

/// The optional desktop account/publishing feature has one settings entry point.
/// Private discovery, pairing and device grants never depend on this module.
const commandCenterEnabled = bool.fromEnvironment(
  'WISPNET_COMMAND_CENTER',
  defaultValue: true,
);

class CommandCenterModule extends StatelessWidget {
  const CommandCenterModule({
    super.key,
    required this.backend,
    required this.hostStatus,
    required this.onChanged,
    this.serviceId,
    this.enabled = commandCenterEnabled,
  });

  final WispnetBackend backend;
  final Map<String, dynamic> hostStatus;
  final String? serviceId;
  final VoidCallback onChanged;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    if (!enabled) return const SizedBox.shrink();
    final services = (hostStatus['host']?['services'] as List? ?? [])
        .cast<Map<String, dynamic>>();
    final service = services.where((s) => s['id'] == serviceId).firstOrNull;
    if (serviceId != null && service?['kind'] != 'http') {
      return const SizedBox.shrink();
    }
    return _CommandCenterPanel(
      key: ValueKey('command-center-$serviceId'),
      backend: backend,
      hostStatus: hostStatus,
      service: service,
      onChanged: onChanged,
    );
  }
}

class _CommandCenterPanel extends StatefulWidget {
  const _CommandCenterPanel({
    super.key,
    required this.backend,
    required this.hostStatus,
    required this.service,
    required this.onChanged,
  });

  final WispnetBackend backend;
  final Map<String, dynamic> hostStatus;
  final Map<String, dynamic>? service;
  final VoidCallback onChanged;

  @override
  State<_CommandCenterPanel> createState() => _CommandCenterPanelState();
}

class _CommandCenterPanelState extends State<_CommandCenterPanel> {
  Map<String, dynamic>? status;
  Map<String, dynamic>? pending;
  String? error;
  // Mutation warnings describe an outcome, not current connectivity. A routine
  // status refresh must not hide an unconfirmed remote revocation.
  String? warning;
  bool busy = false;
  bool refreshing = false;
  Timer? timer;

  bool get linked => status?['linked'] == true;
  Map<String, dynamic>? get publication =>
      (widget.hostStatus['public_access']?['services'] as List? ?? [])
          .cast<Map<String, dynamic>>()
          .where(
            (p) =>
                p['service'] == widget.service?['id'] &&
                p['incarnation'] == widget.service?['incarnation'],
          )
          .firstOrNull;
  Map<String, dynamic>? get credential =>
      (status?['credentials'] as List? ?? [])
          .cast<Map<String, dynamic>>()
          .where(
            (c) =>
                c['service'] == widget.service?['id'] &&
                c['incarnation'] == widget.service?['incarnation'],
          )
          .firstOrNull;

  @override
  void initState() {
    super.initState();
    unawaited(refresh());
    timer = Timer.periodic(const Duration(seconds: 3), (_) {
      if (!busy) unawaited(refresh(poll: pending != null));
    });
  }

  @override
  void dispose() {
    timer?.cancel();
    super.dispose();
  }

  Future<Map<String, dynamic>> request(Map<String, dynamic> payload) =>
      widget.backend.invoke('desktopRequest', {'payload': payload});

  Future<void> refresh({bool poll = false}) async {
    if (refreshing) return;
    refreshing = true;
    try {
      if (poll) await request({'op': 'command_center_poll'});
      final next = await request({'op': 'command_center_status'});
      if (mounted) {
        setState(() {
          status = next;
          pending = (next['pending'] as Map?)?.cast<String, dynamic>();
          error = null;
        });
      }
    } catch (_) {
      if (mounted) {
        setState(
          () => error =
              'Command Center is unavailable. Private access still works.',
        );
      }
    } finally {
      refreshing = false;
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
      await refresh();
    } catch (e) {
      if (mounted) {
        setState(
          () => error = e is FormatException
              ? e.message
              : 'Command Center could not complete this action.',
        );
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> connect() async {
    final result = await request({'op': 'command_center_link'});
    if (mounted) setState(() => pending = result);
    // The adapter resolves this URL from owner-local state, never UI input.
    await widget.backend.invoke('launchCommandCenter', {'purpose': 'link'});
  }

  Future<void> confirm(
    String title,
    String detail,
    Map<String, dynamic> payload,
  ) async {
    await showWispSheet<void>(
      context,
      title: title,
      child: _CommandForm(
        submitLabel: title,
        danger: true,
        action: () async {
          final result = await request(payload);
          if (mounted && payload['op'] == 'command_center_unlink') {
            final returnedWarning = result['warning'] as String?;
            setState(() {
              warning = result['remote_revocation'] == 'unconfirmed'
                  ? (returnedWarning?.isNotEmpty == true
                        ? returnedWarning
                        : 'This computer is disconnected, but remote credential revocation could not be confirmed. Review the host in Command Center.')
                  : returnedWarning;
            });
          }
          widget.onChanged();
          await refresh();
        },
        content: [Text(detail)],
      ),
    );
  }

  Future<void> publish() async {
    final service = widget.service!;
    await showWispSheet<void>(
      context,
      title: 'Enable public access',
      child: _PublishForm(
        service: service,
        hostName: widget.hostStatus['host_name'] as String,
        mcpPath: publication?['mcp_path'] as String? ?? '/mcp',
        request: request,
        onChanged: () {
          widget.onChanged();
          unawaited(refresh());
        },
      ),
    );
  }

  Future<void> backendAuthentication() async {
    await showWispSheet<void>(
      context,
      title: 'Backend authentication',
      child: _BackendAuthForm(
        service: widget.service!,
        credential: credential,
        request: request,
        onChanged: () {
          widget.onChanged();
          unawaited(refresh());
        },
      ),
    );
  }

  ButtonStyle get dangerStyle => OutlinedButton.styleFrom(
    foregroundColor: WispColors.error,
    side: const BorderSide(color: WispColors.error),
  ).copyWith(backgroundColor: wispButtonBackground(const Color(0xff401e2a)));

  Widget action(
    String label,
    Future<void> Function() callback, {
    bool enabled = true,
    bool danger = false,
  }) => Padding(
    padding: const EdgeInsets.only(bottom: 8),
    child: OutlinedButton(
      onPressed: busy || !enabled ? null : () => run(callback),
      style: danger ? dangerStyle : null,
      child: Text(label),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final account = status?['account'] as Map?;
    final published = publication?['enabled'] == true;
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: WispPanel(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const Text(
              'Command Center',
              style: TextStyle(fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 8),
            const Text('Private device access does not require an account.'),
            if (error != null) ...[
              const SizedBox(height: 8),
              Text(error!, style: const TextStyle(color: WispColors.error)),
            ],
            if (warning != null) ...[
              const SizedBox(height: 8),
              Semantics(
                liveRegion: true,
                child: Text(
                  warning!,
                  style: const TextStyle(color: WispColors.amber),
                ),
              ),
              action('Open Command Center', () async {
                await widget.backend.invoke('launchCommandCenter', {
                  'purpose': 'dashboard',
                });
              }),
            ],
            if (status == null && error == null)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 12),
                child: LinearProgressIndicator(),
              ),
            if (!linked && status != null) ...[
              const SizedBox(height: 8),
              const Text('Connect your GitHub account to publish public URLs.'),
              action('Connect GitHub account', connect),
            ],
            if (pending != null && !linked) ...[
              const Text('Approve this computer in your browser.'),
              if (pending!['user_code'] is String)
                SelectableText(pending!['user_code'] as String),
              if (pending!['verification_url'] is String)
                SelectableText(pending!['verification_url'] as String),
              action('Open sign-in page', () async {
                await widget.backend.invoke('launchCommandCenter', {
                  'purpose': 'link',
                });
              }),
              action('Check connection', () => refresh(poll: true)),
            ],
            if (linked) ...[
              const SizedBox(height: 8),
              Text('Connected as ${account?['name'] ?? 'GitHub user'}'),
              action('Manage public access', () async {
                await widget.backend.invoke('launchCommandCenter', {
                  'purpose': 'dashboard',
                });
              }),
              action(
                'Disconnect account',
                () => confirm(
                  'Disconnect account',
                  'Stop this computer’s public publications and disconnect its account. Private pairing and device grants stay in place.',
                  {'op': 'command_center_unlink'},
                ),
              ),
            ],
            if (widget.service != null) ...[
              const SizedBox(height: 12),
              if (credential?['mode'] == 'bearer' &&
                  credential?['configured'] == true)
                const Text('Backend bearer token is stored on this computer.'),
              action(
                'Backend authentication',
                backendAuthentication,
                enabled: status != null,
              ),
              if (published)
                action(
                  'Disable public access',
                  () => confirm(
                    'Disable public access',
                    'Stop public access to “${widget.service!['id']}”? Approved paired devices will keep their private access.',
                    {
                      'op': 'configure_public',
                      'service': widget.service!['id'],
                      'enabled': false,
                    },
                  ),
                  danger: true,
                )
              else
                action(
                  'Enable public access',
                  publish,
                  enabled: linked,
                  danger: true,
                ),
            ],
          ],
        ),
      ),
    );
  }
}

class _CommandForm extends StatefulWidget {
  const _CommandForm({
    required this.submitLabel,
    required this.action,
    required this.content,
    this.danger = false,
  });
  final String submitLabel;
  final Future<void> Function() action;
  final List<Widget> content;
  final bool danger;
  @override
  State<_CommandForm> createState() => _CommandFormState();
}

class _CommandFormState extends State<_CommandForm> {
  bool busy = false;
  String? error;
  Future<void> submit() async {
    setState(() {
      busy = true;
      error = null;
    });
    try {
      await widget.action();
      if (mounted) Navigator.pop(context);
    } catch (e) {
      if (mounted) {
        setState(
          () => error = e is FormatException
              ? e.message
              : 'Could not save this change.',
        );
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      ...widget.content,
      const SizedBox(height: 16),
      if (error != null)
        Text(error!, style: const TextStyle(color: WispColors.error)),
      FilledButton(
        onPressed: busy ? null : submit,
        style: widget.danger
            ? FilledButton.styleFrom(foregroundColor: WispColors.error)
            : null,
        child: Text(widget.submitLabel),
      ),
      TextButton(
        onPressed: busy ? null : () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
    ],
  );
}

class _PublishForm extends StatefulWidget {
  const _PublishForm({
    required this.service,
    required this.hostName,
    required this.mcpPath,
    required this.request,
    required this.onChanged,
  });
  final Map<String, dynamic> service;
  final String hostName, mcpPath;
  final Future<Map<String, dynamic>> Function(Map<String, dynamic>) request;
  final VoidCallback onChanged;
  @override
  State<_PublishForm> createState() => _PublishFormState();
}

class _PublishFormState extends State<_PublishForm> {
  late final path = TextEditingController(text: widget.mcpPath);
  @override
  void dispose() {
    path.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => _CommandForm(
    submitLabel: 'Enable public access',
    danger: true,
    content: [
      Text(
        'Publish “${widget.service['id']}” through Command Center. Public callers must be authorized through OAuth or a service bearer key.',
      ),
      const SizedBox(height: 12),
      TextField(
        controller: path,
        decoration: const InputDecoration(
          labelText: 'MCP endpoint path',
          helperText: 'Match the service’s endpoint, including letter case.',
          helperMaxLines: 2,
        ),
      ),
    ],
    action: () async {
      final value = path.text.trim();
      final uri = Uri.tryParse(value);
      if (value.length > 2048 ||
          !value.startsWith('/') ||
          value.startsWith('//') ||
          uri == null ||
          uri.hasScheme ||
          uri.hasAuthority ||
          uri.hasQuery ||
          uri.hasFragment ||
          value.contains('\\') ||
          RegExp(r'[\x00-\x20\x7f]').hasMatch(value)) {
        throw const FormatException(
          'Enter an endpoint path such as /mcp or /MCP.',
        );
      }
      await widget.request({
        'op': 'configure_public',
        'service': widget.service['id'],
        'incarnation': widget.service['incarnation'],
        'enabled': true,
        'name': widget.hostName,
        'managed': true,
        'mcp_path': value,
      });
      widget.onChanged();
    },
  );
}

class _BackendAuthForm extends StatefulWidget {
  const _BackendAuthForm({
    required this.service,
    required this.credential,
    required this.request,
    required this.onChanged,
  });
  final Map<String, dynamic> service;
  final Map<String, dynamic>? credential;
  final Future<Map<String, dynamic>> Function(Map<String, dynamic>) request;
  final VoidCallback onChanged;
  @override
  State<_BackendAuthForm> createState() => _BackendAuthFormState();
}

class _BackendAuthFormState extends State<_BackendAuthForm> {
  final token = TextEditingController();
  late String mode = widget.credential?['mode'] as String? ?? 'passthrough';
  @override
  void dispose() {
    token.clear();
    token.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => _CommandForm(
    submitLabel: 'Save backend authentication',
    content: [
      const Text(
        'Configure authentication for the application running on this computer. Stored tokens are not sent to the command center or paired devices.',
      ),
      const SizedBox(height: 12),
      DropdownButtonFormField<String>(
        initialValue: mode,
        decoration: const InputDecoration(labelText: 'Backend authentication'),
        items: const [
          DropdownMenuItem(
            value: 'passthrough',
            child: Text('Use caller’s credentials'),
          ),
          DropdownMenuItem(
            value: 'none',
            child: Text('No backend authentication'),
          ),
          DropdownMenuItem(
            value: 'bearer',
            child: Text('Host-managed bearer token'),
          ),
        ],
        onChanged: (next) => setState(() {
          mode = next!;
          token.clear();
        }),
      ),
      if (mode == 'bearer') ...[
        const SizedBox(height: 12),
        TextField(
          key: const ValueKey('backend-bearer-input'),
          controller: token,
          obscureText: true,
          enableSuggestions: false,
          autocorrect: false,
          decoration: InputDecoration(
            labelText: widget.credential?['configured'] == true
                ? 'Replacement bearer token'
                : 'Bearer token',
          ),
        ),
      ],
    ],
    action: () async {
      final value = token.text;
      token.clear();
      if (mode == 'bearer' &&
          (value.isEmpty || RegExp(r'[\r\n\x00]').hasMatch(value))) {
        throw const FormatException(
          'Enter a bearer token without line breaks.',
        );
      }
      await widget.request({
        'op': 'configure_backend_auth',
        'service': widget.service['id'],
        'incarnation': widget.service['incarnation'],
        'mode': mode,
        if (mode == 'bearer') 'token': value,
      });
      widget.onChanged();
    },
  );
}
