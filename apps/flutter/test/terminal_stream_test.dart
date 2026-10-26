import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/terminal_sheet.dart';

class _DelayedShellBackend implements WispnetBackend {
  final opened = Completer<Map<String, dynamic>>();
  final calls = <(String, Map<String, dynamic>)>[];
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, arguments));
    if (method == 'shellOpen') return opened.future;
    return {};
  }
}

String firstLine(TerminalEmulatorBuffer buffer) =>
    buffer.lines.first.map((cell) => cell.char).join().trimRight();
void main() {
  test('terminal preserves escape sequences at every frame boundary', () {
    const output =
        '\x1b[31mred\x1b[0m\x1b]2;title\x1b\\\x1bP+q696e646e\x1b\\\x1b(B\x1b[6n';
    final completeReplies = <String>[];
    TerminalEmulatorBuffer(onReply: completeReplies.add).writeString(output);
    for (var split = 1; split < output.length; split++) {
      final replies = <String>[];
      final buffer = TerminalEmulatorBuffer(onReply: replies.add);
      buffer.writeString(output.substring(0, split));
      buffer.writeString(output.substring(split));
      expect(firstLine(buffer), 'red', reason: 'split at $split');
      expect(buffer.lastTitle, 'title', reason: 'split at $split');
      expect(replies, completeReplies, reason: 'split at $split');
      expect(buffer.lines.first.first.fg, const Color(0xFFFF6B7B));
    }
  });
  test('terminal waits for control string terminators before replying', () {
    final replies = <String>[];
    final buffer = TerminalEmulatorBuffer(onReply: replies.add);
    buffer.writeString('\x1b]2;part');
    expect(buffer.lastTitle, isNull);
    buffer.writeString('ial\x07\x1bP+q696e');
    expect(buffer.lastTitle, 'partial');
    expect(replies, isEmpty);
    buffer.writeString('646e\x1b');
    expect(replies, isEmpty);
    buffer.writeString('\\ok');
    expect(replies, ['\x1bP0+r\x1b\\']);
    expect(firstLine(buffer), 'ok');
  });
  test('terminal decodes multibyte UTF-8 across every frame boundary', () {
    const text = 'café € 中文 😀';
    final bytes = utf8.encode(text);
    for (var split = 1; split < bytes.length; split++) {
      final buffer = TerminalEmulatorBuffer();
      buffer.writeBytes(bytes.sublist(0, split));
      buffer.writeBytes(bytes.sublist(split));
      expect(firstLine(buffer), text, reason: 'split at $split');
    }
    final buffer = TerminalEmulatorBuffer();
    for (final byte in bytes) {
      buffer.writeBytes([byte]);
    }
    expect(firstLine(buffer), text);
  });
  test('terminal reset discards incomplete decoder and escape state', () {
    final buffer = TerminalEmulatorBuffer();
    buffer.writeString('\x1b[3');
    buffer.writeBytes([0xe2]);
    buffer.reset();
    buffer.writeBytes(utf8.encode('fresh'));
    expect(firstLine(buffer), 'fresh');
  });
  for (final terminate in [true, false]) {
    test('closing a connecting pane preserves terminate=$terminate', () async {
      final backend = _DelayedShellBackend();
      final pane = DesktopShellPane(
        backend: backend,
        host: 'host',
        hostName: 'host',
        service: 'shell',
        incarnation: 'incarnation',
        session: 'main',
      );
      final connecting = pane.connect();
      pane.close(terminate: terminate);
      backend.opened.complete({'session': 'handle'});
      await connecting;
      final closeCalls = backend.calls.where((call) => call.$1 == 'shellClose');
      expect(closeCalls, hasLength(1));
      expect(closeCalls.single.$2, {
        'session': 'handle',
        'terminate': terminate,
      });
      expect(pane.handle, isNull);
      pane.dispose();
    });
  }
}
