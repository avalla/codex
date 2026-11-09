# Session continuity implementation

Status: first implementation of the six roadmap areas. Remaining work is listed
under deliberate limits.

## Usage

| Entry point | Behavior |
| --- | --- |
| `/handoff` | Saves versioned structured model state and up to five recent runtime check records, then links and starts a new conversation. |
| `/checks` | Shows real command outcomes and conservative workspace freshness, including evidence from the immediate handoff source. |
| `/context` | Shows the last completed turn's runtime token accounting and serialized component sizes in bytes. |
| `/timeline` | Shows source/fork relationships and saved handoff summaries, with a picker for related conversations. |
| `/compact 2` | Summarizes older history and preserves the last two user turns, including their tool history. Accepts 1–100. |
| `/compact` | Uses the original compaction path unchanged. Automatic compaction still uses the original configured policy. |

### Configured language servers

Install the language server separately and add an executable argument vector to
`config.toml`; Codex does not install or download servers automatically:

```toml
[language_servers.rust]
command = ["rust-analyzer"]
language_id = "rust"
extensions = ["rs"]
diagnostics_on_edit = true
```

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

## Deliberate limits of this implementation

- Thread attachments are the canonical persistence store, accessible through
  existing app-server APIs. The TUI initiates handoff; no separate database was
  added. Failed destination-link writes preserve the saved source record and
  restore the continuation text to the composer. `/timeline` can recover the
  saved summary on the source conversation. No filesystem rollback occurs.
- Handoff input still uses bounded visible conversation text. Model claims are
  separated from runtime evidence; no raw command output or reasoning is copied.
  Command arguments are persisted in the ledger, so avoid secrets in arguments.
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

These limits distinguish the delivered first implementation from the broader
roadmap: persistent LSP sessions, integrated backtracking, pinned context and a
general read-version ledger remain future work.

## Validation log

Validated on 2026-11-08 with incremental compilation and debug information
disabled to limit disk usage. Full workspace test coverage is not claimed.

- Initial `cargo check -p codex-tui --tests`: passed; final edits still require
  the subsequent test/build pass.
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

## Boundaries and invariants

- Reuse thread-store attachments for versioned handoff records and source/destination links; do not create a competing persistence database.
- Treat model summaries as claims. Command outcomes must originate in runtime events, and must never imply that the current workspace was tested without matching evidence.
- Reuse runtime context accounting. Mark estimates and unavailable categories explicitly.
- Preserve existing fork/resume and compaction behavior, original transcripts, drafts and queued inputs.
- LSP processes must use configured executables and existing permission boundaries. Diagnostics do not replace tests. Discard stale document results.
- Revalidate patch inputs after approval. A stale file must cause an actionable conflict, not a silent overwrite.

## Work sequence

1. Versioned structured handoff records, shared schema, durable links and recovery.
2. Runtime verification evidence and conservative freshness reporting.
3. Context inspection using actual accounting and clearly labeled estimates.
4. Configurable LSP integration and versioned diagnostics/navigation.
5. Branch navigation and selective summarization preserving active work.
6. File-version guards for edits, including approval delays and rename destinations.
7. Focused tests, formatting, architecture review, CLI rebuild and documentation.

Existing `/handoff` changes are the baseline and must be preserved. Implement each
piece at its owning layer; any narrower first implementation must be documented
explicitly rather than marked as the complete roadmap item.
