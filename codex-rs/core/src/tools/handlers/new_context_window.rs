use crate::compact::AutoCompactionDisabled;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::new_context_window_spec::NEW_CONTEXT_WINDOW_TOOL_NAME;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_protocol::openai_models::ToolMessage;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

pub(crate) const NEW_CONTEXT_WINDOW_MESSAGE: &str =
    "A new context window will start without summarizing conversation history.";

pub struct NewContextWindowHandler {
    spec: ToolSpec,
}

impl NewContextWindowHandler {
    pub(crate) fn new(messages: Option<&ToolMessage>) -> Self {
        Self {
            spec: super::new_context_window_spec::create_new_context_window_tool(messages),
        }
    }
}

impl ToolExecutor<ToolInvocation> for NewContextWindowHandler {
    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }
    fn tool_name(&self) -> ToolName {
        ToolName::plain(NEW_CONTEXT_WINDOW_TOOL_NAME)
    }

    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            if !matches!(invocation.payload, ToolPayload::Function { .. }) {
                return Err(FunctionCallError::RespondToModel(
                    "new_context handler received unsupported payload".to_string(),
                ));
            }

            if invocation
                .session
                .services
                .thread_extension_data
                .get::<AutoCompactionDisabled>()
                .is_some()
            {
                return Err(FunctionCallError::RespondToModel(
                    "Context resets are disabled for this thread.".to_string(),
                ));
            }

            if crate::clm::enabled(&invocation.turn) {
                let history = invocation.session.clone_history().await;
                if !invocation
                    .session
                    .services
                    .thread_store
                    .supports_paginated_history_lists()
                    || !matches!(
                        invocation.turn.history_mode,
                        codex_protocol::protocol::ThreadHistoryMode::Paginated
                    )
                {
                    return Err(FunctionCallError::RespondToModel(
                        "CLM rollover requires a thread store with searchable persisted history."
                            .into(),
                    ));
                }
                if crate::clm::latest_notes(history.annotated_items())
                    .and_then(|item| crate::clm::notes_text(&item.item))
                    .is_none_or(|text| text.trim().is_empty())
                {
                    return Err(FunctionCallError::RespondToModel(
                        "Save non-empty context_notes before requesting a new context window."
                            .into(),
                    ));
                }
            }

            invocation.session.request_new_context_window().await;

            Ok(boxed_tool_output(FunctionToolOutput::from_text(
                if crate::clm::enabled(&invocation.turn) {
                    "CLM rollover requested at the next safe tool boundary. Notes and complete recent tool units will be retained; if rollover cannot safely free space, ordinary compaction may be used.".into()
                } else {
                    NEW_CONTEXT_WINDOW_MESSAGE.to_string()
                },
                Some(true),
            )))
        })
    }
}

impl CoreToolRuntime for NewContextWindowHandler {}
