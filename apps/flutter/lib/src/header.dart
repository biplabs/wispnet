import 'package:flutter/material.dart';

import 'theme.dart';

/// Matches MainActivity's deviceHeader and dockToggle geometry.
class WispHeader extends StatelessWidget {
  const WispHeader({
    super.key,
    required this.label,
    required this.online,
    required this.onSettings,
    required this.onToggle,
  });
  final String label;
  final bool online;
  final VoidCallback? onSettings, onToggle;

  @override
  Widget build(BuildContext context) => SizedBox(
    height: 64,
    child: Row(
      children: [
        Expanded(
          child: Align(
            alignment: Alignment.centerLeft,
            child: Transform.translate(
              offset: const Offset(0, 4),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Flexible(
                    child: InkWell(
                      onTap: onSettings,
                      child: Text(
                        label,
                        key: const ValueKey('device-heading'),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(
                          fontSize: 21,
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                    ),
                  ),
                  SizedBox.square(
                    dimension: 44,
                    child: IconButton(
                      tooltip: 'Settings',
                      onPressed: onSettings,
                      padding: const EdgeInsets.all(10),
                      constraints: const BoxConstraints.tightFor(
                        width: 44,
                        height: 44,
                      ),
                      icon: const Icon(
                        Icons.settings,
                        size: 24,
                        color: WispColors.muted,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
        if (online)
          Transform.translate(
            offset: const Offset(0, 6),
            child: GoOfflineButton(onPressed: onToggle),
          ),
      ],
    ),
  );
}

/// Direct port of SpaceToggle.onDraw's selected/docked branch. A Material
/// OutlinedButton changes its padding, fill, border and indicator geometry.
class GoOfflineButton extends StatelessWidget {
  const GoOfflineButton({super.key, required this.onPressed});
  final VoidCallback? onPressed;

  // SpaceChecks.buttonLayout measures 70.4 dp for the fixed caption on the
  // original Android renderer. Keep that advance across renderers: Android's
  // hinted glyph advances differ from Flutter's fractional text layout.
  static const labelAdvance = 70.4;
  static const labelBaselineOffset = 11.5 * (1900 - 500) / (2 * 2048);

  static TextPainter labelPainter() => TextPainter(
    text: const TextSpan(
      text: 'GO OFFLINE',
      style: TextStyle(
        fontFamily: 'Roboto',
        fontSize: 11.5,
        fontWeight: FontWeight.bold,
        letterSpacing: 11.5 * .08,
        color: Color(0xffedf4ff),
      ),
    ),
    textDirection: TextDirection.ltr,
    textScaler: TextScaler.noScaling,
  )..layout();

  @override
  Widget build(BuildContext context) {
    final density = MediaQuery.devicePixelRatioOf(context);
    final width = ((labelAdvance + 48) * density).ceil() / density;
    final height = (44 * density).round() / density;
    return Semantics(
      label: 'Go offline',
      toggled: true,
      enabled: onPressed != null,
      onTap: onPressed,
      child: ExcludeSemantics(
        child: Tooltip(
          message: 'Go offline',
          child: InkWell(
            onTap: onPressed,
            customBorder: const StadiumBorder(),
            child: RepaintBoundary(
              key: const ValueKey('offline-control'),
              child: CustomPaint(
                size: Size(width, height),
                painter: const _OfflinePill(),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _OfflinePill extends CustomPainter {
  const _OfflinePill();
  @override
  void paint(Canvas canvas, Size size) {
    const x = 2.0, height = 38.0;
    final y = (size.height - height) / 2, width = size.width - 4;
    final paint = Paint()..isAntiAlias = true;
    paint.color = const Color(0x334582ff);
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(x - 2, y - 2, width + 4, height + 4),
        const Radius.circular(21),
      ),
      paint,
    );
    paint.color = const Color(0xd812254a);
    final pill = RRect.fromRectAndRadius(
      Rect.fromLTWH(x, y, width, height),
      const Radius.circular(19),
    );
    canvas.drawRRect(pill, paint);
    paint
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.4
      ..color = const Color(0xff5b9cf7);
    canvas.drawRRect(pill, paint);
    final text = GoOfflineButton.labelPainter();
    const contentWidth = 8.4 + 8 + GoOfflineButton.labelAdvance;
    final bx = (size.width - contentWidth) / 2 + 4.2, by = y + height / 2;
    paint
      ..style = PaintingStyle.fill
      ..color = const Color(0x4465ceff);
    canvas.drawCircle(Offset(bx, by), 7.5, paint);
    paint.color = const Color(0xff65ceff);
    canvas.drawCircle(Offset(bx, by), 4.2, paint);
    paint.color = const Color(0xfff4fcff);
    canvas.drawCircle(Offset(bx, by), 1.8, paint);
    canvas.save();
    canvas.translate(
      bx + 12.2,
      by +
          GoOfflineButton.labelBaselineOffset -
          text.computeDistanceToActualBaseline(TextBaseline.alphabetic),
    );
    canvas.scale(GoOfflineButton.labelAdvance / text.width, 1);
    text.paint(canvas, Offset.zero);
    canvas.restore();
    text.dispose();
  }

  @override
  bool shouldRepaint(_OfflinePill oldDelegate) => false;
}
