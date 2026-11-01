import 'package:flutter/material.dart';

import 'theme.dart';

/// Paint feedback above opaque tiles and custom-painted controls.
class WispInkWell extends StatefulWidget {
  const WispInkWell({
    super.key,
    required this.onTap,
    required this.child,
    this.borderRadius,
    this.customBorder,
  });
  final VoidCallback? onTap;
  final Widget child;
  final BorderRadius? borderRadius;
  final ShapeBorder? customBorder;
  @override
  State<WispInkWell> createState() => _WispInkWellState();
}

class _WispInkWellState extends State<WispInkWell> {
  bool hovered = false, focused = false, pressed = false;
  @override
  Widget build(BuildContext context) {
    final active = widget.onTap != null;
    return InkWell(
      onTap: widget.onTap,
      borderRadius: widget.borderRadius,
      customBorder: widget.customBorder,
      onHover: (value) => setState(() => hovered = value),
      onFocusChange: (value) => setState(() => focused = value),
      onHighlightChanged: (value) => setState(() => pressed = value),
      overlayColor: const WidgetStatePropertyAll(Colors.transparent),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 120),
        foregroundDecoration: ShapeDecoration(
          color: active && (hovered || focused || pressed)
              ? WispColors.cyan.withValues(alpha: pressed ? .24 : .16)
              : Colors.transparent,
          shape:
              widget.customBorder ??
              RoundedRectangleBorder(
                borderRadius: widget.borderRadius ?? BorderRadius.circular(8),
              ),
        ),
        child: widget.child,
      ),
    );
  }
}

/// Browser-style link with shared hover, focus, and accessibility behavior.
class WispLink extends StatelessWidget {
  const WispLink({
    super.key,
    required this.text,
    required this.tooltip,
    this.onTap,
  });
  final String text, tooltip;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    link: true,
    child: Tooltip(
      message: tooltip,
      child: WispInkWell(
        onTap: onTap,
        child: Text(
          text,
          style: const TextStyle(
            color: WispColors.cyan,
            decoration: TextDecoration.underline,
            decorationColor: WispColors.cyan,
          ),
        ),
      ),
    ),
  );
}
