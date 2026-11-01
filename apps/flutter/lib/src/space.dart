import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../ui.dart';

/// The Android starfield, rendered by Flutter on every target.
class WispSpace extends StatefulWidget {
  const WispSpace({
    super.key,
    required this.online,
    required this.warp,
    required this.child,
  });
  final bool online;
  final Animation<double> warp;
  final Widget child;
  @override
  State<WispSpace> createState() => _WispSpaceState();
}

class _WispSpaceState extends State<WispSpace>
    with SingleTickerProviderStateMixin {
  late final AnimationController cruise = AnimationController(
    vsync: this,
    duration: const Duration(seconds: 120),
  );
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      cruise.stop();
    } else if (!cruise.isAnimating) {
      cruise.repeat();
    }
  }

  @override
  void dispose() {
    cruise.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final reduced = MediaQuery.disableAnimationsOf(context);
    return Stack(
      fit: StackFit.expand,
      children: [
        RepaintBoundary(
          child: CustomPaint(
            painter: _Stars(
              cruise: cruise,
              warp: widget.warp,
              online: widget.online,
              reduced: reduced,
            ),
          ),
        ),
        widget.child,
      ],
    );
  }
}

class _Stars extends CustomPainter {
  _Stars({
    required this.cruise,
    required this.warp,
    required this.online,
    required this.reduced,
  }) : super(repaint: reduced ? null : Listenable.merge([cruise, warp]));
  final Animation<double> cruise, warp;
  final bool online, reduced;
  static final stars = List<double>.generate(480, (_) => _random.nextDouble());
  static final _random = math.Random(17);
  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero), scale = center.distance;
    final t = reduced ? 0.0 : cruise.value, p = reduced ? 0.0 : warp.value;
    final surge = math.sin(math.pi * p);
    final paint = Paint()..strokeCap = StrokeCap.round;
    final glow = Rect.fromCircle(
      center: Offset(size.width / 2, size.height * .4),
      radius: size.longestSide * .75,
    );
    paint.shader = RadialGradient(
      colors: [
        online ? const Color(0x5522569e) : const Color(0x48264b8a),
        const Color(0x2414284c),
        Colors.transparent,
      ],
    ).createShader(glow);
    canvas.drawRect(Offset.zero & size, paint);
    paint.shader = null;
    for (var i = 0; i < 480; i += 3) {
      final a = stars[i] * math.pi * 2;
      final depth =
          (stars[i + 1] +
              t * (online ? 9 : 5) * (.6 + stars[i + 2] * .8) +
              p * .75) %
          1;
      final radius = math.max(4.0, (.03 + depth * depth) * scale);
      final direction = Offset(math.cos(a), math.sin(a));
      final tail = p > 0 && p < 1
          ? surge * surge * (.025 + depth * .42) * scale
          : depth * depth * (online ? 13 : 6);
      final color = i % 15 == 0
          ? const Color(0xffffdfa8)
          : i % 9 == 0
          ? const Color(0xff7da2ff)
          : i % 6 == 0
          ? WispColors.cyan
          : const Color(0xfff0f8ff);
      paint
        ..color = color.withValues(alpha: (.25 + depth * .6).clamp(0, 1))
        ..strokeWidth = 1.1 + depth * 2.1 * (.6 + stars[i + 2] * .6);
      if (tail < 1.5) {
        canvas.drawCircle(
          center + direction * radius,
          paint.strokeWidth / 2,
          paint,
        );
      } else {
        canvas.drawLine(
          center + direction * radius,
          center + direction * (radius + tail),
          paint,
        );
      }
    }
    if (p > .76 && p < 1) {
      final wave = (p - .76) / .24;
      paint
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3.2 - wave * 2.2
        ..color = WispColors.cyan.withValues(alpha: (1 - wave) * .65);
      canvas.drawCircle(center, (.06 + wave * .78) * scale, paint);
    }
  }

  @override
  bool shouldRepaint(_Stars old) =>
      old.online != online || old.reduced != reduced;
}

class SpaceToggle extends StatelessWidget {
  const SpaceToggle({
    super.key,
    required this.first,
    required this.busy,
    required this.beacons,
    required this.onPressed,
  });
  final bool first, busy;
  final List<Color> beacons;
  final VoidCallback? onPressed;
  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: first ? 'Establish your wisp network' : 'Enter wisp space',
    child: Tooltip(
      message: first ? 'Establish your wisp network' : 'Enter wisp space',
      child: WispInkWell(
        onTap: onPressed,
        customBorder: const CircleBorder(),
        child: SizedBox(
          width: 108,
          height: 108,
          child: CustomPaint(
            painter: _Core(first, beacons),
            child: busy
                ? const Center(
                    child: SizedBox(
                      width: 24,
                      height: 24,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  )
                : null,
          ),
        ),
      ),
    ),
  );
}

class _Core extends CustomPainter {
  _Core(this.plus, this.beacons);
  final bool plus;
  final List<Color> beacons;
  @override
  void paint(Canvas c, Size size) {
    final center = size.center(Offset.zero), p = Paint()..isAntiAlias = true;
    p.color = const Color(0x284582ff);
    c.drawCircle(center, 43, p);
    p.color = const Color(0xff0f1c36);
    c.drawCircle(center, 38, p);
    p
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.6
      ..color = const Color(0xff528ef2);
    c.drawCircle(center, 38, p);
    p
      ..strokeWidth = 1.3
      ..color = const Color(0x8865ceff);
    for (var a = 0; a < 4; a++) {
      c.drawArc(
        Rect.fromCircle(center: center, radius: 46),
        (a * 90 + 18) * math.pi / 180,
        54 * math.pi / 180,
        false,
        p,
      );
    }
    if (plus) {
      p
        ..strokeCap = StrokeCap.round
        ..strokeWidth = 3.2
        ..color = WispColors.text;
      c.drawLine(center - const Offset(13, 0), center + const Offset(13, 0), p);
      c.drawLine(center - const Offset(0, 13), center + const Offset(0, 13), p);
    } else {
      p
        ..strokeWidth = 1.1
        ..color = const Color(0x4465ceff);
      c.drawCircle(center, 23.6, p);
      p
        ..style = PaintingStyle.fill
        ..color = const Color(0x3365ceff);
      c.drawCircle(center, 11, p);
      p.color = WispColors.cyan;
      c.drawCircle(center, 4.5, p);
      p.color = Colors.white;
      c.drawCircle(center, 2, p);
      p
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.2;
      c.drawLine(
        center + const Offset(-7, -10),
        center + const Offset(0, -16),
        p,
      );
      c.drawLine(
        center + const Offset(7, -10),
        center + const Offset(0, -16),
        p,
      );
      for (var i = 0; i < beacons.length; i++) {
        final angle = -math.pi / 2 + i * math.pi * 2 / beacons.length;
        final point = center + Offset(math.cos(angle), math.sin(angle)) * 46;
        p
          ..style = PaintingStyle.fill
          ..color = WispColors.background;
        c.drawCircle(point, 5.2, p);
        p.color = beacons[i];
        c.drawCircle(point, 3.6, p);
      }
    }
  }

  @override
  bool shouldRepaint(_Core old) => true;
}
