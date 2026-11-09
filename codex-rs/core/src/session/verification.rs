//! Runtime-authored check evidence. No model output can mark a check successful.
//! Retain at most 40 recent records in the existing attachment store.

use super::session::Session;
use codex_protocol::continuity::*;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandStatus;
use codex_thread_store::AddThreadAttachmentParams;
use codex_thread_store::ListThreadAttachmentsParams;
use codex_thread_store::RemoveThreadAttachmentParams;
use std::collections::HashMap;
use tokio::sync::Mutex;

#[derive(Default)]
struct PendingChecks {
    checks: Mutex<HashMap<String, Option<String>>>,
    persistence: Mutex<()>,
}

impl Session {
    pub(super) async fn persist_context_inspection(&self, turn: &super::turn_context::TurnContext) {
        if !self.services.thread_store.supports_thread_attachments() {
            return;
        }
        let usage = super::context_window::context_window_token_status(self, turn).await;
        let history = self.clone_history().await;
        let base = self.get_base_instructions().await;
        let mut instructions = serde_json::to_vec(&base).map_or(0, |v| v.len());
        let mut messages = 0;
        let mut outputs = 0;
        let mut tools = None;
        for item in history.raw_items() {
            let Ok(value) = serde_json::to_value(item) else {
                continue;
            };
            let bytes = serde_json::to_vec(&value).map_or(0, |v| v.len());
            if matches!(
                item,
                codex_protocol::models::ResponseItem::AdditionalTools { .. }
            ) {
                *tools.get_or_insert(0) += bytes;
            } else if matches!(
                value.get("role").and_then(|v| v.as_str()),
                Some("system" | "developer")
            ) {
                instructions += bytes;
            } else if value
                .get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|kind| kind.ends_with("_output"))
            {
                outputs += bytes;
            } else {
                messages += bytes;
            }
        }
        let report = ContextInspection {
            schema_version: 1,
            turn_id: turn.sub_id.clone(),
            captured_at_ms: chrono::Utc::now().timestamp_millis(),
            active_context_tokens: usage.active_context_tokens,
            auto_compact_scope_tokens: usage.auto_compact_scope_tokens,
            auto_compact_scope_limit: usage.auto_compact_scope_limit,
            full_context_window_limit: usage.full_context_window_limit,
            remaining_tokens: usage.base_window_tokens_remaining,
            serialized_instruction_bytes: instructions,
            serialized_history_bytes: messages,
            serialized_tool_output_bytes: outputs,
            tool_definition_bytes: tools,
        };
        let result = async {
            self.services
                .thread_store
                .remove_thread_attachment(RemoveThreadAttachmentParams {
                    thread_id: self.thread_id,
                    attachment_type: CONTEXT_ATTACHMENT_TYPE.to_string(),
                    identity_key: "latest".to_string(),
                })
                .await?;
            self.services
                .thread_store
                .add_thread_attachment(AddThreadAttachmentParams {
                    thread_id: self.thread_id,
                    attachment_type: CONTEXT_ATTACHMENT_TYPE.to_string(),
                    identity_key: "latest".to_string(),
                    payload: serde_json::to_value(report)?,
                })
                .await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, "could not save context inspection");
        }
    }

    pub(super) async fn observe_verification(&self, event: &EventMsg) {
        if !matches!(
            event,
            EventMsg::ExecCommandBegin(_) | EventMsg::ExecCommandEnd(_)
        ) {
            return;
        }
        if !self.services.thread_store.supports_thread_attachments() {
            return;
        }
        let pending = self
            .services
            .thread_extension_data
            .get_or_init(PendingChecks::default);
        // Native Git/content capture must never bypass restricted read roots.
        let environments = self.services.turn_environments.snapshot_now();
        let local_only = environments.single_local_environment().is_some()
            && environments.turn_environments().all(|environment| {
                environment
                    .permission_profile()
                    .file_system_sandbox_policy()
                    .has_full_disk_read_access()
            });
        match event {
            EventMsg::ExecCommandBegin(begin) if is_verification_command(&begin.command) => {
                let before = match (local_only, begin.cwd.to_abs_path()) {
                    (true, Ok(path)) => {
                        codex_git_utils::workspace_fingerprint(path.as_path()).await
                    }
                    _ => None,
                };
                let mut pending = pending.checks.lock().await;
                if pending.len() < 128 {
                    pending.insert(begin.call_id.clone(), before);
                }
            }
            EventMsg::ExecCommandEnd(end) if is_verification_command(&end.command) => {
                let before = pending.checks.lock().await.remove(&end.call_id).flatten();
                let after = match (local_only, end.cwd.to_abs_path()) {
                    (true, Ok(path)) => {
                        codex_git_utils::workspace_fingerprint(path.as_path()).await
                    }
                    _ => None,
                };
                let record = VerificationRecord {
                    schema_version: 1,
                    call_id: end.call_id.clone(),
                    turn_id: end.turn_id.clone(),
                    command: end.command.clone(),
                    cwd: end.cwd.to_string(),
                    completed_at_ms: end.completed_at_ms,
                    exit_code: end.exit_code,
                    outcome: match end.status {
                        ExecCommandStatus::Completed if end.exit_code == 0 => {
                            VerificationOutcome::Succeeded
                        }
                        ExecCommandStatus::Declined => VerificationOutcome::NotRun,
                        _ => VerificationOutcome::Failed,
                    },
                    workspace_before: before,
                    workspace_after: after,
                };
                let _guard = pending.persistence.lock().await;
                if let Err(error) = self.persist_verification(&record).await {
                    tracing::warn!(%error, "could not persist verification evidence");
                }
            }
            _ => (),
        }
    }

    async fn persist_verification(&self, record: &VerificationRecord) -> anyhow::Result<()> {
        let store = &self.services.thread_store;
        // Attachment stores cap the total at 100; one page covers the bounded set.
        let page = store
            .list_thread_attachments(ListThreadAttachmentsParams {
                thread_id: self.thread_id,
                cursor: None,
                limit: 100,
            })
            .await?;
        let mut records: Vec<_> = page
            .attachments
            .into_iter()
            .filter(|a| a.attachment_type == VERIFICATION_ATTACHMENT_TYPE)
            .collect();
        records.sort_by_key(|a| a.created_at);
        for old in records.iter().take(records.len().saturating_sub(39)) {
            store
                .remove_thread_attachment(RemoveThreadAttachmentParams {
                    thread_id: self.thread_id,
                    attachment_type: old.attachment_type.clone(),
                    identity_key: old.identity_key.clone(),
                })
                .await?;
        }
        store
            .add_thread_attachment(AddThreadAttachmentParams {
                thread_id: self.thread_id,
                attachment_type: VERIFICATION_ATTACHMENT_TYPE.to_string(),
                identity_key: format!("{}:{}", record.turn_id, record.call_id),
                payload: serde_json::to_value(record)?,
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_evidence_and_context_snapshot_survive_store_reads() {
        let (mut session, turn, _events) =
            crate::session::tests::make_session_and_context_with_auth_and_config_and_rx(
                codex_login::CodexAuth::create_dummy_chatgpt_auth_for_testing(),
                Vec::new(),
                |_| {},
            )
            .await;
        let config = session.get_config().await;
        let runtime = codex_state::StateRuntime::init(
            config.sqlite.clone(),
            config.model_provider_id.clone(),
        )
        .await
        .unwrap();
        let metadata = codex_state::ThreadMetadataBuilder::new(
            session.thread_id,
            config
                .codex_home
                .join("verification-test.jsonl")
                .to_path_buf(),
            chrono::Utc::now(),
            codex_protocol::protocol::SessionSource::Cli,
        )
        .build(&config.model_provider_id);
        runtime.upsert_thread(&metadata).await.unwrap();
        std::sync::Arc::get_mut(&mut session)
            .unwrap()
            .services
            .thread_store = std::sync::Arc::new(codex_thread_store::LocalThreadStore::new(
            codex_thread_store::LocalThreadStoreConfig::from_config(&config),
            Some(runtime),
        ));
        let record = VerificationRecord {
            schema_version: 1,
            call_id: "check-1".into(),
            turn_id: turn.sub_id.clone(),
            command: vec!["cargo".into(), "check".into()],
            cwd: "file:///workspace".into(),
            completed_at_ms: 1,
            exit_code: 1,
            outcome: VerificationOutcome::Failed,
            workspace_before: None,
            workspace_after: None,
        };
        session.persist_verification(&record).await.unwrap();
        session.persist_context_inspection(&turn).await;
        session.persist_context_inspection(&turn).await;
        let page = session
            .services
            .thread_store
            .list_thread_attachments(ListThreadAttachmentsParams {
                thread_id: session.thread_id,
                cursor: None,
                limit: 100,
            })
            .await
            .unwrap();
        let checks: Vec<_> = page
            .attachments
            .iter()
            .filter(|a| a.attachment_type == VERIFICATION_ATTACHMENT_TYPE)
            .collect();
        assert_eq!(checks.len(), 1);
        assert_eq!(
            serde_json::from_value::<VerificationRecord>(checks[0].payload.clone()).unwrap(),
            record
        );
        assert_eq!(
            page.attachments
                .iter()
                .filter(|a| a.attachment_type == CONTEXT_ATTACHMENT_TYPE)
                .count(),
            1
        );
    }
}
