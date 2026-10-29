import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/src/backend.dart';
import 'package:wispnet/src/command_center/command_center.dart';
import 'package:wispnet/src/theme.dart';

class CommandBackend implements WispnetBackend {
  bool linked = false;
  bool storedToken = false;
  bool failStatus = false;
  bool approvePoll = false;
  bool completeThenFailPoll = false;
  Map<String, dynamic>? pending;
  Map<String, dynamic>? unlinkResult;
  Completer<Map<String, dynamic>>? save;
  Completer<Map<String, dynamic>>? linking;
  final requests = <Map<String, dynamic>>[];
  final methods = <String>[];
  final launches = <Map<String, dynamic>>[];

  @override
  Future<Map<String, dynamic>> invoke(
    String method, [
    Map<String, dynamic> arguments = const {},
  ]) async {
    methods.add(method);
    if (method == 'launchCommandCenter') launches.add(arguments);
    if (method != 'desktopRequest') return {};
    final payload = arguments['payload'] as Map<String, dynamic>;
    requests.add(payload);
    if (payload['op'] == 'command_center_status') {
      if (failStatus) throw const FormatException('Local node unavailable');
      return {
        'enabled': true,
        'base_url': 'https://cmd.wispnet.app',
        'linked': linked,
        if (linked) 'account': {'id': 'account-id', 'name': 'Alice'},
        if (pending != null) 'pending': pending,
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
    if (payload['op'] == 'command_center_link') {
      pending = {
        'user_code': 'ABCD-EF01-2345',
        'verification_url':
            'https://cmd.wispnet.app/connect?code=ABCD-EF01-2345',
      };
      return linking?.future ?? {'state': 'pending', ...pending!};
    }
    if (payload['op'] == 'command_center_poll') {
      if (completeThenFailPoll) {
        linked = true;
        pending = null;
        throw const FormatException('Another poll completed this link');
      }
      if (approvePoll) {
        linked = true;
        pending = null;
        return {'state': 'linked'};
      }
      return {'state': 'pending'};
    }
    if (payload['op'] == 'configure_backend_auth' && save != null) {
      return save!.future;
    }
    if (payload['op'] == 'command_center_unlink') {
      linked = false;
      pending = null;
      return unlinkResult ??
          {'linked': false, 'remote_revocation': 'confirmed'};
    }
    return {};
  }
}

Future<void> mountAccount(
  WidgetTester tester,
  CommandBackend backend, {
  bool enabled = true,
}) async {
  await tester.pumpWidget(
    MaterialApp(
      theme: wispTheme(),
      home: Scaffold(
        body: Column(
          children: [
            const Text('Private device access'),
            CommandCenterAccountButton(
              backend: backend,
              enabled: enabled,
              onChanged: () {},
            ),
          ],
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
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
  testWidgets(
    'home account entry is removable and does not poll between frames',
    (tester) async {
      final disabled = CommandBackend();
      await mountAccount(tester, disabled, enabled: false);
      expect(find.byKey(const ValueKey('public-account')), findsNothing);
      expect(disabled.methods, isEmpty);
      expect(find.text('Private device access'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());

      final backend = CommandBackend();
      await mountAccount(tester, backend);
      expect(
        find.widgetWithText(TextButton, 'Sign in with GitHub'),
        findsOneWidget,
      );
      expect(backend.requests, hasLength(1));
      await tester.pump(const Duration(seconds: 8));
      await tester.pump();
      expect(backend.requests, hasLength(1));
      backend.linked = true;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();
      expect(find.widgetWithText(TextButton, 'Alice'), findsOneWidget);
      expect(backend.requests, hasLength(2));
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'account entry opens only account controls and explains signout',
    (tester) async {
      final backend = CommandBackend()..linked = true;
      await mountAccount(tester, backend);
      await tester.tap(find.byKey(const ValueKey('public-account')));
      await tester.pumpAndSettle();
      expect(find.text('Connected as Alice'), findsOneWidget);
      expect(find.text('Enable public access'), findsNothing);
      expect(find.text('Backend authentication'), findsNothing);
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Manage public access'),
      );
      await tester.pumpAndSettle();
      expect(backend.launches.single['purpose'], 'dashboard');
      await tester.tap(find.widgetWithText(OutlinedButton, 'Sign out'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('stop public publishing linked to this account'),
        findsOneWidget,
      );
      expect(
        backend.requests.where((r) => r['op'] == 'command_center_unlink'),
        isEmpty,
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await tester.pumpAndSettle();
      expect(
        backend.requests.where((r) => r['op'] == 'command_center_unlink'),
        hasLength(1),
      );
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      expect(
        find.widgetWithText(TextButton, 'Sign in with GitHub'),
        findsOneWidget,
      );
      expect(find.text('Private device access'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'offline account controls expose retry while private use stays available',
    (tester) async {
      final backend = CommandBackend()..failStatus = true;
      await mountAccount(tester, backend);
      await tester.tap(find.byKey(const ValueKey('public-account')));
      await tester.pumpAndSettle();
      expect(find.textContaining('Start or update Wispnet'), findsOneWidget);
      expect(
        find.widgetWithText(OutlinedButton, 'Retry account status'),
        findsOneWidget,
      );
      backend.failStatus = false;
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Retry account status'),
      );
      await tester.pumpAndSettle();
      expect(
        find.widgetWithText(OutlinedButton, 'Sign in with GitHub'),
        findsOneWidget,
      );
      expect(
        backend.requests.where((r) => r['op'] == 'command_center_link'),
        isEmpty,
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'browser linking guards the displayed flow and recovers another successful poll',
    (tester) async {
      final backend = CommandBackend();
      await mount(tester, backend, serviceId: null);
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Sign in with GitHub'),
      );
      await tester.pumpAndSettle();
      expect(backend.launches.single, {
        'purpose': 'link',
        'expected_user_code': 'ABCD-EF01-2345',
        'expected_verification_url':
            'https://cmd.wispnet.app/connect?code=ABCD-EF01-2345',
      });
      backend.completeThenFailPoll = true;
      await tester.tap(find.widgetWithText(OutlinedButton, 'Check connection'));
      await tester.pumpAndSettle();
      expect(
        backend.requests.singleWhere((r) => r['op'] == 'command_center_poll'),
        {
          'op': 'command_center_poll',
          'expected_user_code': 'ABCD-EF01-2345',
          'expected_verification_url':
              'https://cmd.wispnet.app/connect?code=ABCD-EF01-2345',
        },
      );
      expect(find.text('Connected as Alice'), findsOneWidget);
      expect(
        find.textContaining('Could not check browser approval'),
        findsNothing,
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'replaced pending login is not consumed and closing never unlinks',
    (tester) async {
      final backend = CommandBackend();
      await mountAccount(tester, backend);
      await tester.tap(find.byKey(const ValueKey('public-account')));
      await tester.pumpAndSettle();
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Sign in with GitHub'),
      );
      await tester.pumpAndSettle();
      backend.pending = {
        'user_code': '9999-8888-7777',
        'verification_url':
            'https://cmd.wispnet.app/connect?code=9999-8888-7777',
      };
      await tester.tap(find.widgetWithText(OutlinedButton, 'Check connection'));
      await tester.pumpAndSettle();
      expect(
        backend.requests.where((r) => r['op'] == 'command_center_poll'),
        isEmpty,
      );
      expect(
        find.textContaining('Another sign-in was started'),
        findsOneWidget,
      );
      expect(
        find.widgetWithText(OutlinedButton, 'Start again'),
        findsOneWidget,
      );
      expect(find.text('9999-8888-7777'), findsNothing);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      expect(
        backend.requests.where((r) => r['op'] == 'command_center_unlink'),
        isEmpty,
      );
      expect(backend.pending!['user_code'], '9999-8888-7777');
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'closing while sign-in starts keeps its node flow without a late browser launch',
    (tester) async {
      final backend = CommandBackend()
        ..linking = Completer<Map<String, dynamic>>();
      await mountAccount(tester, backend);
      await tester.tap(find.byKey(const ValueKey('public-account')));
      await tester.pumpAndSettle();
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Sign in with GitHub'),
      );
      await tester.pump();
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();
      backend.linking!.complete({'state': 'pending', ...backend.pending!});
      await tester.pumpAndSettle();
      expect(backend.launches, isEmpty);
      expect(backend.pending, isNotNull);
      expect(
        backend.requests.where((r) => r['op'] == 'command_center_unlink'),
        isEmpty,
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'browser linking stops waiting after ten minutes and offers restart',
    (tester) async {
      final backend = CommandBackend();
      await mount(tester, backend, serviceId: null);
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Sign in with GitHub'),
      );
      await tester.pumpAndSettle();
      await tester.pump(const Duration(minutes: 10));
      await tester.pumpAndSettle();
      expect(find.textContaining('Sign-in timed out'), findsOneWidget);
      expect(
        find.widgetWithText(OutlinedButton, 'Start again'),
        findsOneWidget,
      );
      expect(
        find.widgetWithText(OutlinedButton, 'Check connection'),
        findsNothing,
      );
      final calls = backend.requests
          .where((r) => r['op'] == 'command_center_poll')
          .length;
      await tester.pump(const Duration(seconds: 5));
      await tester.pumpAndSettle();
      expect(
        backend.requests.where((r) => r['op'] == 'command_center_poll'),
        hasLength(calls),
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

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
      expect(find.text('Sign in with GitHub'), findsOneWidget);
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
      await tester.tap(find.widgetWithText(OutlinedButton, 'Sign out'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Sign out'));
      await tester.pumpAndSettle();
      expect(find.text(warning), findsOneWidget);
      expect(find.text('Connected as Alice'), findsNothing);
      expect(find.text('Sign in with GitHub'), findsOneWidget);
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
      expect(find.text('Sign in with GitHub'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
}
