# Session continuity

This fork extends the Codex terminal UI with session handoff, verification
history, context inspection, timeline navigation, and selective compaction.
See [Slash commands](slash_commands.md#session-continuity-in-this-fork) for
usage and the [roadmap](session-continuity-roadmap.md) for planned work.

## Language server requests

See [Configuration](config.md#language-servers-in-this-fork) to register a
language server.

When configured, the `language_server` tool exposes diagnostics, definition,
references and semantic rename previews. Positions use zero-based UTF-16 units.
Rename returns a WorkspaceEdit preview; applying changes uses the normal patch
approval path. Automatic diagnostics run after a successful `apply_patch` batch,
for at most ten matching paths and a shared 20-second deadline. Server failures
are reported separately and do not change a successful patch into a failure.

The transport uses the selected environment's executor, filesystem sandbox and
network controls. Each query opens an immutable version-1 document snapshot,
has a 20-second timeout, and terminates its server process on completion or
cancellation. Changed source documents and mismatched diagnostic versions are
rejected. Server-initiated edits are denied. Frame size is limited to 4 MiB and
documents to 2 MiB. The protocol follows
[LSP 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification).

## How session data is preserved

- `/handoff` uses the same new-session settings as `/new`. The new chat starts
  with a summary of the objective, constraints, decisions, completed work, and
  next steps. The summary uses a bounded transcript of visible user and
  assistant messages and asks the new assistant to verify the workspace before
  continuing.
- Thread attachments are the canonical persistence store, accessible through
  existing app-server APIs. If saving the destination link fails, the source
  record stays available in `/timeline` and the composer recovers the continuation
  text.
- Handoff stores the model summary separately from runtime evidence and omits
  raw command output and reasoning.
  Command arguments are persisted in the ledger, so avoid secrets in arguments.
  A handoff carries up to five recent check records into the new conversation.
- The ledger retains at most 40 records per thread and recognizes simple test,
  lint and check invocations. Compound shell expressions and arbitrary wrappers
  are not inferred. Missing evidence is unknown, never success. Timeout and
  interruption cannot produce successful records.
- Freshness compares Git-visible content before/after execution and at display,
  including dirty and untracked files. It excludes ignored files, dependencies,
  services and environment variables. It is not an atomic filesystem snapshot.
  Capture is bounded to three seconds, 10,000 files and 100 MiB. Remote sessions,
  subdirectory execution, restricted read roots, symlinks and unsupported layouts
  report unknown freshness rather than a partial match.
- `/context` is a completed-turn snapshot. Component bytes are not token counts.
  Tool/MCP definition sizes are available when declarations are recorded in
  history; otherwise explicitly unavailable. User-pinned context is deferred.
- `/timeline` navigates parents and linked handoffs, not every descendant in the
  repository. Existing backtracking remains available separately. Selective
  compaction uses the local summary request path, rejects pending mail and an
  oversized retained context, and preserves original durable conversation events.
- LSP processes currently start per query. There is no persistent server pool,
  automatic server discovery or automatic application of semantic rename edits.
  Only the queried document is checked for changes; cross-file results remain
  previews. Shell edits do not trigger automatic diagnostics.
- Patch guards compare exact source and destination contents between patch
  verification and execution, including approval delays. This does not track
  arbitrary earlier shell reads and is not an atomic compare-and-write operation.
  A conflict requires rereading and regenerating the patch. Hashline editing is
  deferred; existing patch syntax stays compatible.

## Validation

Focused checks ran on November 8–9, 2026.

- `cargo check -p codex-tui --tests` passed before the final edits. The focused
  TUI tests and CLI build below ran after those edits.
- Core compaction module: 25 passed, including two selective-compaction tests
  and a mocked model request asserting the preserved suffix is not summarized.
- Shared continuity schema/freshness tests: 3 passed.
- Git fingerprint test: passed (dirty, untracked and deleted files).
- Apply-patch library suite: 69 passed. The first run exposed a regression for
  binary destinations; version snapshots now compare bytes and preserve that
  existing behavior.
- Core patch runtime/handler regressions: 27 passed.
- LSP transport: 3 passed (framing, stale diagnostics, denied server edits and
  process termination).
- SQLite evidence/context persistence: 1 passed.
- Thread attachment lifecycle: 7 passed, including persistence before the first
  user turn. Legacy JSONL can precede its SQLite projection; adding an attachment
  now indexes the durable creation record under the existing lifecycle lock.
- New TUI command routing/argument validation: 2 passed; temporary structured
  requests: 8 passed.
- Recap and handoff: 65 TUI tests passed, including source and destination
  attachment persistence.
- App-server, Python SDK and configuration schemas were regenerated. The final
  diff was checked for the compaction parameter and language-server settings.
- The shared continuity tests passed again on 2026-11-09 after the shell-command
  detection change.
- `cargo build --locked -p codex-cli --bin codex` passed. The resulting binary
  responded to `--version` and `--help`.
