import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend.dart';
import 'theme.dart';

String failureMessage(Object error) {
  final message = error is FormatException
      ? error.message
      : error is PlatformException
      ? error.message ?? error.code
      : error.toString();
  if (error is PlatformException && error.code == 'invitation_denied') {
    return 'This host did not accept the pairing code. Generate a new code on your computer.';
  }
  if (message.toLowerCase().contains('expired')) {
    return 'Pairing code expired or invalid. Generate a new code on your computer.';
  }
  return message;
}

class PairingForm extends StatefulWidget {
  const PairingForm({
    super.key,
    required this.backend,
    required this.onPaired,
    this.initialCode,
    this.desktop = false,
  });
  final WispnetBackend backend;
  final VoidCallback onPaired;
  final String? initialCode;
  final bool desktop;
  @override
  State<PairingForm> createState() => _PairingFormState();
}

class _PairingFormState extends State<PairingForm> {
  late final TextEditingController code = TextEditingController(
    text: widget.initialCode,
  )..addListener(changed);
  bool busy = false, error = false;
  late int tab = widget.initialCode == null && !widget.desktop ? 0 : 1;
  String feedback = '';
  Map<String, dynamic>? preview;
  @override
  void initState() {
    super.initState();
    if (widget.initialCode != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => review());
    }
  }

  @override
  void dispose() {
    code.dispose();
    super.dispose();
  }

  void changed() {
    setState(() {
      preview = null;
      feedback = '';
    });
  }

  String normalized() {
    final value = code.text.replaceAll(RegExp(r'\s+'), '');
    if (!value.startsWith('wispnet1:') || utf8.encode(value).length > 8192) {
      throw const FormatException(
        'Paste the complete Wispnet pairing code or scan its QR.',
      );
    }
    return value;
  }

  Future<void> review() async {
    if (busy) return;
    setState(() {
      busy = true;
      error = false;
      feedback = 'Checking pairing code…';
    });
    try {
      final result = await widget.backend.invoke('preview', {
        'code': normalized(),
      });
      if (mounted) {
        setState(() {
          preview = result;
          feedback = 'Pairing code ready. Request pairing, then approve it on your computer.';
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          error = true;
          feedback = failureMessage(e);
        });
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> pair() async {
    if (busy) return;
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      busy = true;
      error = false;
      feedback = 'Requesting pairing… This may take up to a minute.';
    });
    try {
      await widget.backend.invoke('pair', {'code': normalized()});
      if (mounted) widget.onPaired();
    } catch (e) {
      if (mounted) {
        setState(() {
          error = true;
          feedback = failureMessage(e);
        });
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> qr(String method) async {
    if (busy) return;
    setState(() {
      busy = true;
      error = false;
      feedback = method == 'scanQr'
          ? 'Opening camera…'
          : 'Choose a pairing QR image…';
    });
    try {
      final result = await widget.backend.invoke(method);
      if (!mounted) return;
      final scanned = result['code'] as String?;
      if (scanned == null) {
        setState(() {
          busy = false;
          feedback =
              'Scan cancelled. Import a QR image or paste a pairing code.';
        });
        return;
      }
      code.text = scanned;
      setState(() {
        busy = false;
        tab = 1;
      });
      await pair();
    } catch (e) {
      if (mounted) {
        setState(() {
          busy = false;
          error = true;
          feedback = failureMessage(e);
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: !busy,
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (!widget.desktop) ...[
          WispPanel(
            radius: 14,
            color: const Color(0xff0d1526),
            padding: const EdgeInsets.all(4),
            child: Row(
              children: [
                for (final entry in {
                  if (!widget.desktop) 0: 'QR',
                  1: 'Pairing Code',
                }.entries)
                  Expanded(
                    child: TextButton(
                      onPressed: busy
                          ? null
                          : () => setState(() => tab = entry.key),
                      style: TextButton.styleFrom(
                        foregroundColor: tab == entry.key
                            ? WispColors.text
                            : WispColors.muted,
                        backgroundColor: tab == entry.key
                            ? const Color(0xff1f3666)
                            : Colors.transparent,
                        minimumSize: const Size(0, 44),
                        shape: RoundedRectangleBorder(
                          borderRadius: BorderRadius.circular(10),
                        ),
                      ),
                      child: Text(entry.value),
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(height: 14),
        ],
        if (tab == 0) ...[
          FilledButton(
            onPressed: busy ? null : () => qr('scanQr'),
            child: const Text('Scan with camera'),
          ),
          const SizedBox(height: 10),
          OutlinedButton(
            onPressed: busy ? null : () => qr('importQr'),
            child: const Text('Import from photos'),
          ),
        ] else ...[
          const Text(
            'Paste the pairing code from your computer.',
            style: TextStyle(color: WispColors.muted),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: code,
            enabled: !busy,
            minLines: 4,
            maxLines: 6,
            autocorrect: false,
            enableSuggestions: false,
            decoration: const InputDecoration(
              hintText: 'Pairing code',
              labelText: 'Pairing code',
            ),
          ),
          const SizedBox(height: 10),
          FilledButton(
            onPressed: busy || code.text.trim().isEmpty ? null : pair,
            child: Text(busy ? 'Pairing…' : 'Pair'),
          ),
        ],
        if (feedback.isNotEmpty)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 12),
            child: Semantics(
              liveRegion: true,
              child: Text(
                feedback,
                style: TextStyle(
                  color: error ? WispColors.error : const Color(0xffb8d4ff),
                ),
              ),
            ),
          ),
        if (preview != null)
          WispPanel(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Text('Pair with this machine'),
                const SizedBox(height: 8),
                Text(
                  preview!['host_name'] as String? ?? 'Host',
                  style: const TextStyle(
                    fontSize: 16,
                    fontWeight: FontWeight.bold,
                  ),
                ),
                const SizedBox(height: 8),
                SelectableText(
                  preview!['host'] as String,
                  style: const TextStyle(fontSize: 12, color: WispColors.muted),
                ),
                const SizedBox(height: 12),
                FilledButton(
                  onPressed: busy ? null : pair,
                  child: const Text('Request pairing'),
                ),
              ],
            ),
          ),
      ],
    ),
  );
}

class MachineSettings extends StatefulWidget {
  const MachineSettings({
    super.key,
    required this.machine,
    required this.backend,
    required this.onChanged,
  });
  final Machine machine;
  final WispnetBackend backend;
  final VoidCallback onChanged;
  @override
  State<MachineSettings> createState() => _MachineSettingsState();
}

class _MachineSettingsState extends State<MachineSettings> {
  bool busy = false;
  String? error;
  Future<void> perform(String method) async {
    setState(() {
      busy = true;
      error = null;
    });
    try {
      await widget.backend.invoke(method, {'host': widget.machine.id});
      widget.onChanged();
      if (mounted) Navigator.pop(context);
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  ButtonStyle get dangerStyle => OutlinedButton.styleFrom(
    foregroundColor: WispColors.error,
    side: const BorderSide(color: WispColors.error),
  ).copyWith(backgroundColor: wispButtonBackground(const Color(0xff401e2a)));

  Future<void> confirmRemove() async {
    final confirmed = await showWispSheet<bool>(
      context,
      title: 'Remove machine?',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Remove ${widget.machine.name} and close its Wispnet access on this computer? This computer will forget the pairing and close its local service routes. The other machine will learn about removal on its next contact. You can pair again later.',
          ),
          const SizedBox(height: 16),
          FilledButton(
            style: dangerStyle,
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Remove machine'),
          ),
          const SizedBox(height: 12),
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
        ],
      ),
    );
    if (confirmed == true && mounted) await perform('removeHost');
  }

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      SelectableText(
        'Device ID\n${widget.machine.name}\n\nHost ID\n${widget.machine.id}',
        style: const TextStyle(color: WispColors.muted),
      ),
      const SizedBox(height: 16),
      OutlinedButton(
        style: dangerStyle,
        onPressed: busy ? null : confirmRemove,
        child: const Text('Remove'),
      ),
      if (error != null)
        Padding(
          padding: const EdgeInsets.only(top: 12),
          child: Text(error!, style: const TextStyle(color: WispColors.error)),
        ),
    ],
  );
}

String diagnosticObserved(Object? value) => value == null
    ? '?'
    : value == true
    ? '✓'
    : '✗';

void appendDiagnosticObservation(
  List<String> lines,
  String label,
  Map side, {
  Map? routerMapping,
}) {
  final network = side['network'] as Map?;
  final n = network?['report'] as Map?;
  final p = side['paths'] as Map?;
  lines.add('\n$label');
  if (side.containsKey('relay_only')) {
    lines.add(
      '• ${side['relay_only'] == true ? "✗" : "✓"} Direct UDP transports enabled',
    );
  }
  final mapping = routerMapping;
  if (mapping == null) {
    lines.add('• ? Automatic router mapping status unavailable');
  } else {
    lines.add(
      '• ${diagnosticObserved(mapping['enabled'])} Automatic router mapping enabled',
    );
    lines.add(
      '• Router responses: UPnP ${diagnosticObserved(mapping['upnp'])} · PCP ${diagnosticObserved(mapping['pcp'])} · NAT-PMP ${diagnosticObserved(mapping['nat_pmp'])}',
    );
    lines.add(
      '• ${diagnosticObserved(mapping['active'])} Router mapping granted${mapping['active'] == true ? " · UDP ${mapping['external_port']}" : ""}',
    );
    if (mapping['active'] == true) {
      lines.add(
        '• ${diagnosticObserved(mapping['public_address'])} Mapped address is public',
      );
      lines.add(
        '• A granted mapping does not prove reachability; see the direct-path result.',
      );
    }
  }
  if (p != null) {
    lines.add(
      '• ${p['initial_path']} → ${p['selected_path']} · RTT: ${p['rtt_ms'] ?? "unavailable"} ms',
    );
    lines.add(
      '• Observed for ${side['elapsed_ms']} ms · relay: ${side['relay_status']}',
    );
  }
  lines.add(
    network?['fresh'] == true
        ? '• Discovery: network report updated during this test (${network?['wait_ms']} ms)'
        : n != null
        ? '• Discovery: cached; no new report observed during this test'
        : '• Discovery: unavailable during this test',
  );
  for (final check in <(String, Object?)>[
    ('UDP response IPv4', n?['udp_v4']),
    ('UDP response IPv6', n?['udp_v6']),
    ('Public IPv4 address discovered', n?['public_ipv4_discovered']),
    ('Public IPv6 address discovered', n?['public_ipv6_discovered']),
    ('NAT mapping varies by destination', n?['mapping_varies']),
    ('Captive portal suspected (heuristic)', n?['captive_portal']),
  ]) {
    lines.add('• ${diagnosticObserved(check.$2)} ${check.$1}');
  }
  if (n != null &&
      (n['udp_v4'] == true || n['udp_v6'] == true) &&
      n['public_ipv4_discovered'] == false &&
      n['public_ipv6_discovered'] == false) {
    lines.add(
      '• Discovery responded, but no public IP was found. A discovery server behind the same router can return a private LAN address.',
    );
  }
  if (n?['mapping_varies'] == true) {
    lines.add(
      '• Destination-dependent NAT can make direct connections harder.',
    );
  }
  if (n != null && n['udp_v4'] == false && n['udp_v6'] == false) {
    lines.add(
      '• No UDP discovery response in this report. This alone does not prove UDP is blocked.',
    );
  }
  if (side.containsKey('peer_ip_candidates')) {
    lines.add(
      '• Known peer IP addresses in transport cache: ${side['peer_ip_candidates'] ?? "unavailable"}',
    );
  }
  lines.add(
    '• Hole-punch candidate list: unavailable in the transport API; cache count does not measure address exchange.',
  );
  if (p != null) {
    lines.add(
      '• Direct paths seen: ${p['direct_paths_seen']} · closed: ${p['direct_paths_closed']}',
    );
    lines.add(
      '• Datagrams on direct paths: sent ${p['direct_datagrams_sent']} · received ${p['direct_datagrams_received']}',
    );
    lines.add(
      '• Validation on direct paths: challenges sent ${p['direct_challenges_sent']} · responses received ${p['direct_responses_received']}',
    );
    lines.add(
      '• Validation across all paths (includes relay): challenges sent ${p['connection_challenges_sent']} · responses received ${p['connection_responses_received']}',
    );
    if ((p['events_lost'] as num? ?? 0) > 0) {
      lines.add(
        '• Some path events were lost; activity counts may be incomplete.',
      );
    }
  }
}

class ConnectionDiagnostics extends StatefulWidget {
  const ConnectionDiagnostics({super.key, required this.backend});
  final WispnetBackend backend;
  @override
  State<ConnectionDiagnostics> createState() => _ConnectionDiagnosticsState();
}

class _ConnectionDiagnosticsState extends State<ConnectionDiagnostics> {
  bool running = false;
  String report = '';
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) run();
    });
  }

  Future<void> run() async {
    setState(() {
      running = true;
      report = 'Refreshing discovery and testing paired devices…';
    });
    final lines = <String>[
      'Connection diagnostic · ${DateTime.now().toLocal().toIso8601String().split(".").first.replaceFirst("T", " ")}',
      '✓ Yes · ✗ No · ? Unavailable',
    ];
    try {
      final snapshot = await widget.backend.invoke('snapshot');
      final hosts = snapshot['hosts'] as Map? ?? {};
      if (hosts.isEmpty) lines.add('No paired devices to test.');
      for (final host in hosts.keys) {
        if (!mounted) return;
        setState(
          () => report =
              '${lines.join('\n')}\nTesting ${hosts[host]['name']}… (up to 55 seconds)',
        );
        final result = await widget.backend.invoke('connectionDiagnostics', {
          'host': host,
        });
        lines.add(
          '• Network: ${result['network']} · VPN: ${result['vpn'] == null
              ? "?"
              : result['vpn'] == true
              ? "✓"
              : "✗"}',
        );
        for (final peer in result['peers'] as List? ?? []) {
          lines.add('\n${peer['name']}');
          if (peer['error'] != null) {
            lines.add('• ? Could not verify connection: ${peer['error']}');
            continue;
          }
          final r = peer['report'] as Map;
          lines.add(
            '• Path: ${r['initial_path']} → ${r['selected_path']} · RTT: ${r['rtt_ms'] ?? "unavailable"} ms',
          );
          lines.add(
            '• Observed for ${r['observation_ms']} ms · relay: ${r['relay_status']}',
          );
          lines.add('• ${r['reason']}');
          final transport = r['transport'] as Map?;
          if (transport != null) {
            lines.add('\nPhone relay connections');
            for (final relay in transport['relays'] as List? ?? []) {
              lines.add(
                '• ${diagnosticObserved(relay['connected'])} ${relay['url']}${relay['preferred'] == true ? " · preferred" : ""}',
              );
            }
            lines.add('Phone discovery observations');
            for (final discovery in transport['discovery'] as List? ?? []) {
              lines.add(
                '• ${discovery['url']} · IPv4 ${diagnosticObserved(discovery['ipv4'])} · IPv6 ${diagnosticObserved(discovery['ipv6'])}',
              );
            }
          }
          final local = r['local'] as Map?;
          final mapping = r['router_mapping'] as Map?;
          appendDiagnosticObservation(
            lines,
            'Phone observations',
            local ??
                {
                  'network': {'fresh': false, 'report': r['network_report']},
                },
            routerMapping: mapping?['local'] as Map?,
          );
          final remote = r['remote'] as Map?;
          if (remote != null) {
            appendDiagnosticObservation(
              lines,
              'Target computer observations',
              remote,
              routerMapping: mapping?['remote'] as Map?,
            );
          } else {
            lines.add(
              '\n• ? ${r['remote_error'] ?? "Target computer observations unavailable; update the host to collect both ends."}',
            );
          }
          lines.add(
            '• ? Failed candidate validation reasons: unavailable in the transport API. Zero observed paths does not prove no attempts were made.',
          );
        }
        if (result['network_changed'] == true) {
          lines.add(
            '• Network changed during the test; run again on a stable connection.',
          );
        }
      }
      lines.add(
        '\nResults describe this test. Normal device badges show the last request’s path.',
      );
    } catch (e) {
      lines.add(failureMessage(e));
    }
    if (mounted) {
      setState(() {
        running = false;
        report = lines.join('\n');
      });
    }
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: const Text('Diagnostic')),
    body: SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(24),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Expanded(
                  child: TextButton.icon(
                    onPressed: running ? null : run,
                    icon: const Icon(Icons.network_check),
                    label: Text(running ? 'Testing…' : 'Run again'),
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: TextButton.icon(
                    onPressed: report.isEmpty
                        ? null
                        : () => Clipboard.setData(ClipboardData(text: report)),
                    icon: const Icon(Icons.copy),
                    label: const Text('Copy report'),
                  ),
                ),
              ],
            ),
            if (report.isNotEmpty) ...[
              const SizedBox(height: 8),
              SelectableText(report),
            ],
          ],
        ),
      ),
    ),
  );
}

class ServiceLinks extends StatefulWidget {
  const ServiceLinks({
    super.key,
    required this.backend,
    required this.machine,
    required this.service,
    required this.android,
    required this.canResolve,
  });
  final WispnetBackend backend;
  final Machine machine;
  final Map<String, dynamic> service;
  final bool android, canResolve;
  @override
  State<ServiceLinks> createState() => _ServiceLinksState();
}

class _ServiceLinksState extends State<ServiceLinks> {
  String? localUrl, error;
  bool loading = true;
  @override
  void initState() {
    super.initState();
    load();
  }

  Future<void> load() async {
    setState(() {
      loading = true;
      error = null;
    });
    try {
      final args = {'host': widget.machine.id, 'service': widget.service['id']};
      final settings = await widget.backend.invoke('settings', args);
      if (!mounted) return;
      var url = settings['url'] as String? ?? '';
      if (url.isEmpty) {
        if (!widget.canResolve) {
          throw const FormatException(
            'Go online and refresh this service to get its local URL.',
          );
        }
        final route = await widget.backend.invoke('open', {
          ...args,
          'incarnation': widget.service['incarnation'],
        });
        url = route['url'] as String? ?? '';
      }
      if (url.isEmpty) throw const FormatException('Local URL unavailable.');
      if (mounted) setState(() => localUrl = url);
    } catch (failure) {
      if (mounted) setState(() => error = failureMessage(failure));
    } finally {
      if (mounted) setState(() => loading = false);
    }
  }

  Widget option(
    String label,
    String url,
    String tooltip, {
    String? description,
  }) => Padding(
    padding: const EdgeInsets.only(bottom: 12),
    child: WispPanel(
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  label,
                  style: const TextStyle(fontWeight: FontWeight.bold),
                ),
                if (description != null) ...[
                  const SizedBox(height: 6),
                  Text(
                    description,
                    style: const TextStyle(
                      color: WispColors.muted,
                      fontSize: 13,
                    ),
                  ),
                ],
                const SizedBox(height: 6),
                SelectableText(
                  url,
                  style: const TextStyle(color: WispColors.cyan),
                ),
              ],
            ),
          ),
          IconButton(
            tooltip: tooltip,
            onPressed: () async {
              await Clipboard.setData(ClipboardData(text: url));
              if (mounted) {
                ScaffoldMessenger.of(context)
                    .showSnackBar(const SnackBar(content: Text('Link copied')));
              }
            },
            icon: const Icon(Icons.copy, color: WispColors.cyan),
          ),
        ],
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final publication = widget.service['public_access'] as Map?;
    final publicUrl = publication?['url'] as String?;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (localUrl != null)
          option('Local URL', localUrl!, 'Copy local URL')
        else
          WispPanel(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Text(
                  'Local URL',
                  style: TextStyle(fontWeight: FontWeight.bold),
                ),
                const SizedBox(height: 6),
                Text(
                  loading ? 'Loading…' : error ?? 'Local URL unavailable.',
                  style: TextStyle(
                    color: loading ? WispColors.muted : WispColors.error,
                  ),
                ),
                if (!loading)
                  TextButton(onPressed: load, child: const Text('Retry')),
              ],
            ),
          ),
        if (localUrl == null) const SizedBox(height: 12),
        if (widget.android)
          option(
            'Wispnet URL',
            'wispnet://${widget.machine.id}/${widget.service['id']}',
            'Copy Wispnet link',
            description: 'Stays the same across devices, even if their local ports differ. Use this for a consistent link to the service.',
          ),
        if (publication?['enabled'] == true &&
            publicUrl != null &&
            publicUrl.isNotEmpty)
          option('Public URL', publicUrl, 'Copy public URL'),
      ],
    );
  }
}

class SettingsForm extends StatefulWidget {
  const SettingsForm({
    super.key,
    required this.backend,
    required this.deviceName,
    required this.onSaved,
  });
  final WispnetBackend backend;
  final String deviceName;
  final VoidCallback onSaved;
  @override
  State<SettingsForm> createState() => _SettingsFormState();
}

class _SettingsFormState extends State<SettingsForm> {
  late final name = TextEditingController(text: widget.deviceName);
  final port = TextEditingController();
  bool loading = true, saving = false, opening = false;
  String? error;
  String address = '';
  @override
  void initState() {
    super.initState();
    load();
  }

  @override
  void dispose() {
    name.dispose();
    port.dispose();
    super.dispose();
  }

  Future<void> load() async {
    try {
      final result = await widget.backend.invoke('settings');
      if (!mounted) return;
      port.text = '${result['port'] ?? ''}';
      setState(() {
        loading = false;
        address = result['url'] as String? ?? '';
      });
    } catch (e) {
      if (mounted) {
        setState(() {
          loading = false;
          error = failureMessage(e);
        });
      }
    }
  }

  String get browserAddress =>
      Uri.parse(address).replace(path: '/v1/services').toString();

  Future<void> openAddress() async {
    if (opening || address.isEmpty) return;
    setState(() {
      opening = true;
      error = null;
    });
    try {
      await widget.backend.invoke('launch', {'url': browserAddress});
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    } finally {
      if (mounted) setState(() => opening = false);
    }
  }

  Future<void> save() async {
    final raw = port.text.trim();
    final number = int.tryParse(raw);
    if (number == null || (number < 1024 || number > 65535)) {
      setState(() => error = 'Port must be between 1024 and 65535.');
      return;
    }
    final value = name.text.trim();
    if (!RegExp(r'^[a-z0-9][a-z0-9._-]{0,63}$').hasMatch(value)) {
      setState(
        () => error = 'Use 1–64 lowercase letters, numbers, dots, underscores or hyphens; start with a letter or number.',
      );
      return;
    }
    setState(() {
      saving = true;
      error = null;
    });
    try {
      await widget.backend.invoke('saveSettings', {
        'port': number,
        'name': value,
      });
      if (mounted) widget.onSaved();
    } catch (e) {
      if (mounted) setState(() => error = failureMessage(e));
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      TextField(
        controller: name,
        enabled: !saving,
        decoration: const InputDecoration(labelText: 'Device ID'),
      ),
      const SizedBox(height: 12),
      const Text(
        'One HTTP port for all Wispnet services and the registry on this device. Default: 7842. Changes reconnect active sessions.',
        style: TextStyle(color: WispColors.muted),
      ),
      const SizedBox(height: 16),
      TextField(
        controller: port,
        enabled: !loading && !saving,
        keyboardType: TextInputType.number,
        decoration: InputDecoration(
          floatingLabelBehavior: FloatingLabelBehavior.always,
          labelText: 'Wispnet port',
          hintText: '7842',
        ),
      ),
      if (address.isNotEmpty)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 12),
          child: Align(
            alignment: Alignment.centerLeft,
            child: Semantics(
              link: true,
              child: Tooltip(
                message: 'Open registry JSON in browser',
                child: WispInkWell(
                  key: const ValueKey('settings-address'),
                  onTap: opening || saving ? null : openAddress,
                  child: Text(
                    browserAddress,
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
        ),
      if (error != null)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 12),
          child: Text(error!, style: const TextStyle(color: WispColors.error)),
        ),
      const SizedBox(height: 12),
      FilledButton(
        onPressed: loading || saving ? null : save,
        child: Text(
          loading
              ? 'Loading…'
              : saving
              ? 'Saving…'
              : 'Save',
        ),
      ),
      if (Theme.of(context).platform == TargetPlatform.android) ...[
        const SizedBox(height: 12),
        OutlinedButton(
          onPressed: saving
              ? null
              : () {
                  FocusManager.instance.primaryFocus?.unfocus();
                  Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) =>
                          ConnectionDiagnostics(backend: widget.backend),
                    ),
                  );
                },
          child: const Text('Diagnostic'),
        ),
      ],
    ],
  );
}
