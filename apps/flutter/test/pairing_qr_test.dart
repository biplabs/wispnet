import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/desktop_backend.dart';
import 'package:wispnet/src/pairing_qr.dart';
import 'package:wispnet/src/qr_camera_sheet.dart';
import 'package:wispnet/src/sheets.dart';
import 'package:wispnet/ui.dart';
import 'package:zxing2/qrcode.dart' as zxing;

Future<Uint8List> qrImage(String value) async {
  final matrix = zxing.Encoder.encode(
    value,
    zxing.ErrorCorrectionLevel.m,
  ).matrix!;
  const scale = 5, quiet = 4;
  final size = (matrix.width + quiet * 2) * scale;
  final rgba = Uint8List(size * size * 4)..fillRange(0, size * size * 4, 255);
  for (var y = 0; y < matrix.height; y++) {
    for (var x = 0; x < matrix.width; x++) {
      if (matrix.get(x, y) != 1) continue;
      for (var dy = 0; dy < scale; dy++) {
        for (var dx = 0; dx < scale; dx++) {
          final offset =
              (((y + quiet) * scale + dy) * size + (x + quiet) * scale + dx) *
              4;
          rgba[offset] = rgba[offset + 1] = rgba[offset + 2] = 0;
        }
      }
    }
  }
  final ready = Completer<ui.Image>();
  ui.decodeImageFromPixels(
    rgba,
    size,
    size,
    ui.PixelFormat.rgba8888,
    ready.complete,
  );
  final image = await ready.future;
  try {
    return (await image.toByteData(format: ui.ImageByteFormat.png))!.buffer
        .asUint8List();
  } finally {
    image.dispose();
  }
}

class QrBackend implements WispnetBackend {
  bool camera = false;
  Map<String, dynamic> selection = {};
  Object? failure;
  Object? stopFailure;
  Completer<Map<String, dynamic>>? frame;
  final calls = <(String, Map<String, dynamic>)>[];
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, arguments));
    if (method == 'qrCapabilities') {
      return {'camera': camera, 'import': true};
    }
    if (method == 'importQr') {
      if (failure case final Object error) throw error;
      return selection;
    }
    if (method == 'startQrCamera') {
      frame = null;
      return {};
    }
    if (method == 'qrCameraFrame') {
      return (frame ??= Completer<Map<String, dynamic>>()).future;
    }
    if (method == 'stopQrCamera' && frame != null && !frame!.isCompleted) {
      frame!.completeError(const FormatException('Camera closed.'));
    }
    if (method == 'stopQrCamera' && stopFailure != null) throw stopFailure!;
    return {};
  }
}

Future<void> form(
  WidgetTester tester,
  QrBackend backend, {
  bool desktop = false,
}) async {
  await tester.pumpWidget(
    MaterialApp(
      theme: wispTheme().copyWith(
        platform: desktop ? TargetPlatform.linux : TargetPlatform.android,
      ),
      home: Scaffold(
        body: SingleChildScrollView(
          child: PairingForm(backend: backend, onPaired: () {}),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'a generated pairing QR decodes through the bounded desktop image pipeline',
    () async {
      expect(
        await decodePairingQr(await qrImage('wispnet1:generated-pairing-code')),
        'wispnet1:generated-pairing-code',
      );
    },
  );

  test('invalid images and non-pairing QR data are rejected', () async {
    await expectLater(decodePairingQr(Uint8List(0)), throwsFormatException);
    await expectLater(
      decodePairingQr(Uint8List(pairingQrImageLimit + 1)),
      throwsFormatException,
    );
    await expectLater(
      decodePairingQr(Uint8List.fromList([1, 2, 3])),
      throwsFormatException,
    );
    await expectLater(
      decodePairingQr(await qrImage('https://example.com')),
      throwsFormatException,
    );
    expect(
      () => normalizedPairingCode('wispnet1:${'x' * 8192}'),
      throwsFormatException,
    );
  });

  test(
    'an expired QR worker budget cancels decoding and releases the isolate',
    () async {
      final image = await qrImage('wispnet1:bounded-worker');
      await expectLater(
        decodePairingQr(image, processingTimeout: Duration.zero),
        throwsA(
          isA<FormatException>().having(
            (error) => error.message,
            'message',
            contains('timed out'),
          ),
        ),
      );
      expect(await decodePairingQr(image), 'wispnet1:bounded-worker');
    },
  );

  test('Android frame adapter decodes only bounded image bytes', () async {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    addTearDown(
      () => messenger.setMockMethodCallHandler(PlatformBackend.channel, null),
    );
    messenger.setMockMethodCallHandler(
      PlatformBackend.channel,
      (_) async => jsonEncode({
        'image_b64': base64Encode([1, 2, 3]),
      }),
    );
    expect((await PlatformBackend().invoke('qrCameraFrame'))['bytes'], [
      1,
      2,
      3,
    ]);
    for (final invalid in ['', 'not base64!', 'A' * (6 * 1024 * 1024)]) {
      messenger.setMockMethodCallHandler(
        PlatformBackend.channel,
        (_) async => jsonEncode({'image_b64': invalid}),
      );
      await expectLater(
        PlatformBackend().invoke('qrCameraFrame'),
        throwsFormatException,
      );
    }
  });

  test('desktop image import uses the native picker and never starts the node bridge', () async {
    final directory = await Directory.systemTemp.createTemp(
      'wispnet-pairing-qr-',
    );
    addTearDown(() => directory.delete(recursive: true));
    final path = '${directory.path}/pairing.png';
    await File(path).writeAsBytes(await qrImage('wispnet1:desktop-import'));
    final methods = <String>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(DesktopBackend.shell, (call) async {
          methods.add(call.method);
          return path;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(DesktopBackend.shell, null),
    );
    final backend = DesktopBackend(helper: '/must-not-start.py');
    expect(await backend.invoke('importQr'), {
      'code': 'wispnet1:desktop-import',
    });
    expect(methods, ['pickQrImage']);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(DesktopBackend.shell, (_) async => null);
    expect(await backend.invoke('importQr'), isEmpty);
    await backend.close();
  });

  for (final desktop in [false, true]) {
    testWidgets(
      '${desktop ? 'desktop' : 'Android'} uses the same QR/code form and only hides an absent camera',
      (tester) async {
        final backend = QrBackend();
        await form(tester, backend, desktop: desktop);
        expect(find.text('QR'), findsOneWidget);
        expect(find.text('Pairing Code'), findsOneWidget);
        expect(find.text('Import QR image'), findsOneWidget);
        expect(find.text('Scan with camera'), findsNothing);
        await tester.tap(find.text('Pairing Code'));
        await tester.pumpAndSettle();
        expect(find.byType(TextField), findsOneWidget);
        await tester.pumpWidget(const SizedBox());
      },
    );
  }

  testWidgets(
    'cancelled, invalid, and failed image imports never pair or replace a draft',
    (tester) async {
      final backend = QrBackend();
      await form(tester, backend);
      await tester.tap(find.text('Pairing Code'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'wispnet1:saved-draft');
      await tester.tap(find.text('QR'));
      await tester.pumpAndSettle();
      for (final invalid in [
        null,
        'not-a-pairing-code',
        PlatformException(code: 'invalid', message: 'Cannot read this image'),
      ]) {
        backend.selection = invalid is String ? {'code': invalid} : {};
        backend.failure = invalid is PlatformException ? invalid : null;
        await tester.tap(find.text('Import QR image'));
        await tester.pumpAndSettle();
        expect(backend.calls.where((call) => call.$1 == 'pair'), isEmpty);
      }
      await tester.tap(find.text('Pairing Code'));
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        'wispnet1:saved-draft',
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('successful QR import sends only the validated pairing code', (
    tester,
  ) async {
    final backend = QrBackend()
      ..selection = {'code': 'wispnet1:valid-selected-code'};
    await form(tester, backend);
    await tester.tap(find.text('Import QR image'));
    await tester.pumpAndSettle();
    expect(backend.calls.where((call) => call.$1 == 'pair').single.$2, {
      'code': 'wispnet1:valid-selected-code',
    });
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets(
    'hidden camera stops capture and requires an explicit retry on resume',
    (tester) async {
      final backend = QrBackend()..camera = true;
      await form(tester, backend, desktop: true);
      await tester.tap(find.text('Scan with camera'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      await tester.pump();
      expect(
        backend.calls.where((call) => call.$1 == 'stopQrCamera'),
        isNotEmpty,
      );
      expect(find.textContaining('Camera paused.'), findsOneWidget);
      final starts = backend.calls
          .where((call) => call.$1 == 'startQrCamera')
          .length;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      expect(
        backend.calls.where((call) => call.$1 == 'startQrCamera').length,
        starts,
      );
      await tester.ensureVisible(find.text('Try again'));
      await tester.tap(find.text('Try again'));
      await tester.pump();
      expect(
        backend.calls.where((call) => call.$1 == 'startQrCamera').length,
        starts + 1,
      );
      await tester.ensureVisible(find.text('Cancel'));
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(
        backend.calls.where((call) => call.$1 == 'pair' || call.$1 == 'scanQr'),
        isEmpty,
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  for (final desktop in [false, true]) {
    testWidgets(
      '${desktop ? 'desktop' : 'Android'} camera uses the shared page and Cancel never pairs',
      (tester) async {
        final backend = QrBackend()
          ..camera = true
          ..stopFailure = PlatformException(code: 'window_closed');
        await form(tester, backend, desktop: desktop);
        await tester.tap(find.text('Scan with camera'));
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 500));
        expect(find.byType(QrCameraSheet), findsOneWidget);
        await tester.ensureVisible(find.text('Cancel'));
        await tester.tap(find.text('Cancel'));
        await tester.pumpAndSettle();
        expect(find.byType(QrCameraSheet), findsNothing);
        expect(
          backend.calls.where((call) => call.$1 == 'stopQrCamera'),
          isNotEmpty,
        );
        expect(
          backend.calls.where(
            (call) => call.$1 == 'pair' || call.$1 == 'scanQr',
          ),
          isEmpty,
        );
        await tester.pumpWidget(const SizedBox());
      },
    );
  }
}
