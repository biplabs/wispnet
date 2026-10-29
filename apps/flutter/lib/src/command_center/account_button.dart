part of 'command_center.dart';

/// A secondary desktop settings entry. Status reads require no account.
class CommandCenterAccountButton extends StatelessWidget {
  const CommandCenterAccountButton({
    super.key,
    required this.backend,
    required this.onChanged,
    this.enabled = commandCenterEnabled,
  });

  final WispnetBackend backend;
  final VoidCallback onChanged;
  final bool enabled;

  @override
  Widget build(BuildContext context) => enabled
      ? _CommandCenterAccountButton(
          key: ObjectKey(backend),
          backend: backend,
          onChanged: onChanged,
        )
      : const SizedBox.shrink();
}

class _CommandCenterAccountButton extends StatefulWidget {
  const _CommandCenterAccountButton({
    super.key,
    required this.backend,
    required this.onChanged,
  });

  final WispnetBackend backend;
  final VoidCallback onChanged;

  @override
  State<_CommandCenterAccountButton> createState() =>
      _CommandCenterAccountButtonState();
}

class _CommandCenterAccountButtonState
    extends State<_CommandCenterAccountButton>
    with WidgetsBindingObserver {
  Map<String, dynamic>? status;
  bool unavailable = false;
  bool reading = false;
  bool open = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(refresh());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) unawaited(refresh());
  }

  Future<void> refresh() async {
    if (reading) return;
    reading = true;
    try {
      final next = await widget.backend.invoke('desktopRequest', {
        'payload': {'op': 'command_center_status'},
      });
      if (next['enabled'] != true || next['linked'] is! bool) {
        throw const FormatException('Public account controls are unavailable.');
      }
      if (mounted) {
        setState(() {
          status = next;
          unavailable = false;
        });
      }
    } catch (_) {
      if (mounted) setState(() => unavailable = true);
    } finally {
      reading = false;
    }
  }

  Future<void> showAccount() async {
    if (open) return;
    setState(() => open = true);
    try {
      await showWispSheet<void>(
        context,
        title: 'Public account',
        child: CommandCenterModule(
          enabled: true,
          backend: widget.backend,
          // Account-only controls do not depend on the service registry.
          hostStatus: const {},
          onChanged: () {
            widget.onChanged();
            unawaited(refresh());
          },
        ),
      );
    } finally {
      if (mounted) {
        setState(() => open = false);
        await refresh();
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final linked = status?['linked'] == true;
    final account = status?['account'] as Map?;
    final name = account?['name'] as String?;
    final label = unavailable
        ? 'Public account'
        : linked
        ? 'Logged in as ${name?.trim().isNotEmpty == true ? name! : 'GitHub user'}'
        : status == null
        ? 'Public account'
        : 'Sign in with GitHub';
    return Tooltip(
      message: unavailable
          ? 'Public account unavailable. Open to retry.'
          : linked
          ? 'Public account · $label'
          : 'Sign in with GitHub for public publishing',
      child: TextButton.icon(
        key: const ValueKey('public-account'),
        onPressed: open ? null : showAccount,
        icon: Icon(
          unavailable
              ? Icons.warning_amber_rounded
              : linked
              ? Icons.account_circle_outlined
              : Icons.login,
          size: 18,
        ),
        label: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis),
      ),
    );
  }
}
