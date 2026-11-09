//! Versioned session continuity records carried by thread-store attachments.
//! Model claims are deliberately separate from runtime verification evidence.

use crate::ThreadId;
use serde::Deserialize;
use serde::Serialize;

pub const HANDOFF_ATTACHMENT_TYPE: &str = "codex.handoff.v1";
pub const VERIFICATION_ATTACHMENT_TYPE: &str = "codex.verification.v1";
pub const CONTEXT_ATTACHMENT_TYPE: &str = "codex.context.v1";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextInspection {
    pub schema_version: u32,
    pub turn_id: String,
    pub captured_at_ms: i64,
    pub active_context_tokens: i64,
    pub auto_compact_scope_tokens: i64,
    pub auto_compact_scope_limit: Option<i64>,
    pub full_context_window_limit: Option<i64>,
    pub remaining_tokens: Option<i64>,
    pub serialized_instruction_bytes: usize,
    pub serialized_history_bytes: usize,
    pub serialized_tool_output_bytes: usize,
    /// Tool definitions are not included in raw conversation history.
    pub tool_definition_bytes: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Succeeded,
    Failed,
    NotRun,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct VerificationRecord {
    pub schema_version: u32,
    pub call_id: String,
    pub turn_id: String,
    pub command: Vec<String>,
    pub cwd: String,
    pub completed_at_ms: i64,
    pub exit_code: i32,
    pub outcome: VerificationOutcome,
    /// Git-visible contents only; absent when capture is unsupported or unstable.
    pub workspace_before: Option<String>,
    pub workspace_after: Option<String>,
}

impl VerificationRecord {
    pub fn freshness(&self, current: Option<&str>) -> &'static str {
        match (
            self.workspace_before.as_deref(),
            self.workspace_after.as_deref(),
            current,
        ) {
            (Some(before), Some(after), Some(current)) if before == after && after == current => {
                "matching Git-visible files"
            }
            (Some(_), Some(_), Some(_)) => "stale (files changed)",
            _ => "unknown (workspace not fully captured)",
        }
    }
}

/// Recognize simple check invocations, never shell expressions such as `echo
/// cargo test` or `cargo test || true`. Runtime exit status remains authoritative.
pub fn is_verification_command(command: &[String]) -> bool {
    let shell = command
        .first()
        .and_then(|program| program.rsplit(['/', '\\']).next());
    let args: Vec<&str> = if command.len() >= 3
        && matches!(shell, Some("sh" | "bash" | "zsh" | "dash" | "ksh"))
        && matches!(command[1].as_str(), "-c" | "-lc")
    {
        // Validate the original script: tokenization would erase newline separators
        // and could mistake a later command's exit status for the check's result.
        if command[2].contains([';', '|', '&', '`', '$', '\n', '\r']) {
            return false;
        }
        command[2].split_whitespace().collect()
    } else {
        command.iter().map(String::as_str).collect()
    };
    if args
        .iter()
        .any(|s| s.contains([';', '|', '&', '`', '$', '\n']))
    {
        return false;
    }
    matches!(
        args.as_slice(),
        ["cargo", "test" | "check" | "clippy", ..]
            | ["bun" | "npm" | "pnpm" | "yarn", "test", ..]
            | [
                "bun" | "npm" | "pnpm" | "yarn",
                "run",
                "test" | "lint" | "typecheck" | "check",
                ..
            ]
            | ["pytest" | "jest" | "vitest", ..]
            | ["python" | "python3", "-m", "pytest" | "unittest", ..]
            | ["go", "test" | "vet", ..]
    )
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandoffState {
    pub objective: String,
    pub constraints: Vec<String>,
    pub decisions: Vec<String>,
    pub completed_work: Vec<String>,
    pub open_tasks: Vec<String>,
    pub changed_files: Vec<String>,
}

impl HandoffState {
    pub fn is_bounded(&self) -> bool {
        let lists = [
            &self.constraints,
            &self.decisions,
            &self.completed_work,
            &self.open_tasks,
            &self.changed_files,
        ];
        self.objective.chars().count() <= 2_000
            && lists.iter().all(|list| {
                list.len() <= 32 && list.iter().all(|text| text.chars().count() <= 1_000)
            })
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= 20_000)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandoffRecord {
    pub schema_version: u32,
    pub id: String,
    pub source_thread_id: ThreadId,
    pub source_turn_revision: usize,
    pub source_completed_turns: usize,
    pub created_at_ms: i64,
    pub cwd: String,
    /// All fields in this section are model-generated claims, not runtime evidence.
    pub model_state: HandoffState,
    pub summary: String,
    pub next_action: Option<String>,
    /// Runtime records copied from the source thread, never supplied by the recap model.
    #[serde(default)]
    pub verification_evidence: Vec<VerificationRecord>,
}

impl HandoffRecord {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("unsupported handoff version");
        }
        if self.id.is_empty()
            || self.id.len() > 128
            || self.cwd.len() > 4096
            || self.summary.trim().is_empty()
            || self.summary.chars().count() > 12_000
            || self
                .next_action
                .as_ref()
                .is_some_and(|s| s.chars().count() > 2_000)
            || !self.model_state.is_bounded()
            || self.verification_evidence.len() > 5
            || !serde_json::to_vec(&self.verification_evidence)
                .is_ok_and(|bytes| bytes.len() <= 16_000)
            || !serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= 64 * 1024)
        {
            return Err("invalid or oversized handoff record");
        }
        Ok(())
    }

    /// Render from the persisted record, so clients do not maintain a second summary.
    pub fn render(&self) -> String {
        format!(
            "Continue the work from conversation {} using handoff {}. Verify the current workspace before making changes.\n\nThe following is model-generated context, not proof that checks pass on the current files.\n\n{}\n\nStructured state:\n{}\n\nNext steps: {}\n\nRuntime verification evidence (historical outcomes only; freshness must be checked against current files, and absent evidence means unknown):\n{}",
            self.source_thread_id,
            self.id,
            self.summary,
            serde_json::to_string_pretty(&self.model_state).unwrap_or_default(),
            self.next_action
                .as_deref()
                .unwrap_or("Verify the workspace and identify remaining work."),
            serde_json::to_string_pretty(&self.verification_evidence).unwrap_or_default()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_detection_rejects_masked_failures() {
        assert!(!is_verification_command(&[
            "echo".into(),
            "-c".into(),
            "cargo test".into()
        ]));
        for (script, expected) in [
            ("cargo test --lib", true),
            ("cargo test || true", false),
            ("echo cargo test", false),
            ("npm run typecheck", true),
        ] {
            assert_eq!(
                is_verification_command(&["zsh".into(), "-lc".into(), script.into()]),
                expected
            );
        }
    }

    #[test]
    fn verification_detection_rejects_shell_newlines_before_tokenizing() {
        for shell in ["sh", "bash", "zsh", "dash", "ksh", "/bin/bash"] {
            for flag in ["-c", "-lc"] {
                for script in [
                    "cargo test\ntrue",
                    "cargo test\r\ntrue",
                    "cargo test --lib\nexit 0",
                    "npm run typecheck\ntrue",
                    "pytest\ntrue",
                    "cargo test\\\n --lib",
                ] {
                    assert!(
                        !is_verification_command(&[shell.into(), flag.into(), script.into()]),
                        "must not classify {shell} {flag} {script:?} as a single check",
                    );
                }
                assert!(is_verification_command(&[
                    shell.into(),
                    flag.into(),
                    "cargo\ttest --lib".into(),
                ]));
            }
        }
        assert!(!is_verification_command(&[
            "cargo".into(),
            "test\ntrue".into()
        ]));
    }

    #[test]
    fn evidence_requires_matching_before_after_and_current_files() {
        let mut record = VerificationRecord {
            schema_version: 1,
            call_id: "c".into(),
            turn_id: "t".into(),
            command: vec!["cargo".into(), "test".into()],
            cwd: "file:///workspace".into(),
            completed_at_ms: 1,
            exit_code: 0,
            outcome: VerificationOutcome::Succeeded,
            workspace_before: Some("a".into()),
            workspace_after: Some("a".into()),
        };
        assert_eq!(record.freshness(Some("a")), "matching Git-visible files");
        assert!(record.freshness(Some("b")).starts_with("stale"));
        record.workspace_before = Some("b".into());
        assert!(record.freshness(Some("a")).starts_with("stale"));
        record.workspace_before = None;
        assert!(record.freshness(Some("a")).starts_with("unknown"));
    }

    #[test]
    fn state_limits_count_unicode_and_bound_total_payload() {
        let mut state = HandoffState {
            objective: "🚀".repeat(2000),
            ..Default::default()
        };
        assert!(state.is_bounded());
        state.objective.push('x');
        assert!(!state.is_bounded());
        state.objective.clear();
        state.open_tasks = vec!["task".into(); 33];
        assert!(!state.is_bounded());
    }
}
