import 'package:flutter/material.dart';

import 'feedback.dart';
import 'theme.dart';

/// Identity stays at the start; independent controls always follow the content.
/// At narrow widths controls move below the label instead of squeezing it out.
class WispListItem extends StatelessWidget {
  const WispListItem({
    super.key,
    required this.title,
    this.subtitle,
    this.leading,
    this.actions = const [],
    this.onTap,
    this.enabled = true,
    this.contentPadding = const EdgeInsets.symmetric(
      horizontal: 12,
      vertical: 8,
    ),
    this.contentOpacity = 1,
    this.titleStyle,
    this.minHeight = 64,
  });

  final Widget title;
  final Widget? subtitle, leading;
  final List<Widget> actions;
  final VoidCallback? onTap;
  final bool enabled;
  final EdgeInsetsGeometry contentPadding;
  final double contentOpacity, minHeight;
  final TextStyle? titleStyle;

  @override
  Widget build(BuildContext context) => Material(
    type: MaterialType.transparency,
    child: WispInkWell(
      onTap: enabled ? onTap : null,
      borderRadius: BorderRadius.circular(16),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: minHeight),
        child: Padding(
          padding: contentPadding,
          child: LayoutBuilder(
            builder: (context, constraints) {
              final content = Opacity(
                opacity: contentOpacity,
                child: Row(
                  children: [
                    if (leading != null) ...[
                      leading!,
                      const SizedBox(width: 10),
                    ],
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          DefaultTextStyle.merge(
                            style:
                                titleStyle ??
                                const TextStyle(color: WispColors.text),
                            child: title,
                          ),
                          if (subtitle != null) ...[
                            const SizedBox(height: 4),
                            DefaultTextStyle.merge(
                              style: const TextStyle(
                                color: WispColors.muted,
                                fontSize: 13,
                              ),
                              child: subtitle!,
                            ),
                          ],
                        ],
                      ),
                    ),
                  ],
                ),
              );
              if (actions.isEmpty) return content;
              final controls = Wrap(
                spacing: 4,
                runSpacing: 4,
                alignment: WrapAlignment.end,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: actions,
              );
              final labelWidth =
                  140 * MediaQuery.textScalerOf(context).scale(14) / 14;
              final controlsWidth = actions.length * 56.0;
              if (constraints.maxWidth < labelWidth + controlsWidth + 12) {
                return Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    content,
                    const SizedBox(height: 4),
                    Align(
                      alignment: AlignmentDirectional.centerEnd,
                      child: controls,
                    ),
                  ],
                );
              }
              return Row(
                children: [
                  Expanded(child: content),
                  const SizedBox(width: 12),
                  controls,
                ],
              );
            },
          ),
        ),
      ),
    ),
  );
}
