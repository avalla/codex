//! Read-only continuity views backed by the app server's thread store.

use super::App;
use crate::app_event::AppEvent;
use crate::app_server_session::AppServerSession;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionViewParams;
use crate::slash_command::SlashCommand;
use codex_protocol::ThreadId;
use codex_protocol::continuity::*;

impl App {
    pub(super) async fn show_continuity(
        &mut self,
        server: &mut AppServerSession,
        thread_id: ThreadId,
        view: SlashCommand,
    ) {
        let result = self.continuity_report(server, thread_id, view).await;
        match result {
            Ok(report) => self.chat_widget.add_info_message(report, None),
            Err(error) => self
                .chat_widget
                .add_error_message(format!("Could not read continuity data: {error}")),
        }
    }

    async fn continuity_report(
        &mut self,
        server: &mut AppServerSession,
        thread_id: ThreadId,
        view: SlashCommand,
    ) -> color_eyre::Result<String> {
        let attachments = server.continuity_attachments(thread_id).await?;
        match view {
            SlashCommand::Context => {
                let report = attachments
                    .iter()
                    .filter(|a| a.attachment_type == CONTEXT_ATTACHMENT_TYPE)
                    .filter_map(|a| {
                        serde_json::from_value::<ContextInspection>(a.payload.clone()).ok()
                    })
                    .filter(|r| r.schema_version == 1)
                    .max_by_key(|r| r.captured_at_ms);
                let Some(report) = report else {
                    return Ok("Context inspection is available after a turn completes in this build. No measured snapshot is available yet.".into());
                };
                Ok(render_context(&report))
            }
            SlashCommand::Checks => {
                let mut records = attachments
                    .iter()
                    .filter(|a| a.attachment_type == VERIFICATION_ATTACHMENT_TYPE)
                    .filter_map(|a| {
                        serde_json::from_value::<VerificationRecord>(a.payload.clone()).ok()
                    })
                    .filter(|r| r.schema_version == 1)
                    .collect::<Vec<_>>();
                let mut sources = Vec::new();
                for attachment in &attachments {
                    if attachment.attachment_type == HANDOFF_ATTACHMENT_TYPE
                        && let Some(source) = attachment
                            .payload
                            .get("source_thread_id")
                            .and_then(|v| v.as_str())
                        && source != thread_id.to_string()
                        && !sources.iter().any(|s: &String| s == source)
                    {
                        sources.push(source.to_string());
                    }
                }
                for source in sources {
                    if let Ok(source) = ThreadId::from_string(&source) {
                        records.extend(
                            server
                                .continuity_attachments(source)
                                .await?
                                .into_iter()
                                .filter(|a| a.attachment_type == VERIFICATION_ATTACHMENT_TYPE)
                                .filter_map(|a| {
                                    serde_json::from_value::<VerificationRecord>(a.payload).ok()
                                })
                                .filter(|r| r.schema_version == 1),
                        );
                    }
                }
                if records.is_empty() {
                    return Ok("No runtime check evidence recorded. Checks from older builds, unsupported command forms, or interrupted executions may have no record; this does not mean they passed.".into());
                }
                records.sort_by_key(|r| r.completed_at_ms);
                let cwd =
                    codex_utils_path_uri::PathUri::from_abs_path(&self.config.cwd).to_string();
                // Bound inspection to one current workspace, never all historical paths.
                let fingerprint = if server.uses_remote_workspace()
                    || !self
                        .config
                        .permissions
                        .file_system_sandbox_policy()
                        .has_full_disk_read_access()
                    || !records.iter().any(|record| {
                        record.cwd == cwd
                            && record.workspace_before.is_some()
                            && record.workspace_after.is_some()
                    }) {
                    None
                } else {
                    codex_git_utils::workspace_fingerprint(self.config.cwd.as_path()).await
                };
                let mut output = String::from(
                    "Runtime check evidence (up to 40 recent records per thread). File matching excludes ignored files, dependencies, environment and services.\n",
                );
                for record in records {
                    let current = if record.cwd == cwd {
                        fingerprint.as_deref()
                    } else {
                        None
                    };
                    output.push_str(&format!(
                        "\n{:?} (exit {}) — {}\n  {}\n  cwd: {} · completed: {} ms UTC\n",
                        record.outcome,
                        record.exit_code,
                        record.freshness(current),
                        record.command.join(" "),
                        record.cwd,
                        record.completed_at_ms
                    ));
                }
                Ok(output)
            }
            SlashCommand::Timeline => {
                let thread = server.thread_read(thread_id, false).await?;
                let mut destinations = std::collections::BTreeSet::new();
                let mut output = format!("Session {thread_id}\n");
                if let Some(parent) = thread.forked_from_id {
                    destinations.insert(parent.to_string());
                    output.push_str(&format!("Forked from {parent} — /resume {parent}\n"));
                }
                if let Some(parent) = thread.parent_thread_id {
                    destinations.insert(parent.to_string());
                    output.push_str(&format!("Parent {parent} — /resume {parent}\n"));
                }
                for attachment in attachments
                    .iter()
                    .filter(|a| a.attachment_type == HANDOFF_ATTACHMENT_TYPE)
                {
                    output.push_str(&format!("\nHandoff {}\n", attachment.identity_key));
                    for owner in server.handoff_owners(&attachment.identity_key).await? {
                        if owner != thread_id.to_string() {
                            destinations.insert(owner.to_string());
                        }
                        output.push_str(&format!("  {owner} — /resume {owner}\n"));
                    }
                    if let Ok(record) =
                        serde_json::from_value::<HandoffRecord>(attachment.payload.clone())
                        && record.validate().is_ok()
                    {
                        output.push_str(&format!("Saved model summary: {}\n", record.summary));
                    }
                }
                output.push_str("\n/resume switches conversations; /fork branches the current conversation. Neither restores workspace files.");
                if !destinations.is_empty() {
                    self.chat_widget.show_selection_view(SelectionViewParams {
                        title: Some("Related conversations".into()),
                        subtitle: Some(
                            "Select a conversation to resume; workspace files are unchanged."
                                .into(),
                        ),
                        items: destinations
                            .into_iter()
                            .map(|id| SelectionItem {
                                name: id.clone(),
                                actions: vec![Box::new(move |tx| {
                                    tx.send(AppEvent::ResumeSessionByIdOrName(id.clone()))
                                })],
                                dismiss_on_select: true,
                                ..Default::default()
                            })
                            .collect(),
                        ..SelectionViewParams::picker()
                    });
                }
                Ok(output)
            }
            _ => Ok(String::new()),
        }
    }
}

fn render_context(r: &ContextInspection) -> String {
    let number = |v: Option<i64>| v.map_or_else(|| "unavailable".to_string(), |v| v.to_string());
    format!(
        "Context snapshot at completed turn {} ({} ms UTC)\nRuntime active tokens: {} (may include local runtime estimates)\nAuto-compact scope: {} / {}\nEffective context window: {}\nRemaining before limit: {}\n\nSerialized context sizes (bytes, NOT token counts):\nInstructions: {}\nHistory and tool calls: {}\nTool outputs: {}\nTool/MCP definitions: {}\n\nComponent byte sizes do not sum to runtime tokens. This snapshot is not live during a running turn.",
        r.turn_id,
        r.captured_at_ms,
        r.active_context_tokens,
        r.auto_compact_scope_tokens,
        number(r.auto_compact_scope_limit),
        number(r.full_context_window_limit),
        number(r.remaining_tokens),
        r.serialized_instruction_bytes,
        r.serialized_history_bytes,
        r.serialized_tool_output_bytes,
        r.tool_definition_bytes.map_or_else(
            || "not exposed by this snapshot".to_string(),
            |v| v.to_string()
        )
    )
}
