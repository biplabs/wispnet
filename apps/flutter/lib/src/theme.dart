import 'package:flutter/material.dart';

abstract final class WispColors {
  static const background = Color(0xff070b14);
  static const sheet = Color(0xff0b1324);
  static const panel = Color(0xff101a30);
  static const tile = Color(0xff111e38);
  static const border = Color(0xff294270);
  static const text = Color(0xffedf2ff);
  static const muted = Color(0xff94a6c4);
  static const cyan = Color(0xff65ceff);
  static const green = Color(0xff7be0b8);
  static const amber = Color(0xfff0b85c);
  static const error = Color(0xffff9aa8);
}

ThemeData wispTheme() => ThemeData(
  brightness: Brightness.dark,
  scaffoldBackgroundColor: WispColors.background,
  colorScheme: const ColorScheme.dark(
    primary: WispColors.cyan,
    surface: WispColors.sheet,
    error: WispColors.error,
  ),
  fontFamily: 'Roboto',
  textTheme: const TextTheme(
    bodyMedium: TextStyle(color: WispColors.text),
    bodySmall: TextStyle(color: WispColors.muted),
  ),
  filledButtonTheme: FilledButtonThemeData(
    style: FilledButton.styleFrom(
      backgroundColor: const Color(0xff1f3666),
      foregroundColor: WispColors.text,
      minimumSize: const Size(48, 48),
      side: const BorderSide(color: Color(0xff4b73c2)),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
    ),
  ),
  outlinedButtonTheme: OutlinedButtonThemeData(
    style: OutlinedButton.styleFrom(
      foregroundColor: WispColors.text,
      backgroundColor: const Color(0xff142038),
      minimumSize: const Size(48, 48),
      side: const BorderSide(color: Color(0xff283b60)),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
    ),
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

Future<T?> showWispSheet<T>(
  BuildContext context, {
  required String title,
  required Widget child,
}) => showModalBottomSheet<T>(
  context: context,
  isScrollControlled: true,
  useSafeArea: true,
  backgroundColor: Colors.transparent,
  constraints: const BoxConstraints(maxWidth: 640),
  builder: (context) => Padding(
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
        padding: const EdgeInsets.fromLTRB(20, 12, 20, 24),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Center(
                child: Container(
                  width: 40,
                  height: 4,
                  margin: const EdgeInsets.only(bottom: 12),
                  decoration: BoxDecoration(
                    color: WispColors.muted.withValues(alpha: .4),
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),
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
              child,
            ],
          ),
        ),
      ),
    ),
  ),
);

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
