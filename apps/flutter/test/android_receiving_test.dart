import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/sheets.dart';

class ReceivingBackend implements WispnetBackend {
  String folder = '', label = '';
  bool available = false, cancelPicker = false, conflict = false;
  List<String> allow = [];
  final calls = <(String, Map<String, dynamic>)>[];
  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    calls.add((method, Map.of(arguments)));
    switch (method) {
      case 'receivingStatus':
        return {
          'revision': 9,
          'folder': folder,
          'label': label,
          'available': available,
          'online': false,
          'clients': [
            {'peer': 'a' * 64, 'name': 'Desktop'},
          ],
          'allow': allow,
        };
      case 'pickReceivingFolder':
        return cancelPicker
            ? {}
            : {'folder': '/private/staging-id', 'label': 'Downloads'};
      case 'saveReceiving':
        if (conflict) {
          throw PlatformException(
            code: 'conflict',
            message: 'Settings changed',
          );
        }
        return {};
      default:
        throw StateError(method);
    }
  }
}

Future<void> show(WidgetTester tester, ReceivingBackend backend) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: AndroidReceivingSettings(backend: backend),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'folder picker enables an explicit paired-device grant and hides internal paths',
    (tester) async {
      final backend = ReceivingBackend();
      await show(tester, backend);
      expect(
        tester
            .widget<CheckboxListTile>(find.byType(CheckboxListTile))
            .onChanged,
        isNull,
      );
      await tester.tap(find.byKey(const ValueKey('receiving-folder-picker')));
      await tester.pumpAndSettle();
      expect(find.text('Downloads'), findsOneWidget);
      expect(find.textContaining('/private/'), findsNothing);
      await tester.tap(find.byType(CheckboxListTile));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('receiving-save')));
      await tester.pumpAndSettle();
      expect(backend.calls.last.$1, 'saveReceiving');
      expect(backend.calls.last.$2, {
        'expected_revision': 9,
        'folder': '/private/staging-id',
        'allow': ['a' * 64],
      });
    },
  );
  testWidgets(
    'canceling a folder picker preserves the configured folder and grants',
    (tester) async {
      final backend = ReceivingBackend()
        ..folder = '/private/original'
        ..label = 'Original'
        ..available = true
        ..allow = ['a' * 64]
        ..cancelPicker = true;
      await show(tester, backend);
      await tester.tap(find.byKey(const ValueKey('receiving-folder-picker')));
      await tester.pumpAndSettle();
      expect(find.text('Original'), findsOneWidget);
      expect(
        tester.widget<CheckboxListTile>(find.byType(CheckboxListTile)).value,
        isTrue,
      );
      expect(
        backend.calls.where((call) => call.$1 == 'saveReceiving'),
        isEmpty,
      );
    },
  );
  testWidgets('removing the folder revokes every Files grant', (tester) async {
    final backend = ReceivingBackend()
      ..folder = '/private/original'
      ..label = 'Original'
      ..available = true
      ..allow = ['a' * 64];
    await show(tester, backend);
    await tester.tap(find.byKey(const ValueKey('receiving-folder-remove')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('receiving-save')));
    await tester.pumpAndSettle();
    expect(backend.calls.last.$1, 'saveReceiving');
    expect(backend.calls.last.$2, {
      'expected_revision': 9,
      'folder': '',
      'allow': <String>[],
    });
  });
  testWidgets(
    'expired storage access disables saving and conflicts remain visible',
    (tester) async {
      final backend = ReceivingBackend()
        ..folder = '/private/original'
        ..label = 'Original'
        ..conflict = true;
      await show(tester, backend);
      expect(
        tester
            .widget<FilledButton>(find.byKey(const ValueKey('receiving-save')))
            .onPressed,
        isNull,
      );
      await tester.tap(find.byKey(const ValueKey('receiving-folder-picker')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('receiving-save')));
      await tester.pumpAndSettle();
      expect(find.textContaining('Settings changed'), findsOneWidget);
      expect(find.text('Downloads'), findsOneWidget);
    },
  );
}
