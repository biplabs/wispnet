import 'dart:async';

import 'package:cross_file_io/cross_file_io.dart';
// The plugin exposes its native transport override specifically for widget tests.
// ignore: implementation_imports
import 'package:desktop_drop/src/native_transport.dart';
import 'package:flutter_test/flutter_test.dart';

Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  TestWidgetsFlutterBinding.ensureInitialized();
  CrossFileIO.registerWith();
  NativeTransport.debugOverride = NativeTransport.testing()
    ..invokeHandler = (call) async {
      if (call.method == 'initialize') {
        final arguments = call.arguments as Map;
        return {'view': arguments['view'], 'generation': 1};
      }
      return null;
    };
  await testMain();
}
