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
    required this.onSettings,
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
  final ValueChanged<Map<String, dynamic>> onOpen, onSettings;
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
        ? ['ONLINE', if (tel.isNotEmpty) tel].join(' · ')
        : machine.state == 'approval_pending'
        ? 'AWAITING APPROVAL'
        : machine.state == 'host_denied'
        ? 'ACCESS DENIED'
        : 'OFFLINE';
    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (showHeader)
          WispInkWell(
            onTap: busy ? null : onManage,
            borderRadius: BorderRadius.circular(12),
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 6),
              child: LayoutBuilder(
                builder: (context, constraints) {
                  final title = Text(
                    '${machine.name.toUpperCase()} · ${machine.id.substring(0, 8)}',
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 14,
                      fontWeight: FontWeight.bold,
                      letterSpacing: 14 * .08,
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
                  return constraints.maxWidth < 460
                      ? Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [title, const SizedBox(height: 6), status],
                        )
                      : Row(
                          children: [
                            Expanded(child: title),
                            const SizedBox(width: 12),
                            status,
                          ],
                        );
                },
              ),
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
                      child: serviceCard(machine, services[id]!),
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

  Widget serviceCard(Machine machine, Map<String, dynamic> service) {
    final allowed = machine.canOpen(service);
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
        child: Row(
          children: [
            Expanded(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(12, 8, 0, 8),
                child: Opacity(
                  opacity: allowed ? 1 : .45,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          const Icon(
                            Icons.auto_awesome,
                            size: 16,
                            color: WispColors.cyan,
                          ),
                          const SizedBox(width: 8),
                          Expanded(
                            child: Text(
                              service['display_name'] as String,
                              maxLines: 2,
                              overflow: TextOverflow.ellipsis,
                              style: const TextStyle(
                                fontSize: 17,
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                          ),
                          const SizedBox(width: 8),
                          const Icon(
                            Icons.north_east,
                            size: 16,
                            color: WispColors.cyan,
                          ),
                        ],
                      ),
                      const SizedBox(height: 2),
                      Text(
                        subtitle,
                        style: const TextStyle(
                          fontSize: 13,
                          color: WispColors.muted,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            IconButton(
              tooltip:
                  'Settings for ${service['display_name']} on ${machine.name}',
              onPressed: busy ? null : () => onSettings(service),
              icon: const Icon(
                Icons.settings,
                size: 24,
                color: WispColors.muted,
              ),
            ),
          ],
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
