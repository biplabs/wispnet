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

/// Consistent destructive actions in settings and confirmation sheets.
ButtonStyle wispDangerButtonStyle() => OutlinedButton.styleFrom(
  foregroundColor: WispColors.error,
  side: const BorderSide(color: WispColors.error),
).copyWith(backgroundColor: wispButtonBackground(const Color(0xff401e2a)));
