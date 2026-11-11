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

Alternatively start the rebuilt CLI with `codex --enable clm`. The default is
off. Local launches with CLM enabled use this build's embedded server, even when
automatic daemon startup is enabled. The downloaded upstream daemon does not
contain this fork's CLM implementation. Explicit server connections are preserved;
that server must itself support CLM. `--no-daemon` can also force embedded mode.
Start or resume a session with the setting enabled; no provider-specific
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

### Tool discovery with code mode

In code-mode-only sessions, `context_notes`, `context_history` and
`get_context_remaining` are nested tools available through `functions.exec`.
Their absence from the direct tool list does not mean CLM is disabled. Inspect
`ALL_TOOLS` inside `functions.exec` and call the corresponding `tools` methods:

```javascript
text(ALL_TOOLS.filter(tool =>
  ["context_notes", "context_history", "get_context_remaining"].includes(tool.name)
));
text(await tools.get_context_remaining({}));
await tools.context_notes({text: "CLM smoke test: marker 7391"});
text(await tools.context_notes({}));
```

Notebook and history calls return JavaScript objects in code mode; for example,
`(await tools.context_notes({})).text` reads the notebook contents. Direct tool
responses retain their JSON text format.

This replaces the notebook, so use it in a disposable test thread.
`new_context` remains a direct tool; the smoke test does not call it.
If the nested catalog also lacks these tools, check the configuration and the
server executable handling the session. `features list` checks the local CLI's
configuration; it does not establish which features a separate app-server
process supports. A stock server cannot expose tools added only in this fork.

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

## Why startup and code-mode handling were improved

Enabling a feature in the CLI's configuration does not add that feature to a
different server executable. The managed daemon is downloaded from upstream,
where these CLM tools do not exist. Its compatibility check compares explicit
CLI feature overrides, but file settings belong to the daemon until restart.
Consequently, `clm = true` in the fork's configuration could coexist with a
session served by a binary without CLM. Local implicit launches now select the
embedded server from the same build when the effective configuration enables
CLM. This avoids relying on a daemon restart to install fork-specific code.
Explicit server selections remain the caller's responsibility, and disabling
CLM preserves the existing startup policy.

Tool visibility also depends on the active tool mode. In code-mode-only sessions,
the notebook, history and budget tools belong to the nested JavaScript catalog,
while `new_context` stays direct. The model guidance now explains this route so
the model can distinguish a nested tool from a missing tool.

The integration investigation exposed a separate result-format defect:
`context_notes` and `context_history` returned JSON strings to JavaScript, making
expressions such as `notes.text` return `undefined`. They now return structured
objects through the existing `ToolOutput::code_mode_result` boundary. Notebook
results also declare their output schema for generated tool documentation.
Direct responses keep their JSON text representation, and persistence,
authorization checks and thread-scoped history access retain their existing
behavior.

## Validation

### Startup and code-mode fixes

Validated on 2026-11-11 with both CLM integration tests and all seven daemon
startup tests. The new code-mode regression reproduced the notebook result
defect before the fix and passed afterward. The startup regression loads CLM
from `config.toml` and checks enabled/disabled settings, implicit/explicit
connections and local/remote targets. A rebuilt CLI and an interactive TUI
were also exercised against a local simulated provider with CLM and automatic
daemon startup enabled: tool discovery, notebook marker round-trip, token budget
and history search succeeded without calling `new_context`. Formatting and
`git diff --check` passed. These checks do not constitute a full workspace test
run or a live-provider end-to-end test.

### Earlier validation

Validated on 2026-11-09 with 12 focused core tests (notebook provenance, limits,
clearing, safe tool units, checkpoint replay and MCP attribution), all 47 feature
tests, and an integration test using a simulated model. The integration test
saves notes, requests rollover without a summarization call, and successfully
searches the original transcript afterward. `cargo check -p codex-core --tests`,
CLI build, feature-flag/config smoke checks, schema generation and
`git diff --check` passed. The complete workspace test suite was not run.
