import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend.dart';
import 'space.dart';
import 'theme.dart';
import 'sheets.dart';
import 'cards.dart';
import 'header.dart';
import 'desktop_settings.dart';

class WispnetApp extends StatelessWidget {
  const WispnetApp({super.key, required this.backend});
  final WispnetBackend backend;
  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Wispnet',
    debugShowCheckedModeBanner: false,
    theme: wispTheme(),
    home: WispHome(backend: backend),
  );
}

class WispHome extends StatefulWidget {
  const WispHome({super.key, required this.backend});
  final WispnetBackend backend;
  @override
  State<WispHome> createState() => _WispHomeState();
}

class _WispHomeState extends State<WispHome>
    with SingleTickerProviderStateMixin, WidgetsBindingObserver {
  Map<String, dynamic> snapshot = {};
  String? initializationError;
  bool polling = false,
      refreshing = false,
      busy = false,
      foreground = true,
      pairingSheet = false;
  int ticks = 0, launchGeneration = 0;
  String lastNetwork = '';
  Timer? timer;
  late final AnimationController warp = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 900),
  );
  bool get online => snapshot['enabled'] == true;
  Map<String, dynamic> get records =>
      (snapshot['hosts'] as Map?)?.cast<String, dynamic>() ?? {};
  Map<String, dynamic> get order =>
      (snapshot['order'] as Map?)?.cast<String, dynamic>() ?? {};
  List<Machine> get machines =>
      orderedIds(records.keys, order['machines'] as List?)
          .map(
            (id) => Machine(id, (records[id] as Map).cast<String, dynamic>()),
          )
          .toList();
  WispnetBackend get backend => widget.backend;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(initialize());
    timer = Timer.periodic(const Duration(seconds: 2), (_) {
      if (!foreground) return;
      unawaited(poll());
      if (++ticks % 8 == 0) unawaited(refresh());
    });
  }

  Future<void> initialize() async {
    try {
      await poll(propagate: true);
      if (mounted) {
        setState(() => initializationError = null);
        unawaited(refresh());
      }
    } catch (error) {
      if (mounted) setState(() => initializationError = message(error));
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    foreground =
        state == AppLifecycleState.resumed ||
        snapshot['desktop'] == true && state == AppLifecycleState.inactive;
    if (foreground) {
      unawaited(poll());
      unawaited(refresh());
    } else {
      launchGeneration++;
    }
  }

  @override
  void dispose() {
    timer?.cancel();
    WidgetsBinding.instance.removeObserver(this);
    warp.dispose();
    super.dispose();
  }

  String message(Object error) => error is PlatformException
      ? error.message ?? error.code
      : error is MissingPluginException
      ? 'This platform adapter is not available yet.'
      : error.toString();
  void notice(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(text)));
  }

  Future<void> poll({bool propagate = false}) async {
    if (polling) return;
    polling = true;
    try {
      final next = await backend.invoke('snapshot');
      if (!mounted) return;
      final network = next['network'] as String? ?? '';
      if (next['enabled'] == true &&
          network != lastNetwork &&
          network != 'Network available' &&
          network.isNotEmpty) {
        notice(network);
      }
      lastNetwork = network;
      setState(() {
        snapshot = next;
        if (next['desktop'] == true &&
            WidgetsBinding.instance.lifecycleState ==
                AppLifecycleState.inactive) {
          foreground = true;
        }
        initializationError = null;
      });
      if (pairingSheet) return;
      final invitation = await backend.invoke('takeInvitation');
      final invitationError = invitation['error'] as String?;
      if (invitationError != null) notice(invitationError);
      final code = invitation['code'] as String?;
      if (code != null && code.isNotEmpty && mounted && !pairingSheet) {
        unawaited(pair(code: code));
      }
    } catch (error) {
      if (propagate) rethrow;
    } finally {
      polling = false;
    }
  }

  Future<void> refresh() async {
    if (refreshing || snapshot.isEmpty || !mounted) return;
    setState(() => refreshing = true);
    await Future.wait(
      machines.where((m) => m.data['local'] != true).map((machine) async {
        try {
          await backend.invoke('refreshHost', {'host': machine.id});
          await poll();
        } catch (error) {
          notice(message(error));
        }
      }),
    );
    if (mounted) setState(() => refreshing = false);
  }

  Future<void> toggle() async {
    if (busy) return;
    setState(() => busy = true);
    try {
      final enable = !online;
      await backend.invoke('setAccess', {'enabled': enable});
      if (enable) {
        final deadline = DateTime.now().add(const Duration(seconds: 15));
        while (mounted) {
          await poll();
          if (online) break;
          if (DateTime.now().isAfter(deadline)) {
            await backend.invoke('setAccess', {'enabled': false});
            throw StateError('Access could not start. Try again.');
          }
          await Future<void>.delayed(const Duration(milliseconds: 200));
        }
        if (mounted && !MediaQuery.disableAnimationsOf(context)) {
          await warp.forward(from: 0);
          if (mounted) warp.reset();
        }
        unawaited(refresh());
      } else {
        await Future<void>.delayed(const Duration(milliseconds: 100));
        await poll();
      }
    } catch (error) {
      notice(message(error));
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> open(Machine machine, Map<String, dynamic> service) async {
    if (busy || !online) return;
    final generation = ++launchGeneration;
    setState(() => busy = true);
    notice('Opening ${service['display_name']}…');
    try {
      final result = await backend.invoke('open', {
        'host': machine.id,
        'service': service['id'],
        'incarnation': service['incarnation'],
      });
      if (!mounted || !foreground || generation != launchGeneration) return;
      if (!MediaQuery.disableAnimationsOf(context)) {
        await warp.forward(from: 0);
        if (mounted) warp.reset();
      }
      if (!mounted || !foreground || generation != launchGeneration) return;
      ScaffoldMessenger.of(context).hideCurrentSnackBar();
      await backend.invoke('launch', {
        'url': result['launch_url'] ?? result['url'],
      });
    } catch (error) {
      notice('Cannot open service: ${message(error)}');
    } finally {
      if (mounted) {
        warp.reset();
        setState(() => busy = false);
        await poll();
      }
    }
  }

  Future<void> pair({String? code}) async {
    if (pairingSheet) return;
    pairingSheet = true;
    await showWispSheet<void>(
      context,
      title: 'Pair a machine',
      child: PairingForm(
        backend: backend,
        initialCode: code,
        desktop: snapshot['desktop'] == true,
        onPaired: () {
          Navigator.pop(context);
          notice('Pending owner approval');
          unawaited(poll());
        },
      ),
    );
    pairingSheet = false;
  }

  Future<void> manage() async {
    final remotes = machines.where((m) => m.data['local'] != true).toList();
    if (remotes.isEmpty) {
      await pair();
      return;
    }
    await showWispSheet<void>(
      context,
      title: 'Paired machines',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (snapshot['desktop'] == true &&
              (snapshot['pendingRequests'] as int? ?? 0) > 0)
            OutlinedButton(
              onPressed: () {
                Navigator.pop(context);
                deviceSettings(section: 'Pairing requests');
              },
              child: Text(
                'Review ${snapshot['pendingRequests']} pairing requests',
              ),
            ),
          for (final machine in remotes)
            Padding(
              padding: const EdgeInsets.only(bottom: 10),
              child: OutlinedButton(
                onPressed: () {
                  Navigator.pop(context);
                  machineSettings(machine);
                },
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 12),
                  child: Row(
                    children: [
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              '${machine.name} · ${shortId(machine.id)}',
                              style: const TextStyle(
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                            Text(
                              machine.status,
                              style: TextStyle(
                                fontSize: 12,
                                color: stateColor(machine),
                              ),
                            ),
                          ],
                        ),
                      ),
                      const Icon(
                        Icons.settings,
                        size: 24,
                        color: WispColors.muted,
                      ),
                    ],
                  ),
                ),
              ),
            ),
          FilledButton(
            onPressed: () {
              Navigator.pop(context);
              pair();
            },
            child: const Text('+ Pair another machine'),
          ),
          const SizedBox(height: 16),
          SelectableText(
            'Device ID\n${snapshot['peer'] ?? ''}',
            style: const TextStyle(fontSize: 12, color: WispColors.muted),
          ),
        ],
      ),
    );
  }

  Future<void> machineSettings(Machine machine) async {
    if (snapshot['desktop'] == true && machine.data['local'] == true) {
      await deviceSettings();
      return;
    }
    await showWispSheet<void>(
      context,
      title: machine.name,
      child: MachineSettings(
        machine: machine,
        backend: backend,
        desktop: snapshot['desktop'] == true,
        onChanged: () {
          unawaited(poll());
          unawaited(refresh());
        },
      ),
    );
  }

  Future<void> deviceSettings({
    String section = 'My services',
    bool servicesOnly = false,
  }) async {
    if (snapshot['desktop'] != true) {
      await settings();
      return;
    }
    await showWispSheet<void>(
      context,
      title: servicesOnly ? 'Local services' : 'Settings',
      child: DesktopSettings(
        backend: backend,
        initialSection: section,
        servicesOnly: servicesOnly,
        onAccess: () => settings(),
        onChanged: () {
          unawaited(poll());
          unawaited(refresh());
        },
      ),
    );
  }

  Future<void> settings({
    Machine? machine,
    Map<String, dynamic>? service,
  }) async {
    await showWispSheet<void>(
      context,
      title: service?['display_name'] as String? ?? 'Settings',
      child: SettingsForm(
        backend: backend,
        deviceName: snapshot['name'] as String? ?? '',
        host: machine?.id,
        service: service?['id'] as String?,
        onSaved: () {
          Navigator.pop(context);
          notice('Settings saved');
          unawaited(poll());
        },
      ),
    );
  }

  Future<void> reorder(
    String group,
    List<String> ids,
    String moved,
    String target,
  ) async {
    final next = [...ids];
    final destination = next.indexOf(target);
    if (!next.remove(moved) || destination < 0) return;
    next.insert(destination, moved);
    setState(() => order[group] = next);
    try {
      await backend.invoke('saveOrder', {'group': group, 'ids': next});
      await poll();
    } catch (error) {
      notice(message(error));
      await poll();
    }
  }

  Color stateColor(Machine m) => m.online
      ? WispColors.green
      : m.state == 'approval_pending'
      ? WispColors.amber
      : m.state == 'host_denied'
      ? WispColors.error
      : WispColors.muted;
  String shortId(String id) => id.length > 8 ? id.substring(0, 8) : id;
  String telemetry(Machine machine) {
    final peer = (snapshot['telemetry'] as Map?)?['peers']?[machine.id] as Map?;
    if (peer == null) return '';
    final path = peer['path'] == 'direct'
        ? 'DIRECT'
        : peer['path'] == 'relay'
        ? 'RELAY'
        : '';
    final rtt = peer['rtt_ms'] as num? ?? 0;
    return [
      if (path.isNotEmpty) path,
      if (rtt > 0) '${rtt.round()}ms',
    ].join(' · ');
  }

  Widget sectionHeading(String title, List<Widget> actions) => Row(
    children: [
      Flexible(
        child: Text(
          title,
          style: const TextStyle(
            fontSize: 16,
            color: WispColors.muted,
            letterSpacing: 1.4,
            fontWeight: FontWeight.bold,
          ),
        ),
      ),
      const SizedBox(width: 12),
      ...actions,
    ],
  );

  Widget directoryCard(Machine machine, {bool local = false}) => MachineCard(
    machine: machine,
    showHeader: !local,
    order: order,
    telemetry: local ? '' : telemetry(machine),
    stateColor: stateColor(machine),
    busy: busy,
    onManage: () => machineSettings(machine),
    onOpen: (service) => open(machine, service),
    onSettings: (service) => settings(machine: machine, service: service),
    onReorder: reorder,
  );

  Widget directory() {
    final local = machines.where((m) => m.data['local'] == true).toList();
    final remote = machines.where((m) => m.data['local'] != true).toList();
    final buttonStyle = OutlinedButton.styleFrom(
      minimumSize: const Size(0, 42),
      shape: const StadiumBorder(),
    );
    return RefreshIndicator(
      key: const ValueKey('directory'),
      onRefresh: refresh,
      child: ListView(
        padding: const EdgeInsets.only(bottom: 24),
        physics: const AlwaysScrollableScrollPhysics(),
        children: [
          if (local.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(bottom: 24),
              child: Column(
                key: const ValueKey('local-sector'),
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  sectionHeading('LOCAL SERVICES', [
                    OutlinedButton(
                      onPressed: busy
                          ? null
                          : () => deviceSettings(servicesOnly: true),
                      style: buttonStyle,
                      child: const Text('Manage'),
                    ),
                  ]),
                  const SizedBox(height: 12),
                  for (final machine in local)
                    directoryCard(machine, local: true),
                ],
              ),
            ),
          Column(
            key: const ValueKey('remote-sector'),
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              sectionHeading('REMOTE SECTOR', [
                OutlinedButton(
                  onPressed: busy ? null : manage,
                  style: buttonStyle,
                  child: Text(
                    snapshot['desktop'] == true &&
                            (snapshot['pendingRequests'] as int? ?? 0) > 0
                        ? 'Manage · ${snapshot['pendingRequests']} requests'
                        : 'Manage',
                  ),
                ),
                IconButton(
                  tooltip: 'Refresh remote services',
                  onPressed: refreshing ? null : refresh,
                  icon: refreshing
                      ? const SizedBox(
                          width: 18,
                          height: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Icon(Icons.refresh),
                ),
              ]),
              const SizedBox(height: 12),
              if (remote.isEmpty) const Text('Pair a machine to get started'),
              for (final machine in remote)
                Padding(
                  padding: const EdgeInsets.only(bottom: 14),
                  child: ReorderTile(
                    group: 'machines',
                    id: machine.id,
                    onDrop: (moved) => reorder(
                      'machines',
                      remote.map((m) => m.id).toList(),
                      moved,
                      machine.id,
                    ),
                    child: directoryCard(machine),
                  ),
                ),
            ],
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final list = machines;
    return Scaffold(
      body: WispSpace(
        online: online,
        warp: warp,
        child: SafeArea(
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 1200),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 20),
                child: Column(
                  children: [
                    if (online)
                      WispHeader(
                        label:
                            '${snapshot['name'] ?? 'Wispnet'}${snapshot['peer'] == null ? '' : ' · ${shortId(snapshot['peer'] as String)}'}',
                        online: online,
                        onSettings: snapshot.isEmpty
                            ? null
                            : () => deviceSettings(),
                        onToggle: busy ? null : toggle,
                      ),
                    Expanded(
                      child: initializationError != null
                          ? Center(
                              child: Column(
                                mainAxisSize: MainAxisSize.min,
                                children: [
                                  Text(
                                    initializationError!,
                                    textAlign: TextAlign.center,
                                  ),
                                  const SizedBox(height: 16),
                                  FilledButton(
                                    onPressed: initialize,
                                    child: const Text('Retry'),
                                  ),
                                ],
                              ),
                            )
                          : snapshot.isEmpty
                          ? const Center(child: CircularProgressIndicator())
                          : AnimatedSwitcher(
                              duration: MediaQuery.disableAnimationsOf(context)
                                  ? Duration.zero
                                  : const Duration(milliseconds: 220),
                              child: online
                                  ? directory()
                                  : Center(
                                      key: const ValueKey('standby'),
                                      child: Column(
                                        mainAxisSize: MainAxisSize.min,
                                        children: [
                                          SpaceToggle(
                                            first: list.isEmpty,
                                            busy: busy,
                                            beacons: list
                                                .map(stateColor)
                                                .toList(),
                                            onPressed: busy
                                                ? null
                                                : list.isEmpty
                                                ? pair
                                                : toggle,
                                          ),
                                          const SizedBox(height: 8),
                                          Text(
                                            list.isEmpty
                                                ? 'establish your wispnet'
                                                : 'hyperjump to wispnet',
                                            textAlign: TextAlign.center,
                                            style: const TextStyle(
                                              fontSize: 21,
                                              letterSpacing: 1,
                                              shadows: [
                                                Shadow(
                                                  color: Color(0x883868c8),
                                                  blurRadius: 10,
                                                  offset: Offset(0, 2),
                                                ),
                                              ],
                                            ),
                                          ),
                                        ],
                                      ),
                                    ),
                            ),
                    ),
                    if (online)
                      Align(
                        alignment: Alignment.centerLeft,
                        child: Padding(
                          padding: const EdgeInsets.only(top: 6, bottom: 2),
                          child: Text(
                            'Relay: ${snapshot['relay'] ?? 'connecting…'}',
                            style: const TextStyle(
                              fontSize: 12,
                              color: WispColors.muted,
                            ),
                          ),
                        ),
                      ),
                    if (online)
                      Align(
                        alignment: Alignment.centerLeft,
                        child: Padding(
                          padding: const EdgeInsets.only(bottom: 6),
                          child: Text(
                            'v${snapshot['version'] ?? ''}',
                            style: const TextStyle(
                              fontSize: 12,
                              color: WispColors.muted,
                            ),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
