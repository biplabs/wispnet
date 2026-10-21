import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'src/app.dart';
import 'src/backend.dart';
import 'src/desktop_backend.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      statusBarIconBrightness: Brightness.light,
      systemNavigationBarColor: Color(0xff070b14),
      systemNavigationBarIconBrightness: Brightness.light,
    ),
  );
  runApp(
    WispnetApp(
      backend: Platform.isLinux || Platform.isMacOS
          ? DesktopBackend()
          : PlatformBackend(),
    ),
  );
}
