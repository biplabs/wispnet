# Native command execution

An authorized Shell service stream accepts either the existing interactive
`Attach` request or a one-shot `Exec` request as its first frame. Both use the
same service incarnation and peer grant. Execution does not create or attach an
interactive terminal session.

On Unix desktop hosts the command is passed, unchanged, to `/bin/sh -c` under the
account running Wispnet. Standard input is closed. The optional working directory
must be absolute; omission uses the account's HOME when available. Environment
variables are inherited from the host service. Shell quoting and multiline text
are interpreted by `/bin/sh`, independently of the interactive account shell.

Requests contain `command`, `cwd`, `timeout_ms`, and `max_output_bytes`. Limits are
8192 command bytes, 1–300000 milliseconds, and 1–1048576 captured bytes. Suggested
client defaults are 30000 milliseconds and 65536 bytes. A host allows at most 16
concurrent native executions. Captured output is bounded across stdout and stderr
combined; excess output is drained and the result is marked truncated.

`Stdout` and `Stderr` are raw binary frames with tags 4 and 5. `ExecFinished`
contains the exit code, `timed_out`, and `output_truncated`. Errors in spawning or
request validation produce `Error`. Existing interactive frame tags are unchanged.

Keep the transport's send direction open while receiving results. Transport EOF,
additional client input, service revocation, or cancellation ends the execution.
Each command owns a separate Unix process group. Timeout and cancellation kill
that group and reap its main child. Background descendants in the group are also
terminated when the main command exits, so inherited stdout/stderr pipes cannot
keep a completed command alive. A process that deliberately creates a separate session can escape group cleanup;
the overall deadline still closes inherited capture pipes and finishes the
request with timeout/truncation flags. This is process cleanup, not an OS sandbox.
