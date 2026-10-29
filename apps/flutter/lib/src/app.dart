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
import 'command_center/command_center.dart';
import 'terminal_sheet.dart';

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
      pollQueued = false,
      refreshing = false,
      busy = false,
      foreground = true,
      pairingSheet = false,
      entering = false;
  int ticks = 0, launchGeneration = 0;
  String lastNetwork = '';
  Timer? timer;
  late final AnimationController warp = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 900),
  );
  late final DesktopShellManager desktopShells =
      DesktopShellManager(widget.backend)..addListener(() {
        if (mounted) setState(() {});
      });
  bool get online => snapshot['enabled'] == true;
  List<Map<String, dynamic>> get minimizedShells {
    if (!online) return const [];
    if (snapshot['desktop'] == true) {
      return desktopShells.minimizedSnapshots;
    }
    final shells = snapshot['shells'];
    if (shells is List) {
      return shells
          .whereType<Map>()
          .map((e) => e.cast<String, dynamic>())
          .where((e) => e['minimized'] == true)
          .toList();
    }
    final shell = snapshot['shell'];
    if (shell is Map) {
      final data = shell.cast<String, dynamic>();
      if (data['minimized'] == true) return [data];
    }
    return const [];
  }

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
    desktopShells.closeAll(terminate: false, notify: false);
    desktopShells.dispose();
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
    if (polling) {
      pollQueued = true;
      return;
    }
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
      if (next['desktop'] == true && next['hosts'] is Map) {
        final validHosts = (next['hosts'] as Map).keys
            .whereType<String>()
            .toSet();
        desktopShells.pruneRemovedHosts(validHosts);
      }
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
      final link = (await backend.invoke('takeLink'))['link'] as String?;
      if (link != null) {
        try {
          final uri = Uri.parse(link);
          if (uri.scheme != 'wispnet' ||
              !RegExp(r'^[0-9a-f]{64}$').hasMatch(uri.host) ||
              uri.userInfo.isNotEmpty ||
              uri.hasPort ||
              uri.hasQuery ||
              uri.hasFragment ||
              !RegExp(r'^/[a-z0-9][a-z0-9._-]{0,63}$').hasMatch(uri.path)) {
            throw const FormatException('Invalid Wispnet service link');
          }
          await backend.invoke('setAccess', {'enabled': true});
          final resolved = await backend.invoke('resolveLink', {'link': link});
          await backend.invoke('launch', {'url': resolved['url']});
        } catch (error) {
          notice(message(error));
        }
      }
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
      if (pollQueued && mounted) {
        pollQueued = false;
        unawaited(poll());
      }
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
    final enable = !online;
    setState(() {
      busy = true;
      entering = enable && !MediaQuery.disableAnimationsOf(context);
    });
    final jump = entering ? warp.forward(from: 0) : null;
    try {
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
        if (mounted && jump != null) await jump;
        unawaited(refresh());
      } else {
        desktopShells.closeAll(terminate: true);
        await Future<void>.delayed(const Duration(milliseconds: 100));
        await poll();
      }
    } catch (error) {
      notice(message(error));
    } finally {
      if (mounted) {
        if (jump != null) warp.reset();
        setState(() {
          busy = false;
          entering = false;
        });
      }
    }
  }

  Future<void> showDesktopShell({
    required String host,
    required String hostName,
    required String service,
    required String incarnation,
    String session = 'main',
  }) async {
    final workspace = desktopShells.openHost(
      host: host,
      hostName: hostName,
      service: service,
      incarnation: incarnation,
      session: session,
    );
    if (!mounted) return;
    await showWispSheet<void>(
      context,
      title: hostName,
      showHeader: false,
      scrollable: false,
      maxWidth: 1120,
      child: DesktopTerminalSheet(
        manager: desktopShells,
        workspace: workspace,
        onOpenExternal: (pane) async {
          try {
            await backend.invoke('openShell', {
              'host': pane.host,
              'host_name': pane.hostName,
              'service': pane.service,
              'incarnation': pane.incarnation,
              'session': pane.session,
            });
          } catch (error) {
            notice('Cannot open terminal: ${message(error)}');
          }
        },
      ),
    );
    if (workspace.tabs.isNotEmpty) {
      workspace.minimized = true;
    }
    if (mounted) setState(() {});
  }

  Future<void> open(Machine machine, Map<String, dynamic> service) async {
    if (busy || !online) return;
    if (service['kind'] == 'shell' && snapshot['desktop'] == true) {
      await showDesktopShell(
        host: machine.id,
        hostName: machine.name,
        service: service['id'] as String? ?? 'shell',
        incarnation: service['incarnation'] as String,
      );
      return;
    }
    final generation = ++launchGeneration;
    setState(() => busy = true);
    if (service['kind'] != 'shell') {
      notice('Opening ${service['id']}…');
    }
    try {
      if (service['kind'] == 'shell') {
        await backend.invoke('openShell', {
          'host': machine.id,
          'host_name': machine.name,
          'service': service['id'],
          'incarnation': service['incarnation'],
          'session': 'main',
        });
        if (mounted) ScaffoldMessenger.of(context).hideCurrentSnackBar();
        return;
      }
      final publication = service['public_access'] as Map?;
      final publicUrl = publication?['url'] as String?;
      final result =
          service['kind'] == 'http' &&
              publication?['enabled'] == true &&
              publicUrl != null &&
              publicUrl.isNotEmpty
          ? {'url': publicUrl}
          : await backend.invoke('open', {
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
      await backend.invoke('launch', {'url': result['url']});
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

  Future<void> expandShell(Map<String, dynamic> shell) async {
    if (busy || !online) return;
    if (snapshot['desktop'] == true) {
      await showDesktopShell(
        host: shell['host'] as String,
        hostName: shell['host_name'] as String? ?? 'Shell',
        service: shell['service'] as String? ?? 'shell',
        incarnation: shell['incarnation'] as String,
        session: shell['session'] as String? ?? 'main',
      );
      return;
    }
    setState(() => busy = true);
    try {
      await backend.invoke('openShell', {
        'host': shell['host'],
        'host_name': shell['host_name'],
        'service': shell['service'] ?? 'shell',
        'incarnation': shell['incarnation'],
        'session': shell['session'] ?? 'main',
      });
    } catch (error) {
      notice('Cannot open service: ${message(error)}');
    } finally {
      if (mounted) {
        setState(() => busy = false);
        await poll();
      }
    }
  }

  Future<void> confirmCloseShell(Map<String, dynamic> shell) async {
    if (busy) return;
    final host = shell['host'] as String? ?? '';
    final hostName = shell['host_name'] as String? ?? 'this machine';
    final confirmed = await showWispSheet<bool>(
      context,
      title: 'Close terminal?',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Close all terminal sessions on $hostName? Running commands in those tabs will be terminated.',
          ),
          const SizedBox(height: 16),
          FilledButton(
            key: const ValueKey('confirm-close-shell'),
            onPressed: () => Navigator.pop(context, true),
            style:
                OutlinedButton.styleFrom(
                  foregroundColor: WispColors.error,
                  side: const BorderSide(color: WispColors.error),
                ).copyWith(
                  backgroundColor: wispButtonBackground(
                    const Color(0xff401e2a),
                  ),
                ),
            child: const Text('Close sessions'),
          ),
          const SizedBox(height: 8),
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
        ],
      ),
    );
    if (confirmed == true && mounted) {
      await closeShell(host);
    }
  }

  Future<void> closeShell([String? host]) async {
    if (busy) return;
    if (snapshot['desktop'] == true) {
      if (host != null && host.isNotEmpty) {
        desktopShells.closeHost(host, terminate: true);
      } else {
        desktopShells.closeAll(terminate: true);
      }
      return;
    }
    setState(() {
      final next = {...snapshot};
      if (host != null && host.isNotEmpty) {
        final shells = (next['shells'] as List?)
            ?.whereType<Map>()
            .where((e) => e['host'] != host)
            .toList();
        if (shells != null) next['shells'] = shells;
        if ((next['shell'] as Map?)?['host'] == host) {
          next.remove('shell');
        }
      } else {
        next.remove('shell');
        next.remove('shells');
      }
      snapshot = next;
    });
    try {
      await backend.invoke(
        'closeShell',
        host != null && host.isNotEmpty ? {'host': host} : const {},
      );
    } catch (error) {
      notice(message(error));
    } finally {
      if (mounted) await poll();
    }
  }

  Future<void> serviceLinks(
    Machine machine,
    Map<String, dynamic> service,
  ) async {
    await showWispSheet<void>(
      context,
      title: 'Links for ${service['id']}',
      child: ServiceLinks(
        backend: backend,
        machine: machine,
        service: service,
        android:
            snapshot['desktop'] != true &&
            Theme.of(context).platform == TargetPlatform.android,
        canResolve: online && machine.canOpen(service),
      ),
    );
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
            'Host ID\n${snapshot['peer'] ?? ''}',
            style: const TextStyle(fontSize: 12, color: WispColors.muted),
          ),
        ],
      ),
    );
  }

  Future<void> machineSettings(Machine machine) async {
    if (snapshot['desktop'] == true && machine.data['local'] == true) {
      await deviceSettings(servicesOnly: true);
      return;
    }
    await showWispSheet<void>(
      context,
      title: machine.name,
      child: MachineSettings(
        machine: machine,
        backend: backend,
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

  Future<void> localServiceSettings(Map<String, dynamic> service) async {
    await showWispSheet<void>(
      context,
      title: service['id'] as String,
      child: DesktopSettings(
        backend: backend,
        servicesOnly: true,
        serviceId: service['id'] as String,
        onAccess: settings,
        onChanged: () {
          unawaited(poll());
          unawaited(refresh());
        },
      ),
    );
  }

  Future<void> settings() async {
    await showWispSheet<void>(
      context,
      title: 'Settings',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SettingsForm(
            backend: backend,
            deviceName: snapshot['name'] as String? ?? '',
            onSaved: () {
              Navigator.pop(context);
              notice('Settings saved');
              unawaited(poll());
            },
          ),
          if (snapshot['desktop'] == true && commandCenterEnabled) ...[
            const SizedBox(height: 16),
            const Divider(),
            Align(
              alignment: Alignment.centerLeft,
              child: CommandCenterAccountButton(
                backend: backend,
                onChanged: () => unawaited(poll()),
              ),
            ),
            const SizedBox(height: 4),
            const Text(
              'An account is only needed to expose services to the public internet through the Wispnet gateway.',
              style: TextStyle(color: WispColors.muted),
            ),
          ],
        ],
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
        ? 'Direct'
        : peer['path'] == 'relay'
        ? 'Relay'
        : '';
    final rtt = peer['rtt_ms'] as num? ?? 0;
    return [
      if (path.isNotEmpty) path,
      if (rtt > 0) '${rtt.round()}ms',
    ].join(' · ');
  }

  Widget sectionHeading(
    String title,
    List<Widget> actions, {
    Widget? trailing,
  }) {
    final heading = Row(
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
    return trailing == null
        ? heading
        : Row(
            children: [
              Expanded(child: heading),
              trailing,
            ],
          );
  }

  Widget directoryCard(Machine machine, {bool local = false}) => MachineCard(
    machine: machine,
    showHeader: !local,
    order: order,
    telemetry: local ? '' : telemetry(machine),
    stateColor: stateColor(machine),
    busy: busy,
    onManage: () => machineSettings(machine),
    onOpen: (service) => open(machine, service),
    onLinks: (service) => serviceLinks(machine, service),
    onSettings: local && snapshot['desktop'] == true
        ? localServiceSettings
        : null,
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
              sectionHeading(
                'REMOTE SECTOR',
                [
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
                ],
                trailing: IconButton(
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
              ),
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

  Widget shellDock() {
    final shells = minimizedShells;
    final animate = !MediaQuery.disableAnimationsOf(context);
    return AnimatedSwitcher(
      duration: animate ? const Duration(milliseconds: 220) : Duration.zero,
      switchInCurve: Curves.easeOutCubic,
      switchOutCurve: Curves.easeInCubic,
      transitionBuilder: (child, animation) => FadeTransition(
        opacity: animation,
        child: SlideTransition(
          position: Tween<Offset>(
            begin: const Offset(0, .35),
            end: Offset.zero,
          ).animate(animation),
          child: child,
        ),
      ),
      child: shells.isEmpty
          ? const SizedBox.shrink()
          : Padding(
              key: ValueKey(
                'shell-docks-${shells.map((s) => s['host']).join(',')}',
              ),
              padding: const EdgeInsets.only(top: 4, bottom: 8),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (var i = 0; i < shells.length; i++) ...[
                    if (i > 0) const SizedBox(height: 6),
                    _buildDockBar(shells[i], primary: i == 0),
                  ],
                ],
              ),
            ),
    );
  }

  Widget _buildDockBar(Map<String, dynamic> shell, {required bool primary}) {
    final host = shell['host'] as String? ?? '';
    return WispPanel(
      key: ValueKey(primary ? 'shell-dock' : 'shell-dock-$host'),
      color: WispColors.sheet,
      radius: 16,
      padding: const EdgeInsets.fromLTRB(6, 4, 6, 4),
      child: Row(
        children: [
          Expanded(
            child: Semantics(
              button: true,
              label:
                  'Expand shell ${shell['title'] ?? shell['host_name'] ?? 'Shell'}',
              child: WispInkWell(
                key: ValueKey(
                  primary ? 'shell-dock-expand' : 'shell-dock-expand-$host',
                ),
                onTap: busy ? null : () => expandShell(shell),
                borderRadius: BorderRadius.circular(12),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 8,
                    vertical: 8,
                  ),
                  child: Row(
                    children: [
                      WispStatusDot(
                        color: shell['state'] == 'attached'
                            ? WispColors.green
                            : shell['state'] == 'connecting'
                            ? WispColors.amber
                            : WispColors.error,
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Text(
                          (shell['title'] as String?)?.isNotEmpty == true
                              ? shell['title'] as String
                              : '${shell['host_name'] ?? 'Shell'} · ${shell['session'] ?? 'main'}',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(
                            fontSize: 14,
                            fontWeight: FontWeight.bold,
                          ),
                        ),
                      ),
                      const SizedBox(width: 8),
                      const Icon(
                        Icons.keyboard_arrow_up,
                        size: 20,
                        color: WispColors.cyan,
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
          const SizedBox(width: 4),
          IconButton(
            key: ValueKey(
              primary ? 'shell-dock-close' : 'shell-dock-close-$host',
            ),
            tooltip: 'Close shell',
            onPressed: busy ? null : () => confirmCloseShell(shell),
            icon: const Icon(Icons.close, size: 18, color: WispColors.muted),
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
        child: IgnorePointer(
          ignoring: entering,
          child: FadeTransition(
            key: const ValueKey('entry-reveal'),
            opacity: entering
                ? online
                      ? warp.drive(CurveTween(curve: const Interval(7 / 9, 1)))
                      : const AlwaysStoppedAnimation(0.0)
                : const AlwaysStoppedAnimation(1.0),
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
                                : () => settings(),
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
                                  duration:
                                      MediaQuery.disableAnimationsOf(context)
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
                        shellDock(),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
