import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

/// A bounded camera stream; image bytes stay in memory and are never saved.
class DesktopQrCamera {
  Process? _process;
  StreamSubscription<List<int>>? _output;
  final _buffer = <int>[];
  Completer<Uint8List>? _waiting;
  Uint8List? _latest;
  String? _failure;
  int _generation = 0;
  List<String> _devices = [];
  int _deviceIndex = 0;

  static Future<List<String>> linuxDevices() async {
    if (!Platform.isLinux) return [];
    final devices = await Directory('/dev')
        .list(followLinks: false)
        .where((entry) => RegExp(r'^/dev/video[0-9]+$').hasMatch(entry.path))
        .map((entry) => entry.path)
        .toList();
    devices.sort();
    return devices;
  }

  Future<void> start() async {
    await stop();
    final current = ++_generation;
    final devices = await linuxDevices();
    if (devices.isEmpty) {
      throw const FormatException(
        'No camera is available. Import a QR image instead.',
      );
    }
    _devices = devices.take(8).toList();
    _deviceIndex = 0;
    await _openDevice(current);
  }

  Future<void> _openDevice(int current) async {
    if (current != _generation) throw const FormatException('Camera closed.');
    _failure = null;
    try {
      final process = await Process.start('ffmpeg', [
        '-nostdin',
        '-hide_banner',
        '-loglevel',
        'error',
        '-f',
        'v4l2',
        '-video_size',
        '640x480',
        '-i',
        _devices[_deviceIndex],
        '-vf',
        'fps=4,scale=640:-2',
        '-f',
        'image2pipe',
        '-vcodec',
        'mjpeg',
        'pipe:1',
      ]);
      if (current != _generation) {
        // Cancellation can win while Process.start is still opening the
        // camera, before normal stop() has a process handle to close.
        await _terminate(process);
        throw const FormatException('Camera closed.');
      }
      _process = process;
    } on ProcessException {
      throw const FormatException(
        'Camera capture needs FFmpeg on this computer. Import a QR image or install FFmpeg.',
      );
    }
    final process = _process!;
    unawaited(process.stderr.drain<void>());
    _output = process.stdout.listen(
      _receive,
      onError: (_) =>
          _fail('Camera capture failed. Check camera access and try again.'),
    );
    unawaited(
      process.exitCode.then((_) {
        if (identical(_process, process)) {
          _fail('Camera capture stopped. Check camera access and try again.');
        }
      }),
    );
  }

  void _receive(List<int> data) {
    _buffer.addAll(data);
    if (_buffer.length > 4 * 1024 * 1024) {
      _fail('Camera image exceeded the size limit.');
      _process?.kill();
      return;
    }
    while (_buffer.length >= 4) {
      var start = -1, end = -1;
      for (var index = 0; index + 1 < _buffer.length; index++) {
        if (start < 0 && _buffer[index] == 0xff && _buffer[index + 1] == 0xd8) {
          start = index;
        }
        if (start >= 0 &&
            _buffer[index] == 0xff &&
            _buffer[index + 1] == 0xd9) {
          end = index + 2;
          break;
        }
      }
      if (end < 0) return;
      final frame = Uint8List.fromList(_buffer.sublist(start, end));
      _buffer.removeRange(0, end);
      final waiting = _waiting;
      _waiting = null;
      if (waiting != null && !waiting.isCompleted) {
        waiting.complete(frame);
      } else {
        _latest = frame;
      }
    }
  }

  void _fail(String message) {
    _failure = message;
    final waiting = _waiting;
    _waiting = null;
    if (waiting != null && !waiting.isCompleted) {
      waiting.completeError(FormatException(message));
    }
  }

  Future<Uint8List> frame() async {
    final current = _generation;
    final deadline = DateTime.now().add(const Duration(seconds: 20));
    while (true) {
      try {
        return await _takeFrame();
      } on FormatException {
        // UVC cameras can expose metadata-only video nodes. Try another node
        // instead of treating the first failed capture as absent hardware.
        if (current != _generation ||
            _deviceIndex + 1 >= _devices.length ||
            DateTime.now().isAfter(deadline)) {
          rethrow;
        }
        await _closeProcess();
        _deviceIndex++;
        await _openDevice(current);
      }
    }
  }

  Future<Uint8List> _takeFrame() async {
    if (_failure case final String failure) throw FormatException(failure);
    if (_process == null) throw const FormatException('Camera is not running.');
    final latest = _latest;
    _latest = null;
    if (latest != null) return latest;
    final waiting = _waiting ??= Completer<Uint8List>();
    try {
      return await waiting.future.timeout(const Duration(seconds: 6));
    } on TimeoutException {
      throw const FormatException(
        'No camera image received. Check camera access and try again.',
      );
    } finally {
      if (identical(_waiting, waiting)) _waiting = null;
    }
  }

  Future<void> stop() async {
    _generation++;
    _devices = [];
    _deviceIndex = 0;
    await _closeProcess();
  }

  Future<void> _closeProcess() async {
    final process = _process;
    _process = null;
    process?.kill();
    await _output?.cancel();
    _output = null;
    _buffer.clear();
    _latest = null;
    _fail('Camera closed.');
    if (process != null) {
      await _terminate(process);
    }
  }

  Future<void> _terminate(Process process) async {
    process.kill();
    try {
      await process.exitCode.timeout(const Duration(seconds: 2));
    } on TimeoutException {
      process.kill(ProcessSignal.sigkill);
    }
  }
}
