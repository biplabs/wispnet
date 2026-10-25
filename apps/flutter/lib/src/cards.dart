import 'package:flutter/material.dart';

import 'backend.dart';
import 'theme.dart';

class MachineCard extends StatelessWidget {
  const MachineCard({
    super.key,
    required this.machine,
    required this.order,
    required this.telemetry,
    required this.stateColor,
    required this.busy,
    required this.onManage,
    required this.onOpen,
    required this.onLinks,
    this.onSettings,
    required this.onReorder,
    this.showHeader = true,
  });
  final Machine machine;
  final bool showHeader;
  final Map<String, dynamic> order;
  final String telemetry;
  final Color stateColor;
  final bool busy;
  final VoidCallback onManage;
  final ValueChanged<Map<String, dynamic>> onOpen, onLinks;
  final ValueChanged<Map<String, dynamic>>? onSettings;
  final void Function(
    String group,
    List<String> ids,
    String moved,
    String target,
  )
  onReorder;
  @override
  Widget build(BuildContext context) {
    final services = {for (final s in machine.services) s['id'] as String: s};
    final ids = orderedIds(
      services.keys,
      order['services/${machine.id}'] as List?,
    );
    final tel = telemetry;
    final badge = machine.online
        ? ['Online', if (tel.isNotEmpty) tel].join(' · ')
        : machine.state == 'approval_pending'
        ? 'AWAITING APPROVAL'
        : machine.state == 'host_denied'
        ? 'ACCESS DENIED'
        : 'Offline';
    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (showHeader)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: LayoutBuilder(
              builder: (context, constraints) {
                final title = WispInkWell(
                  onTap: busy ? null : onManage,
                  borderRadius: BorderRadius.circular(6),
                  child: Text(
                    '${machine.name} · ${machine.id.substring(0, 8)}',
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 14,
                      fontWeight: FontWeight.bold,
                      letterSpacing: 14 * .08,
                    ),
                  ),
                );
                final status = Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    WispStatusDot(
                      color: machine.online
                          ? WispColors.cyan
                          : machine.state == 'approval_pending'
                          ? const Color(0xffe9b65c)
                          : machine.state == 'host_denied'
                          ? WispColors.error
                          : const Color(0xff7a8da8),
                    ),
                    const SizedBox(width: 6),
                    Flexible(
                      child: Text(
                        badge,
                        style: TextStyle(
                          fontSize: 11,
                          color: machine.online
                              ? const Color(0xffedf4ff)
                              : stateColor,
                          letterSpacing: 11 * .06,
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                    ),
                  ],
                );
                return constraints.maxWidth < 320
                    ? Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [title, const SizedBox(height: 6), status],
                      )
                    : Row(
                        children: [
                          Expanded(
                            child: Align(
                              alignment: Alignment.centerLeft,
                              child: title,
                            ),
                          ),
                          const SizedBox(width: 12),
                          status,
                        ],
                      );
              },
            ),
          ),
        if (services.isEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 2, bottom: 4),
            child: Text(
              !showHeader
                  ? 'No local services shared.'
                  : machine.data['catalog'] == null
                  ? machine.state == 'approval_pending'
                        ? 'Awaiting approval'
                        : 'Unavailable'
                  : 'No shared services',
              style: const TextStyle(color: WispColors.muted),
            ),
          ),
        LayoutBuilder(
          builder: (context, constraints) {
            final columns = constraints.maxWidth >= 520 ? 2 : 1;
            final width = (constraints.maxWidth - (columns - 1) * 10) / columns;
            return Wrap(
              spacing: 10,
              runSpacing: 10,
              children: [
                for (final id in ids)
                  SizedBox(
                    width: width,
                    child: ReorderTile(
                      group: 'services/${machine.id}',
                      id: id,
                      onDrop: (moved) =>
                          onReorder('services/${machine.id}', ids, moved, id),
                      child: serviceCard(context, machine, services[id]!),
                    ),
                  ),
              ],
            );
          },
        ),
        if (services.isNotEmpty) const SizedBox(height: 4),
      ],
    );
    return showHeader
        ? WispPanel(
            color: const Color(0xd80b1528),
            padding: const EdgeInsets.fromLTRB(16, 6, 16, 6),
            child: content,
          )
        : content;
  }

  Widget serviceCard(
    BuildContext context,
    Machine machine,
    Map<String, dynamic> service,
  ) {
    final allowed = machine.canOpen(service);
    final publication = service['public_access'] as Map?;
    final publicEnabled = publication?['enabled'] == true;
    final publicCached = machine.data['local'] != true && !machine.online;
    final publicOnline = !publicCached && publication?['state'] == 'online';

    final subtitle = machine.online
        ? (service['kind'] as String).toUpperCase()
        : '${machine.name} · ${machine.state == 'unreachable'
              ? 'Offline · cached'
              : machine.state == 'approval_pending'
              ? 'Pending'
              : 'Unavailable'}';
    return Container(
      constraints: const BoxConstraints(minHeight: 64),
      decoration: BoxDecoration(
        color: WispColors.tile,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(
          color: allowed ? const Color(0xff32518a) : const Color(0xff1d2842),
        ),
      ),
      child: WispInkWell(
        onTap: !busy && allowed ? () => onOpen(service) : null,
        borderRadius: BorderRadius.circular(16),
        child: Builder(
          builder: (context) {
            final content = Expanded(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(12, 8, 0, 8),
                child: Opacity(
                  opacity: allowed ? 1 : .45,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Icon(
                            service['kind'] == 'shell'
                                ? Icons.terminal
                                : Icons.north_east,
                            size: 16,
                            color: WispColors.cyan,
                          ),
                          const SizedBox(width: 8),
                          Expanded(
                            child: Text(
                              service['id'] as String,
                              maxLines: 2,
                              overflow: TextOverflow.ellipsis,
                              style: const TextStyle(
                                fontSize: 17,
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 2),
                      Wrap(
                        spacing: 8,
                        runSpacing: 4,
                        crossAxisAlignment: WrapCrossAlignment.center,
                        children: [
                          Text(
                            subtitle,
                            style: const TextStyle(
                              fontSize: 13,
                              color: WispColors.muted,
                            ),
                          ),
                          if (publicEnabled)
                            Tooltip(
                              message: publicCached
                                  ? 'Last known public link; refresh this machine to verify its status'
                                  : publicOnline
                                  ? 'Public access enabled and online'
                                  : 'Public access enabled but offline',
                              child: Container(
                                padding: const EdgeInsets.symmetric(
                                  horizontal: 6,
                                  vertical: 2,
                                ),
                                decoration: BoxDecoration(
                                  color:
                                      (publicCached
                                              ? WispColors.muted
                                              : publicOnline
                                              ? WispColors.green
                                              : WispColors.error)
                                          .withValues(alpha: .12),
                                  borderRadius: BorderRadius.circular(8),
                                ),
                                child: Text(
                                  publicCached
                                      ? 'Public · Cached'
                                      : publicOnline
                                      ? 'Public · Online'
                                      : 'Public · Offline',
                                  style: TextStyle(
                                    fontSize: 11,
                                    fontWeight: FontWeight.bold,
                                    color: publicCached
                                        ? WispColors.muted
                                        : publicOnline
                                        ? WispColors.green
                                        : WispColors.error,
                                  ),
                                ),
                              ),
                            ),
                        ],
                      ),
                    ],
                  ),
                ),
              ),
            );
            final controls = <Widget>[
              if (service['kind'] == 'http')
                Tooltip(
                  message: 'Show links for ${service['id']}',
                  child: IconButton(
                    onPressed: busy ? null : () => onLinks(service),
                    icon: const Icon(
                      Icons.link,
                      size: 20,
                      color: WispColors.cyan,
                    ),
                  ),
                ),
              if (onSettings != null)
                IconButton(
                  tooltip: 'Settings for ${service['id']} on ${machine.name}',
                  onPressed: busy ? null : () => onSettings!(service),
                  icon: const Icon(
                    Icons.settings,
                    size: 24,
                    color: WispColors.muted,
                  ),
                ),
            ];
            return Row(
              children: [
                content,
                Wrap(
                  spacing: 8,
                  runSpacing: 4,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: controls,
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}

class ReorderTile extends StatelessWidget {
  const ReorderTile({
    super.key,
    required this.group,
    required this.id,
    required this.onDrop,
    required this.child,
  });
  final String group, id;
  final ValueChanged<String> onDrop;
  final Widget child;
  @override
  Widget build(BuildContext context) => DragTarget<String>(
    onWillAcceptWithDetails: (d) =>
        d.data.startsWith('$group\n') && d.data != '$group\n$id',
    onAcceptWithDetails: (d) => onDrop(d.data.substring(group.length + 1)),
    builder: (context, candidates, rejected) => LongPressDraggable<String>(
      data: '$group\n$id',
      maxSimultaneousDrags: 1,
      feedback: Material(
        color: Colors.transparent,
        child: WispPanel(child: Text(id)),
      ),
      childWhenDragging: Opacity(opacity: .4, child: child),
      child: AnimatedOpacity(
        duration: const Duration(milliseconds: 120),
        opacity: candidates.isEmpty ? 1 : .65,
        child: child,
      ),
    ),
  );
}
