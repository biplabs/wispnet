import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wispnet/ui.dart';

class _FlowContent extends StatefulWidget {
  const _FlowContent(this.level, {this.completion});
  final int level;
  final Completer<void>? completion;

  @override
  State<_FlowContent> createState() => _FlowContentState();
}

class _FlowContentState extends State<_FlowContent> {
  String? result;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      TextField(
        key: ValueKey('draft-${widget.level}'),
        decoration: const InputDecoration(labelText: 'Draft'),
      ),
      if (widget.level < 2)
        TextButton(
          key: ValueKey('next-${widget.level}'),
          onPressed: () async {
            final value = await showWispSheet<String>(
              context,
              title: 'Page ${widget.level + 2}',
              child: _FlowContent(
                widget.level + 1,
                completion: widget.completion,
              ),
            );
            if (mounted && wispPageIsCurrent(context)) {
              setState(() => result = value);
            }
          },
          child: const Text('Next page'),
        ),
      if (widget.level > 0) ...[
        TextButton(
          onPressed: () => Navigator.pop(context, 'saved'),
          child: const Text('Save'),
        ),
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
      ],
      if (result != null) Text('Result: $result'),
      if (widget.completion != null)
        TextButton(
          onPressed: () async {
            await widget.completion!.future;
            if (mounted && wispPageIsCurrent(context)) {
              Navigator.pop(context, 'saved');
            }
          },
          child: const Text('Deferred save'),
        ),
    ],
  );
}

class _LongFlowContent extends StatelessWidget {
  const _LongFlowContent({this.nested = false});
  final bool nested;

  @override
  Widget build(BuildContext context) => Column(
    children: [
      if (!nested)
        TextButton(
          onPressed: () => showWispSheet<void>(
            context,
            title: 'Long child page',
            child: const _LongFlowContent(nested: true),
          ),
          child: const Text('Open long child'),
        ),
      for (var row = 0; row < 80; row++)
        SizedBox(height: 44, child: Text('Row $row')),
    ],
  );
}

Future<void> mountFlow(
  WidgetTester tester, {
  double width = 390,
  Completer<void>? completion,
  Widget? content,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = Size(width, 800);
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  await tester.pumpWidget(
    MaterialApp(
      theme: wispTheme(),
      home: Builder(
        builder: (context) => Scaffold(
          body: TextButton(
            onPressed: () => showWispSheet<String>(
              context,
              title: 'Page 1',
              child: content ?? _FlowContent(0, completion: completion),
            ),
            child: const Text('Open popup'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open popup'));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a short sheet keeps its content height', (tester) async {
    await mountFlow(tester);
    expect(tester.getSize(find.byType(WispPanel).last).height, lessThan(400));
    expect(find.byTooltip('Close').hitTestable(), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  for (final width in [360.0, 1000.0]) {
    testWidgets(
      'long sheets keep Back and Close visible after scrolling at ${width.toInt()}px',
      (tester) async {
        await mountFlow(
          tester,
          width: width,
          content: const _LongFlowContent(),
        );
        final rootClose = tester.getRect(find.byTooltip('Close').last);
        await tester.drag(
          find.byType(SingleChildScrollView).last,
          const Offset(0, -1200),
        );
        await tester.pumpAndSettle();
        expect(tester.getRect(find.byTooltip('Close').last), rootClose);
        expect(find.byTooltip('Close').hitTestable(), findsOneWidget);
        await tester.ensureVisible(find.text('Open long child'));
        await tester.tap(find.text('Open long child'));
        await tester.pumpAndSettle();
        final back = tester.getRect(find.byTooltip('Back').last);
        final close = tester.getRect(find.byTooltip('Close').last);
        await tester.drag(
          find.byType(SingleChildScrollView).last,
          const Offset(0, -1200),
        );
        await tester.pumpAndSettle();
        expect(tester.getRect(find.byTooltip('Back').last), back);
        expect(tester.getRect(find.byTooltip('Close').last), close);
        expect(find.byTooltip('Back').hitTestable(), findsOneWidget);
        expect(find.byTooltip('Close').hitTestable(), findsOneWidget);
        await tester.tap(find.byTooltip('Back').last);
        await tester.pumpAndSettle();
        expect(find.text('Long child page'), findsNothing);
        await tester.tap(find.text('Open long child'));
        await tester.pumpAndSettle();
        await tester.drag(
          find.byType(SingleChildScrollView).last,
          const Offset(0, -1200),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Close').last);
        await tester.pumpAndSettle();
        expect(find.byTooltip('Close'), findsNothing);
        expect(find.text('Open popup'), findsOneWidget);
        expect(tester.takeException(), isNull);
      },
    );

    testWidgets(
      'Back returns one page and Close dismisses the whole flow at ${width.toInt()}px',
      (tester) async {
        await mountFlow(tester, width: width);
        expect(find.text('Page 1'), findsOneWidget);
        expect(find.byTooltip('Back'), findsNothing);
        await tester.enterText(
          find.byKey(const ValueKey('draft-0')),
          'Root draft',
        );
        await tester.tap(find.byKey(const ValueKey('next-0')));
        await tester.pumpAndSettle();
        expect(find.text('Page 2'), findsOneWidget);
        expect(find.byTooltip('Back'), findsOneWidget);
        await tester.enterText(
          find.byKey(const ValueKey('draft-1')),
          'Child draft',
        );
        await tester.tap(find.byKey(const ValueKey('next-1')));
        await tester.pumpAndSettle();
        expect(find.text('Page 3'), findsOneWidget);
        await tester.tap(find.byTooltip('Back').last);
        await tester.pumpAndSettle();
        expect(find.text('Page 2'), findsOneWidget);
        expect(find.text('Child draft'), findsOneWidget);
        await tester.binding.handlePopRoute();
        await tester.pumpAndSettle();
        expect(find.text('Page 1'), findsOneWidget);
        expect(find.text('Root draft'), findsOneWidget);
        expect(find.byTooltip('Back'), findsNothing);
        await tester.tap(find.byKey(const ValueKey('next-0')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const ValueKey('next-1')));
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Close').last);
        await tester.pumpAndSettle();
        expect(find.byTooltip('Close'), findsNothing);
        expect(find.byType(_FlowContent), findsNothing);
        await tester.tap(find.text('Open popup'));
        await tester.pumpAndSettle();
        expect(find.text('Page 1'), findsOneWidget);
        expect(find.byTooltip('Back'), findsNothing);
        expect(find.text('Root draft'), findsNothing);
        expect(tester.takeException(), isNull);
      },
    );
  }

  testWidgets('Save and Cancel still return only to the previous page', (
    tester,
  ) async {
    await mountFlow(tester);
    await tester.tap(find.byKey(const ValueKey('next-0')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(find.text('Page 1'), findsOneWidget);
    expect(find.text('Result: saved'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('next-0')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(find.text('Page 1'), findsOneWidget);
    expect(find.byTooltip('Close'), findsOneWidget);
    expect(find.text('Result: saved'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('an async completion cannot pop underneath a closing popup', (
    tester,
  ) async {
    final completion = Completer<void>();
    await mountFlow(tester, completion: completion);
    await tester.tap(find.byKey(const ValueKey('next-0')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Deferred save').last);
    await tester.tap(find.byTooltip('Close').last);
    completion.complete();
    await tester.pumpAndSettle();
    expect(find.byType(_FlowContent), findsNothing);
    expect(find.text('Open popup'), findsOneWidget);
    await tester.tap(find.text('Open popup'));
    await tester.pumpAndSettle();
    expect(find.text('Page 1'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('Back respects a busy page while Close can dismiss the flow', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: wispTheme(),
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => showWispSheet<void>(
                context,
                title: 'Parent page',
                child: Builder(
                  builder: (context) => TextButton(
                    onPressed: () => showWispSheet<void>(
                      context,
                      title: 'Busy page',
                      child: const PopScope(
                        canPop: false,
                        child: Text('Work in progress'),
                      ),
                    ),
                    child: const Text('Open busy page'),
                  ),
                ),
              ),
              child: const Text('Open popup'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open popup'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open busy page'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Back').last);
    await tester.pumpAndSettle();
    expect(find.text('Busy page'), findsOneWidget);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.text('Busy page'), findsOneWidget);
    await tester.tap(find.byTooltip('Close').last);
    await tester.pumpAndSettle();
    expect(find.text('Busy page'), findsNothing);
    expect(find.text('Parent page'), findsNothing);
    expect(find.byTooltip('Close'), findsNothing);
    expect(tester.takeException(), isNull);
  });
}
