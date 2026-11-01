import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'list_item.dart';
import 'theme.dart';

/// Shared status-light control for service state and binary settings.
class WispToggle extends StatelessWidget {
  const WispToggle({super.key, required this.value, required this.onChanged});
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) => Theme(
    data: ThemeData(
      useMaterial3: false,
      colorScheme: Theme.of(context).colorScheme,
    ),
    child: Switch(
      value: value,
      activeThumbColor: const Color(0xff4596ff),
      inactiveThumbColor: const Color(0xffff5252),
      activeTrackColor: const Color(0xff29364a),
      inactiveTrackColor: const Color(0xff29364a),
      onChanged: onChanged,
    ),
  );
}

/// A settings row with identity on the leading side and controls at the end.
class WispToggleListTile extends StatelessWidget {
  const WispToggleListTile({
    super.key,
    required this.value,
    required this.onChanged,
    this.title,
    this.subtitle,
    this.secondary,
    this.contentPadding,
    this.actions = const [],
  });
  final bool value;
  final ValueChanged<bool?>? onChanged;
  final Widget? title, subtitle, secondary;
  final EdgeInsetsGeometry? contentPadding;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) => WispListItem(
    title: title ?? const SizedBox.shrink(),
    subtitle: subtitle,
    leading: secondary,
    enabled: onChanged != null,
    contentOpacity: onChanged == null ? .45 : 1,
    onTap: onChanged == null ? null : () => onChanged!(!value),
    contentPadding:
        contentPadding ??
        const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
    actions: [
      WispToggle(
        value: value,
        onChanged: onChanged == null ? null : (value) => onChanged!(value),
      ),
      ...actions,
    ],
  );
}

/// Settings actions always use the same icon and color.
class WispSettingsButton extends StatelessWidget {
  const WispSettingsButton({
    super.key,
    required this.tooltip,
    required this.onPressed,
  });
  final String tooltip;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) => IconButton(
    tooltip: tooltip,
    onPressed: onPressed,
    icon: const Icon(Icons.settings, size: 24, color: WispColors.muted),
  );
}

/// Copy feedback stays consistent wherever an address or pairing code appears.
class WispCopyButton extends StatelessWidget {
  const WispCopyButton({
    super.key,
    required this.text,
    required this.tooltip,
    this.message = 'Copied',
    this.onCopied,
  });
  final String text, tooltip, message;
  final VoidCallback? onCopied;

  @override
  Widget build(BuildContext context) => IconButton(
    tooltip: tooltip,
    onPressed: () async {
      await Clipboard.setData(ClipboardData(text: text));
      if (!context.mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(message)));
      onCopied?.call();
    },
    icon: const Icon(Icons.copy, color: WispColors.cyan),
  );
}
