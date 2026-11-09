# Session continuity roadmap

The fork includes a first implementation of structured handoff, verification
history, context inspection, language server support, timeline navigation,
selective compaction, and patch guards. See
[Session continuity](session-continuity-implementation.md) for usage and current
limits.

## Pin session constraints

Let users pin a small set of constraints, such as which branch to use or which
files to leave alone. Show them in `/context` and carry them through compaction
and handoff. Give users a way to edit or remove each constraint and set a size
limit so pins have a predictable cost.

## Detect edits made after a file was read

Track the version returned by supported file reads and compare it with the
version on disk before writing a patch. When they differ, report the affected
path and ask the agent to reread it before regenerating the patch. Keep the
existing check that covers changes made while a patch waits for approval.

Evaluate line anchors based on content hashes if whole-file version checks
produce too many conflicts on unrelated edits. Compare both approaches on the
models and editing tasks used by this fork.

## Unify timeline navigation

Bring fork ancestry, linked handoffs, and backtracking into one view. Preserve
the original transcript and keep existing resume and fork behavior. Make clear
which action restores a conversation and which, if available, restores files.

## Reuse language server processes

Reuse a configured server across requests in the same workspace. Define when
the process stops or restarts, and keep the existing permission boundaries.
Track document versions across requests so diagnostics and navigation results
refer to the correct file contents.

## Expand verification history

Recognize more test, lint, and build commands, including invocations from
workspace subdirectories. Keep runtime outcomes and workspace freshness
separate, and show unknown freshness when the workspace cannot be compared.
For expensive builds, consider checking available disk space, dependencies, and
active processes before execution.

## Related designs

- [Pi compaction and branch summaries](https://pi.dev/docs/latest/compaction)
- [Pi sessions](https://pi.dev/docs/latest/sessions) and
  [extension lifecycle](https://pi.dev/docs/latest/extensions)
- [oh-my-pi](https://github.com/can1357/oh-my-pi) and its
  [language server configuration](https://github.com/can1357/oh-my-pi/blob/main/docs/lsp-config.md)
- Claude Code [checkpointing](https://code.claude.com/docs/en/checkpointing),
  [hooks](https://code.claude.com/docs/en/hooks-guide), and
  [usage tracking](https://code.claude.com/docs/en/costs)
