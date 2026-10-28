import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/command_center/command_center.dart';
import 'package:wispnet/src/theme.dart';

class CommandBackend implements WispnetBackend {
  bool linked = false;
  bool storedToken = false;
  Map<String, dynamic>? unlinkResult;
  Completer<Map<String, dynamic>>? save;
  final requests = <Map<String, dynamic>>[];
  final methods = <String>[];

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    methods.add(method);
    if (method != 'desktopRequest') return {};
    final payload = arguments['payload'] as Map<String, dynamic>;
    requests.add(payload);
    if (payload['op'] == 'command_center_status') {
      return {
        'enabled': true,
        'base_url': 'https://cmd.wispnet.app',
        'linked': linked,
        if (linked) 'account': {'id': 'account-id', 'name': 'Alice'},
        'credentials': [
          {
            'service': 'gike',
            'incarnation': 'c' * 64,
            'mode': storedToken ? 'bearer' : 'passthrough',
            'configured': storedToken,
          },
        ],
      };
    }
    if (payload['op'] == 'configure_backend_auth' && save != null) {
      return save!.future;
    }
    if (payload['op'] == 'command_center_unlink') {
      linked = false;
      return unlinkResult ??
          {'linked': false, 'remote_revocation': 'confirmed'};
    }
    return {};
  }
}

Map<String, dynamic> hostStatus() => {
  'host_name': 'My computer',
  'host': {
    'services': [
      {'id': 'gike', 'kind': 'http', 'incarnation': 'c' * 64},
      {'id': 'shell', 'kind': 'shell', 'incarnation': 'd' * 64},
    ],
  },
  'public_access': {'services': <Map<String, dynamic>>[]},
};

Future<void> mount(
  WidgetTester tester,
  CommandBackend backend, {
  bool enabled = true,
  String? serviceId = 'gike',
}) async {
  await tester.pumpWidget(
    MaterialApp(
      theme: wispTheme(),
      home: Scaffold(
        body: SingleChildScrollView(
          child: Column(
            children: [
              const Text('Private device access'),
              CommandCenterModule(
                backend: backend,
                hostStatus: hostStatus(),
                serviceId: serviceId,
                enabled: enabled,
                onChanged: () {},
              ),
            ],
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('default module follows the build feature flag', (tester) async {
    final backend = CommandBackend();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: CommandCenterModule(
              backend: backend,
              hostStatus: hostStatus(),
              onChanged: () {},
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      find.text('Command Center'),
      commandCenterEnabled ? findsOneWidget : findsNothing,
    );
    expect(backend.methods.isEmpty, !commandCenterEnabled);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'disabled module leaves private controls and performs no account calls',
    (tester) async {
      final backend = CommandBackend();
      await mount(tester, backend, enabled: false);
      expect(find.text('Private device access'), findsOneWidget);
      expect(find.text('Command Center'), findsNothing);
      expect(backend.methods, isEmpty);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'public publishing requires a linked account while backend settings stay local',
    (tester) async {
      final backend = CommandBackend();
      await mount(tester, backend);
      expect(find.text('Connect GitHub account'), findsOneWidget);
      final publish = tester.widget<OutlinedButton>(
        find.widgetWithText(OutlinedButton, 'Enable public access'),
      );
      expect(publish.onPressed, isNull);
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Backend authentication'),
            )
            .onPressed,
        isNotNull,
      );
      expect(
        backend.requests.where((r) => r['op'] == 'configure_public'),
        isEmpty,
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'managed bearer input is masked, never loaded, and cleared before saving',
    (tester) async {
      final backend = CommandBackend()..storedToken = true;
      await mount(tester, backend);
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Backend authentication'),
      );
      await tester.pumpAndSettle();
      final input = find.byKey(const ValueKey('backend-bearer-input'));
      final field = tester.widget<TextField>(input);
      expect(field.obscureText, isTrue);
      expect(field.controller!.text, isEmpty);
      backend.save = Completer<Map<String, dynamic>>();
      await tester.enterText(input, 'fixture-backend-credential');
      await tester.ensureVisible(find.text('Save backend authentication'));
      await tester.tap(find.text('Save backend authentication'));
      await tester.pump();
      expect(tester.widget<TextField>(input).controller!.text, isEmpty);
      final saved = backend.requests.singleWhere(
        (r) => r['op'] == 'configure_backend_auth',
      );
      expect(saved['service'], 'gike');
      expect(saved['incarnation'], 'c' * 64);
      expect(saved['token'], 'fixture-backend-credential');
      backend.save!.complete({});
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('backend-bearer-input')), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('publishing preserves the case of the selected MCP endpoint', (
    tester,
  ) async {
    final backend = CommandBackend()..linked = true;
    await mount(tester, backend);
    await tester.tap(
      find.widgetWithText(OutlinedButton, 'Enable public access'),
    );
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '/MCP');
    await tester.tap(find.widgetWithText(FilledButton, 'Enable public access'));
    await tester.pumpAndSettle();
    final published = backend.requests.singleWhere(
      (r) => r['op'] == 'configure_public',
    );
    expect(published['managed'], isTrue);
    expect(published['mcp_path'], '/MCP');
    expect(published['incarnation'], 'c' * 64);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'non-HTTP service settings never load accounts or offer publishing',
    (tester) async {
      final backend = CommandBackend();
      await mount(tester, backend, serviceId: 'shell');
      expect(find.text('Private device access'), findsOneWidget);
      expect(find.text('Command Center'), findsNothing);
      expect(backend.methods, isEmpty);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'local disconnect succeeds and preserves an unconfirmed revocation warning through refresh',
    (tester) async {
      const warning =
          'Local publishing stopped. Remote credential revocation could not be confirmed; review this host in Command Center.';
      final backend = CommandBackend()
        ..linked = true
        ..unlinkResult = {
          'linked': false,
          'remote_revocation': 'unconfirmed',
          'warning': warning,
        };
      await mount(tester, backend);
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Disconnect account'),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Disconnect account'));
      await tester.pumpAndSettle();
      expect(find.text(warning), findsOneWidget);
      expect(find.text('Connected as Alice'), findsNothing);
      expect(find.text('Connect GitHub account'), findsOneWidget);
      expect(find.text('Private device access'), findsOneWidget);
      expect(find.text('Open Command Center'), findsOneWidget);
      final before = backend.requests
          .where((request) => request['op'] == 'command_center_status')
          .length;
      await tester.pump(const Duration(seconds: 6));
      await tester.pumpAndSettle();
      expect(
        backend.requests
            .where((request) => request['op'] == 'command_center_status')
            .length,
        greaterThan(before),
      );
      expect(find.text(warning), findsOneWidget);
      expect(find.text('Connect GitHub account'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
}
