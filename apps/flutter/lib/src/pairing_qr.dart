import 'dart:async';
import 'dart:convert';
import 'dart:isolate';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:zxing2/qrcode.dart' as zxing;

const pairingQrImageLimit = 4 * 1024 * 1024;

String normalizedPairingCode(String input) {
  final code = input.replaceAll(RegExp(r'\s+'), '');
  if (!code.startsWith('wispnet1:') || utf8.encode(code).length > 8192) {
    throw const FormatException(
      'Choose a Wispnet pairing QR or paste its complete pairing code.',
    );
  }
  return code;
}

/// Check dimensions before decoding pixels, then isolate the bounded QR search.
Future<String> decodePairingQr(
  Uint8List bytes, {
  Duration processingTimeout = const Duration(seconds: 8),
}) async {
  if (bytes.isEmpty || bytes.length > pairingQrImageLimit) {
    throw const FormatException('Choose a QR image smaller than 4 MB.');
  }
  final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
  ui.ImageDescriptor? descriptor;
  ui.Codec? codec;
  ui.Image? image;
  try {
    descriptor = await ui.ImageDescriptor.encoded(buffer);
    if (descriptor.width <= 0 ||
        descriptor.height <= 0 ||
        descriptor.width > 16384 ||
        descriptor.height > 16384 ||
        descriptor.width * descriptor.height > 16 * 1024 * 1024) {
      throw const FormatException('Invalid QR image dimensions.');
    }
    final scale =
        2048 /
        (descriptor.width > descriptor.height
            ? descriptor.width
            : descriptor.height);
    codec = await descriptor.instantiateCodec(
      targetWidth: scale < 1
          ? (descriptor.width * scale).round()
          : descriptor.width,
      targetHeight: scale < 1
          ? (descriptor.height * scale).round()
          : descriptor.height,
    );
    if (codec.frameCount != 1) {
      throw const FormatException('Choose a still QR image.');
    }
    image = (await codec.getNextFrame()).image;
    final pixels = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
    if (pixels == null) {
      throw const FormatException('Cannot read this QR image.');
    }
    final replies = ReceivePort();
    Isolate? worker;
    final clock = Stopwatch()..start();
    final budget = processingTimeout < const Duration(seconds: 8)
        ? processingTimeout
        : const Duration(seconds: 8);
    try {
      worker = await Isolate.spawn(_decodePixels, [
        replies.sendPort,
        image.width,
        image.height,
        TransferableTypedData.fromList([pixels.buffer.asUint8List()]),
      ]);
      final remaining = budget - clock.elapsed;
      if (remaining <= Duration.zero) {
        throw TimeoutException('QR decode budget exceeded');
      }
      final reply = await replies.first.timeout(remaining);
      if (reply is String) return normalizedPairingCode(reply);
      throw FormatException((reply as Map)['error'] as String);
    } on TimeoutException {
      throw const FormatException(
        'QR decoding timed out. Choose a smaller, clearer image.',
      );
    } finally {
      worker?.kill(priority: Isolate.immediate);
      replies.close();
    }
  } on FormatException {
    rethrow;
  } catch (_) {
    throw const FormatException(
      'Cannot read this QR image. Choose a clear pairing QR.',
    );
  } finally {
    image?.dispose();
    codec?.dispose();
    descriptor?.dispose();
    buffer.dispose();
  }
}

void _decodePixels(List<Object> request) {
  final reply = request[0] as SendPort;
  try {
    final width = request[1] as int, height = request[2] as int;
    final rgba = (request[3] as TransferableTypedData)
        .materialize()
        .asUint8List();
    final rgb = Int32List(width * height);
    for (var index = 0; index < rgb.length; index++) {
      final offset = index * 4;
      rgb[index] =
          (rgba[offset] << 16) | (rgba[offset + 1] << 8) | rgba[offset + 2];
    }
    final source = zxing.RGBLuminanceSource(width, height, rgb);
    final reader = zxing.QRCodeReader();
    for (final inverted in [false, true]) {
      final bitmap = zxing.BinaryBitmap(
        zxing.HybridBinarizer(inverted ? source.invert() : source),
      );
      for (final pure in [false, true]) {
        final hints = zxing.DecodeHints()..put(zxing.DecodeHintType.tryHarder);
        if (pure) hints.put(zxing.DecodeHintType.pureBarcode);
        try {
          reply.send(reader.decode(bitmap, hints: hints).text);
          return;
        } on zxing.ReaderException {
          // A screenshot may need pure-symbol extraction or inverted contrast.
        }
      }
    }
    reply.send({'error': 'Cannot find a QR code. Choose a clear pairing QR.'});
  } catch (_) {
    reply.send({
      'error': 'Cannot read this QR image. Choose a clear pairing QR.',
    });
  }
}
