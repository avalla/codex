# Configuration

For basic configuration instructions, see [this documentation](https://developers.openai.com/codex/config-basic).

For advanced configuration instructions, see [this documentation](https://developers.openai.com/codex/config-advanced).

For a full configuration reference, see [this documentation](https://developers.openai.com/codex/config-reference).

## Lifecycle hooks

Admins can set top-level `allow_managed_hooks_only = true` in
`requirements.toml` to ignore user, project, and session hook configs while
still allowing managed hooks from requirements and managed config layers. This
setting is only supported in `requirements.toml`; putting it in `config.toml`
does not enable managed-hooks-only mode.

## Automatic compaction

Codex automatically compacts the conversation when it reaches the model's token
threshold. No command or opt-in is required. By default the threshold comes from
the model and is capped at 90% of its context window. To compact earlier, set a
positive token limit at the top level of `config.toml`, for example:

```toml
model_auto_compact_token_limit = 100000
```

Compaction summarizes context within the current chat. You can request it
immediately with `/compact`. See [Slash commands](slash_commands.md#session-continuity-in-this-fork)
for selective compaction and handoff in this fork.

## Language servers in this fork

Install the language server separately, then add a named entry to `config.toml`.
Set `command` to the executable and its arguments, `language_id` to the
identifier expected by the server, and `extensions` to the file extensions it
handles. For example:

```toml
[language_servers.rust]
command = ["rust-analyzer"]
language_id = "rust"
extensions = ["rs"]
diagnostics_on_edit = true
```

Set `diagnostics_on_edit` to `true` to request diagnostics after successful
`apply_patch` edits to matching files. The setting defaults to `false`.
See [Session continuity](session-continuity-implementation.md#language-server-requests)
for supported requests and limits.

## Model-managed context in this fork

Set `[features] clm = true` to enable local working notes, model-requested context
windows, and retrieval of this thread's persisted transcript. See
[CLM mode](clm.md) for configuration, tool behavior, persistence and fallback rules.
