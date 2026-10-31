import 'dart:ui' as ui;

import 'package:flutter/material.dart';

abstract final class WispColors {
  static const background = Color(0xff070b14);
  static const sheet = Color(0xff0b1324);
  static const panel = Color(0xff101a30);
  static const tile = Color(0xff111e38);
  static const border = Color(0xff294270);
  static const borderActive = Color(0xff4b73c2);
  static const text = Color(0xffedf2ff);
  static const muted = Color(0xff94a6c4);
  static const cyan = Color(0xff65ceff);
  static const green = Color(0xff7be0b8);
  static const amber = Color(0xfff0b85c);
  static const error = Color(0xffff9aa8);
}

WidgetStateProperty<Color?> wispButtonBackground(Color base) =>
    WidgetStateProperty.resolveWith((states) {
      if (states.contains(WidgetState.disabled)) return null;
      if (states.contains(WidgetState.pressed)) {
        return Color.alphaBlend(const Color(0x4065ceff), base);
      }
      if (states.contains(WidgetState.hovered) ||
          states.contains(WidgetState.focused)) {
        return Color.alphaBlend(const Color(0x3065ceff), base);
      }
      return base;
    });

ThemeData wispTheme() => ThemeData(
  brightness: Brightness.dark,
  scaffoldBackgroundColor: WispColors.background,
  colorScheme: const ColorScheme.dark(
    primary: WispColors.cyan,
    surface: WispColors.sheet,
    error: WispColors.error,
  ),
  hoverColor: const Color(0x3065ceff),
  focusColor: const Color(0x3065ceff),
  iconButtonTheme: IconButtonThemeData(
    style: ButtonStyle(
      backgroundColor: WidgetStateProperty.resolveWith(
        (states) =>
            !states.contains(WidgetState.disabled) &&
                (states.contains(WidgetState.hovered) ||
                    states.contains(WidgetState.focused))
            ? const Color(0x3065ceff)
            : null,
      ),
    ),
  ),
  fontFamily: 'Roboto',
  textTheme: const TextTheme(
    bodyMedium: TextStyle(color: WispColors.text),
    bodySmall: TextStyle(color: WispColors.muted),
  ),
  textButtonTheme: TextButtonThemeData(
    style: TextButton.styleFrom(
      overlayColor: WispColors.cyan.withValues(alpha: .22),
    ).copyWith(backgroundColor: wispButtonBackground(Colors.transparent)),
  ),
  filledButtonTheme: FilledButtonThemeData(
    style: FilledButton.styleFrom(
      backgroundColor: const Color(0xff1f3666),
      foregroundColor: WispColors.text,
      overlayColor: WispColors.cyan.withValues(alpha: .22),
      minimumSize: const Size(48, 48),
      side: const BorderSide(color: Color(0xff4b73c2)),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
    ).copyWith(backgroundColor: wispButtonBackground(const Color(0xff1f3666))),
  ),
  outlinedButtonTheme: OutlinedButtonThemeData(
    style: OutlinedButton.styleFrom(
      foregroundColor: WispColors.text,
      overlayColor: WispColors.cyan.withValues(alpha: .22),
      backgroundColor: const Color(0xff142038),
      minimumSize: const Size(48, 48),
      side: const BorderSide(color: Color(0xff283b60)),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
    ).copyWith(backgroundColor: wispButtonBackground(const Color(0xff142038))),
  ),
  inputDecorationTheme: InputDecorationTheme(
    filled: true,
    fillColor: const Color(0xff0d1526),
    hintStyle: const TextStyle(color: Color(0xff6e82a3)),
    border: OutlineInputBorder(
      borderRadius: BorderRadius.circular(14),
      borderSide: const BorderSide(color: Color(0xff25385c)),
    ),
    enabledBorder: OutlineInputBorder(
      borderRadius: BorderRadius.circular(14),
      borderSide: const BorderSide(color: Color(0xff25385c)),
    ),
  ),
  snackBarTheme: SnackBarThemeData(
    backgroundColor: const Color(0xeb101d36),
    contentTextStyle: const TextStyle(color: Color(0xffb8d4ff)),
    behavior: SnackBarBehavior.floating,
    shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
  ),
);

class WispPanel extends StatelessWidget {
  const WispPanel({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.all(16),
    this.color = WispColors.panel,
    this.radius = 20,
  });
  final Widget child;
  final EdgeInsets padding;
  final Color color;
  final double radius;
  @override
  Widget build(BuildContext context) => Container(
    padding: padding,
    decoration: BoxDecoration(
      color: color,
      borderRadius: BorderRadius.circular(radius),
      border: Border.all(color: WispColors.border),
    ),
    child: child,
  );
}

class WispDockBar extends StatelessWidget {
  const WispDockBar({
    super.key,
    required this.title,
    required this.statusColor,
    required this.onExpand,
    required this.onClose,
    required this.expandLabel,
    required this.closeLabel,
    this.expandKey,
    this.closeKey,
  });
  final String title, expandLabel, closeLabel;
  final Color statusColor;
  final VoidCallback? onExpand, onClose;
  final Key? expandKey, closeKey;

  @override
  Widget build(BuildContext context) => WispPanel(
    color: WispColors.sheet,
    radius: 16,
    padding: const EdgeInsets.fromLTRB(6, 4, 6, 4),
    child: Row(
      children: [
        Expanded(
          child: Semantics(
            button: true,
            label: expandLabel,
            child: WispInkWell(
              key: expandKey,
              onTap: onExpand,
              borderRadius: BorderRadius.circular(12),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 8),
                child: Row(
                  children: [
                    WispStatusDot(color: statusColor),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Text(
                        title,
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
          key: closeKey,
          tooltip: closeLabel,
          onPressed: onClose,
          icon: const Icon(Icons.close, size: 18, color: WispColors.muted),
        ),
      ],
    ),
  );
}

class _WispBottomSheetRoute<T> extends ModalBottomSheetRoute<T> {
  _WispBottomSheetRoute({
    required super.builder,
    super.capturedThemes,
    super.barrierLabel,
    super.barrierOnTapHint,
    super.backgroundColor,
    super.constraints,
    super.modalBarrierColor,
    required super.isScrollControlled,
    super.useSafeArea,
  });

  @override
  Widget buildModalBarrier() {
    final barrier = super.buildModalBarrier();
    final routeAnimation = animation;
    if (routeAnimation == null || offstage) return barrier;
    return AnimatedBuilder(
      animation: routeAnimation,
      builder: (context, child) {
        final sigma = 8 * Curves.easeOut.transform(routeAnimation.value);
        return BackdropFilter(
          filter: ui.ImageFilter.blur(sigmaX: sigma, sigmaY: sigma),
          child: child,
        );
      },
      child: barrier,
    );
  }
}

Future<T?> showWispSheet<T>(
  BuildContext context, {
  required String title,
  required Widget child,
  bool showHeader = true,
  bool scrollable = true,
  double maxWidth = 640,
}) {
  final navigator = Navigator.of(context);
  final localizations = MaterialLocalizations.of(context);
  return navigator.push(
    _WispBottomSheetRoute<T>(
      isScrollControlled: true,
      useSafeArea: true,
      backgroundColor: Colors.transparent,
      modalBarrierColor: const Color(0x78040812),
      constraints: BoxConstraints(maxWidth: maxWidth),
      capturedThemes: InheritedTheme.capture(
        from: context,
        to: navigator.context,
      ),
      barrierLabel: localizations.scrimLabel,
      barrierOnTapHint: localizations.scrimOnTapHint(
        localizations.bottomSheetLabel,
      ),
      builder: (context) {
        final content = Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Center(
              child: Container(
                width: 40,
                height: 4,
                margin: EdgeInsets.only(bottom: showHeader ? 12 : 6),
                decoration: BoxDecoration(
                  color: WispColors.muted.withValues(alpha: .4),
                  borderRadius: BorderRadius.circular(2),
                ),
              ),
            ),
            if (showHeader) ...[
              Row(
                children: [
                  Expanded(
                    child: Text(
                      title,
                      style: const TextStyle(
                        fontSize: 20,
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                  ),
                  IconButton(
                    tooltip: 'Close',
                    onPressed: () => Navigator.pop(context),
                    icon: const Icon(Icons.close, size: 20),
                  ),
                ],
              ),
              const SizedBox(height: 12),
            ],
            child,
          ],
        );
        return Padding(
          padding: EdgeInsets.fromLTRB(
            12,
            20,
            12,
            12 + MediaQuery.viewInsetsOf(context).bottom,
          ),
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxHeight: MediaQuery.sizeOf(context).height * .86,
            ),
            child: WispPanel(
              color: WispColors.sheet,
              radius: 24,
              padding: showHeader
                  ? const EdgeInsets.fromLTRB(20, 12, 20, 24)
                  : const EdgeInsets.fromLTRB(12, 8, 12, 10),
              child: scrollable ? SingleChildScrollView(child: content) : content,
            ),
          ),
        );
      },
    ),
  );
}

/// Same three-layer shape as Android's StatusDot drawable.
class WispStatusDot extends StatelessWidget {
  const WispStatusDot({super.key, required this.color});
  final Color color;
  @override
  Widget build(BuildContext context) => SizedBox.square(
    dimension: 16,
    child: CustomPaint(painter: _StatusDotPainter(color)),
  );
}

class _StatusDotPainter extends CustomPainter {
  const _StatusDotPainter(this.color);
  final Color color;
  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()..isAntiAlias = true;
    final center = size.center(Offset.zero), unit = size.shortestSide / 16;
    paint.color = color.withAlpha(68);
    canvas.drawCircle(center, 7.5 * unit, paint);
    paint.color = color;
    canvas.drawCircle(center, 4.2 * unit, paint);
    paint.color = const Color(0xfff4fcff);
    canvas.drawCircle(center, 1.8 * unit, paint);
  }

  @override
  bool shouldRepaint(_StatusDotPainter old) => old.color != color;
}

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
