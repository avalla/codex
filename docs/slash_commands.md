# Slash commands

For an overview of Codex CLI slash commands, see [this documentation](https://developers.openai.com/codex/cli/slash-commands).

## Session continuity in this fork

| Command      | What it does                                                                                                                                      |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/handoff`   | Starts a new chat with a summary of the current chat and links the two.                                                                           |
| `/checks`    | Shows recorded check results and whether they still match the current workspace.                                                                  |
| `/context`   | Shows context usage as of the last completed turn.                                                                                                |
| `/timeline`  | Shows fork ancestry and handoff links, and lets you open a related chat.                                                                          |
| `/compact N` | Summarizes older history in the current chat and keeps the last `N` complete user turns, including their tool history. Use a value from 1 to 100. |

Run `/handoff` after the current turn finishes. The original chat remains
available through `/resume`. If you change chats, start another turn, or add a
draft while the summary is being generated, Codex cancels the automatic transfer.
Cloud conversations do not support `/handoff`.

Use `/compact` without an argument to compact using the default policy.
See [Configuration](config.md#automatic-compaction) for automatic compaction
and [Session continuity](session-continuity-implementation.md) for details on
what the commands preserve.
