import 'package:flutter/material.dart';

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

/// Compact status label used alongside service metadata.
class WispBadge extends StatelessWidget {
  const WispBadge({super.key, required this.label, required this.color});
  final String label;
  final Color color;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
    decoration: BoxDecoration(
      color: color.withValues(alpha: .12),
      borderRadius: BorderRadius.circular(8),
    ),
    child: Text(
      label,
      style: TextStyle(fontSize: 11, fontWeight: FontWeight.bold, color: color),
    ),
  );
}
