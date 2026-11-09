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

## Automatic compaction and handoff

Codex automatically compacts the conversation when it reaches the model's token
threshold. No command or opt-in is required. By default the threshold comes from
the model and is capped at 90% of its context window. To compact earlier, set a
positive token limit at the top level of `config.toml`, for example:

```toml
model_auto_compact_token_limit = 100000
```

The model's context-window cap still applies. Compaction keeps you in the same
chat and lets work continue with summarized context. `/compact` requests it
immediately; automatic compaction does not require periodically running that
command.

In the terminal UI, `/handoff` summarizes the current conversation and starts a
new chat with that summary as its first message. Run it after the current task
finishes. The summary covers the objective, constraints, decisions, completed
work and next steps from the visible user and assistant exchanges. Long histories
are bounded, preserving the beginning and end; raw tool output and attachments
are not copied. The new assistant is instructed to verify the workspace before
continuing. The original chat remains available through `/resume`.

Handoff uses the same new-session settings as `/new`. If summary generation fails,
the current chat stays open. If you switch chats, send another turn, or leave new
input in the composer while it is being generated, no automatic transfer occurs.
Handoff is currently unavailable for cloud conversations, like `/recap`.

## Session continuity extensions

This development build adds `/checks`, `/context`, `/timeline`, `/compact N`,
durable structured `/handoff`, and explicitly configured language servers.
See [session continuity implementation](session-continuity-implementation.md)
for configuration examples, persistence behavior and current limits.
