# Local context language model mode

This fork implements an opt-in, notes-backed context mode inspired by
[OMP's experimental context windows](https://github.com/can1357/oh-my-pi/blob/main/docs/compaction.md)
and the [Context Language Models research](https://github.com/facebookresearch/context-language-models).
It uses the current coding model to maintain working notes and request a smaller
context window. It does not install, train, or serve a separate model.

## Enable

Add to `~/.codex/config.toml` (or your existing `[features]` table):

```toml
[features]
clm = true
```

Alternatively start the rebuilt CLI with `codex --no-daemon --enable clm`. The default is
off. Start or resume a session with the setting enabled; no provider-specific
notes API or extra dependency is required. Local rollover requires the normal
SQLite-backed thread store and persisted, searchable history. Restricted internal
threads do not receive CLM tools.
New local threads use the paginated format. Resumed legacy threads keep ordinary
compaction; `new_context` rejects CLM rollover there because searchable recovery
is unavailable. Start a new thread to use CLM windows with an old conversation.

## Model controls

| Tool | Behavior |
| --- | --- |
| `context_notes` | Read with `{}`; replace with `{"text":"..."}`; clear with `{"text":""}`. Maximum 16,384 UTF-8 bytes. Oversized writes leave the previous revision unchanged. |
| `new_context` | Request rollover after the current model response and its tool calls finish. Save non-empty notes first. |
| `get_context_remaining` | Inspect remaining context tokens when the active model has a known window. |
| `context_history` | Search messages with `query`, or page through persisted transcript items and tool outputs with `cursor`. Use `turn_id` and `item_id` for exact lookup. Large items can be read in 8,000-byte slices with `offset_bytes` / `next_offset_bytes`. |

The model receives guidance to preserve the task, constraints, decisions, changed
files, verification evidence, unresolved work and next actions. At 75% of the
effective context budget it receives one reminder per window to update its notes.
The quality and freshness of the notebook remain the model's responsibility.

## Window boundaries and persistence

Automatic maintenance and plain `/compact` try a local rollover first. It keeps
the latest notebook, original user authorization messages, client-authored
developer messages, explicitly retained client outputs, and a recent suffix with
a target of 4,000 estimated tokens. Tool calls and results are kept or dropped as
complete pairs. Pinned messages may increase the retained size. Pending mailbox
input, incomplete tool calls, insufficient savings, oversized retained context,
missing notes or unavailable searchable storage use ordinary compaction instead.
`/compact N` retains its existing selective summarization behavior.

Rollover uses the existing compaction hooks, context-window identities, checkpoint
records, world-state reconstruction, and retained authorization evidence. It
flushes the original transcript before replacing live context. It does not make a
summarization request when local rollover succeeds. Speculative server compaction
is disabled while CLM is enabled.

Notebook revisions are host-annotated tool data in the session journal. Only the
latest revision is sent as a notebook in model requests. The exact latest revision
is also retained through ordinary compaction, including when CLM is disabled.
Resume and full-history forks restore the notebook from that history; fresh
threads start without one. Clearing notes does not restore an older revision.
Notes cannot grant user authorization or change system/developer instructions.

History reads are scoped to the calling thread and use the thread store's visible
branch. They never accept a different thread ID or search another agent as a
fallback. Search returns at most five literal message matches; transcript reads
return one item at a time. Reads time out after ten seconds. Recovery is limited
to what the journal and transcript projection retained: images are not rebuilt,
and earlier destructive history operations or previously truncated tool outputs
cannot be undone.

This is an independent implementation of notes-backed context windows. It does
not expose arbitrary edits to system prompts, raw provider messages, or other
agents' context files, and does not implement the research project's training or
suffix-cache serving changes.

## Validation

Validated on 2026-11-09 with 12 focused core tests (notebook provenance, limits,
clearing, safe tool units, checkpoint replay and MCP attribution), all 47 feature
tests, and an integration test using a simulated model. The integration test
saves notes, requests rollover without a summarization call, and successfully
searches the original transcript afterward. `cargo check -p codex-core --tests`,
CLI build, feature-flag/config smoke checks, schema generation and
`git diff --check` passed. The complete workspace test suite was not run.
