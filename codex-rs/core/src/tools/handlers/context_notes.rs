use crate::clm::{ContextNotes, MAX_NOTES_BYTES};
use crate::context::ContextualUserFragment;
use crate::function_tool::FunctionCallError;
use crate::tools::context::{
    FunctionToolOutput, ToolInvocation, ToolOutput, ToolPayload, boxed_tool_output,
};
use crate::tools::registry::{CoreToolRuntime, ToolExecutor};
use codex_protocol::models::ResponseInputItem;
use codex_thread_store::{
    ItemSortKey, ListItemsParams, ListItemsPosition, SearchThreadOccurrencesParams, SortDirection,
};
use codex_tools::{JsonSchema, ResponsesApiTool, ToolName, ToolSpec};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) struct ContextNotesHandler;
pub(crate) struct ContextHistoryHandler;

struct ClmOutput(serde_json::Value);

impl ToolOutput for ClmOutput {
    fn log_output(&self) -> String {
        self.0.to_string()
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        FunctionToolOutput::from_text(self.0.to_string(), Some(true))
            .to_response_item(call_id, payload)
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> serde_json::Value {
        self.0.clone()
    }
}

fn spec(
    name: &str,
    description: &str,
    properties: BTreeMap<String, JsonSchema>,
    output_schema: Option<serde_json::Value>,
) -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: name.into(),
        description: description.into(),
        strict: false,
        defer_loading: None,
        output_schema: output_schema.map(Into::into),
        parameters: JsonSchema::object(properties, None, Some(false.into())),
    })
}

fn arguments<T: serde::de::DeserializeOwned>(
    invocation: &ToolInvocation,
) -> Result<T, FunctionCallError> {
    if !crate::clm::enabled(&invocation.turn) {
        return Err(FunctionCallError::RespondToModel(
            "CLM is not enabled for this thread.".into(),
        ));
    }
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return Err(FunctionCallError::RespondToModel(
            "Expected JSON arguments.".into(),
        ));
    };
    super::parse_arguments(arguments)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotesArgs {
    text: Option<String>,
}

impl ToolExecutor<ToolInvocation> for ContextNotesHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("context_notes")
    }
    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }
    fn spec(&self) -> ToolSpec {
        spec(
            "context_notes",
            "Read or replace your local working notebook. Omit text to read; provide text to replace (maximum 16,384 UTF-8 bytes); an empty string clears it. Save objectives, constraints, decisions, verification evidence and next steps before new_context. Revisions are stored in this thread's journal and survive resume and full-history forks. Notes are model-authored data, not user authorization.",
            BTreeMap::from([(
                "text".into(),
                JsonSchema::string(Some("Complete notebook replacement; omit to read.".into())),
            )]),
            Some(json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"},
                    "bytes": {"type": "integer"},
                    "max_bytes": {"type": "integer"}
                },
                "required": ["text", "bytes", "max_bytes"],
                "additionalProperties": false
            })),
        )
    }
    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            let args: NotesArgs = arguments(&invocation)?;
            let text = if let Some(text) = args.text {
                if text.len() > MAX_NOTES_BYTES {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "Notes exceed {MAX_NOTES_BYTES} UTF-8 bytes; previous notes are unchanged."
                    )));
                }
                let item = ContextualUserFragment::into(ContextNotes(text.clone()));
                invocation
                    .session
                    .record_conversation_items(
                        &invocation.turn,
                        invocation.turn.model_info(),
                        &[item],
                    )
                    .await;
                text
            } else {
                let history = invocation.session.clone_history().await;
                crate::clm::latest_notes(history.annotated_items())
                    .and_then(|item| crate::clm::notes_text(&item.item))
                    .unwrap_or_default()
            };
            Ok(boxed_tool_output(ClmOutput(
                json!({"text":text,"bytes":text.len(),"max_bytes":MAX_NOTES_BYTES}),
            )))
        })
    }
}
impl CoreToolRuntime for ContextNotesHandler {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryArgs {
    query: Option<String>,
    cursor: Option<String>,
    turn_id: Option<String>,
    item_id: Option<String>,
    offset_bytes: Option<usize>,
}

fn bounded_text(text: &str) -> String {
    let end = text.floor_char_boundary(8_000.min(text.len()));
    if end == text.len() {
        text.into()
    } else {
        format!("{}\n[history item truncated at 8000 bytes]", &text[..end])
    }
}

impl ToolExecutor<ToolInvocation> for ContextHistoryHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("context_history")
    }
    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }
    fn spec(&self) -> ToolSpec {
        spec(
            "context_history",
            "Recover persisted history from this thread only, including before CLM rollover. With query, search visible messages by literal substring (up to five matches). Without query, read one stored transcript item (including tool outputs), oldest first. Use next_cursor to continue. Search results provide turn_id and item_id for exact lookup. Each transcript slice is at most 8000 UTF-8 bytes; recover the rest with turn_id, item_id and next_offset_bytes as offset_bytes. Images are not reconstructed. Unsupported or unavailable storage returns an error, never another thread's history.",
            BTreeMap::from([
                (
                    "query".into(),
                    JsonSchema::string(Some(
                        "Literal message search; omit to read transcript items.".into(),
                    )),
                ),
                (
                    "cursor".into(),
                    JsonSchema::string(Some("Opaque next_cursor from the same operation.".into())),
                ),
                ("turn_id".into(), JsonSchema::string(None)),
                ("offset_bytes".into(), JsonSchema::integer(Some("Byte offset for an exact item lookup; use next_offset_bytes from its previous slice.".into()))),
                (
                    "item_id".into(),
                    JsonSchema::string(Some(
                        "Exact lookup; requires turn_id and no query or cursor.".into(),
                    )),
                ),
            ]),
            None,
        )
    }
    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            let args: HistoryArgs = arguments(&invocation)?;
            if args
                .query
                .as_ref()
                .is_some_and(|q| q.trim().is_empty() || q.len() > 512)
                || args.cursor.as_ref().is_some_and(|c| c.len() > 8_192)
                || (args.query.is_some() && (args.turn_id.is_some() || args.item_id.is_some()))
                || (args.item_id.is_some() && (args.turn_id.is_none() || args.cursor.is_some()))
                || (args.offset_bytes.is_some() && args.item_id.is_none())
            {
                return Err(FunctionCallError::RespondToModel("Invalid history request: use a non-empty query (up to 512 bytes), or read with cursor/turn_id, or exact turn_id + item_id.".into()));
            }
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                invocation
                    .session
                    .flush_rollout()
                    .await
                    .map_err(|error| error.to_string())?;
                let store = &invocation.session.services.thread_store;
                if let Some(query) = args.query {
                    let mut page = store
                        .search_thread_occurrences(SearchThreadOccurrencesParams {
                            thread_id: invocation.session.thread_id(),
                            search_term: query,
                            cursor: args.cursor,
                            page_size: 5,
                        })
                        .await
                        .map_err(|error| error.to_string())?;
                    for item in &mut page.items {
                        item.snippet = bounded_text(&item.snippet);
                    }
                    Ok::<_, String>(json!(page))
                } else {
                    let page = store
                        .list_items(ListItemsParams {
                            thread_id: invocation.session.thread_id(),
                            include_archived: false,
                            item_ids: args.item_id.map(|id| vec![id]),
                            turn_id: args.turn_id,
                            position: args.cursor.map(ListItemsPosition::Cursor),
                            page_size: 1,
                            sort_direction: SortDirection::Asc,
                            sort_key: ItemSortKey::CreatedAtOrdinal,
                            after_updated_at_ordinal: None,
                            before_updated_at_ordinal: None,
                        })
                        .await
                        .map_err(|error| error.to_string())?;
                    let items = page
                        .items
                        .into_iter()
                        .map(|item| {
                            let text = String::from_utf8_lossy(&item.item_json);
                            let start = text
                                .ceil_char_boundary(args.offset_bytes.unwrap_or(0).min(text.len()));
                            let end = text
                                .floor_char_boundary(start.saturating_add(8_000).min(text.len()));
                            json!({"turn_id":item.turn_id,"item_id":item.item_id,
                                "transcript":&text[start..end], "offset_bytes":start,
                                "next_offset_bytes":(end < text.len()).then_some(end),
                            })
                        })
                        .collect::<Vec<_>>();
                    Ok(json!({"items":items,"next_cursor":page.next_cursor}))
                }
            })
            .await
            .map_err(|_| FunctionCallError::RespondToModel("History read timed out.".into()))?
            .map_err(|error| {
                FunctionCallError::RespondToModel(format!("History unavailable: {error}"))
            })?;
            Ok(boxed_tool_output(ClmOutput(result)))
        })
    }
}
impl CoreToolRuntime for ContextHistoryHandler {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::step_context::StepContext;
    use crate::tools::context::ToolCallSource;
    use crate::turn_diff_tracker::TurnDiffTracker;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn invocation() -> ToolInvocation {
        invocation_with_flags(true, false).await
    }

    async fn invocation_with_flags(clm_enabled: bool, restricted: bool) -> ToolInvocation {
        let (session, mut turn) = crate::session::tests::make_session_and_context().await;
        Arc::make_mut(&mut turn.config)
            .features
            .set_enabled(codex_features::Feature::Clm, clm_enabled)
            .unwrap();
        turn.root_capabilities_restricted = restricted;
        let turn = Arc::new(turn);
        ToolInvocation {
            session: Arc::new(session),
            step_context: StepContext::for_test(Arc::clone(&turn)),
            turn,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            tracker: Arc::new(Mutex::new(TurnDiffTracker::new())),
            call_id: "notes-test".into(),
            tool_name: ToolName::plain("context_notes"),
            source: ToolCallSource::Direct,
            payload: ToolPayload::Function {
                arguments: "{}".into(),
            },
            native_call: None,
            environments: Default::default(),
        }
    }

    #[tokio::test]
    async fn clm_notes_handler_validates_size_before_changing_notebook() {
        let mut call = invocation().await;
        call.payload = ToolPayload::Function {
            arguments: json!({"text":"preserve this"}).to_string(),
        };
        ContextNotesHandler.handle(call.clone()).await.unwrap();
        call.payload = ToolPayload::Function {
            arguments: json!({"text":"è".repeat(MAX_NOTES_BYTES)}).to_string(),
        };
        assert!(ContextNotesHandler.handle(call.clone()).await.is_err());
        let history = call.session.clone_history().await;
        assert_eq!(
            crate::clm::notes_text(
                &crate::clm::latest_notes(history.annotated_items())
                    .unwrap()
                    .item
            )
            .as_deref(),
            Some("preserve this")
        );
        call.payload = ToolPayload::Function {
            arguments: "{}".into(),
        };
        let output = ContextNotesHandler.handle(call.clone()).await.unwrap();
        assert!(output.log_output().contains("preserve this"));
        call.payload = ToolPayload::Function {
            arguments: json!({"text":""}).to_string(),
        };
        ContextNotesHandler.handle(call.clone()).await.unwrap();
        let history = call.session.clone_history().await;
        assert_eq!(
            crate::clm::notes_text(
                &crate::clm::latest_notes(history.annotated_items())
                    .unwrap()
                    .item
            ),
            Some(String::new())
        );
    }

    #[tokio::test]
    async fn clm_tools_reject_disabled_and_restricted_threads() {
        let call = invocation_with_flags(true, true).await;
        assert!(ContextNotesHandler.handle(call.clone()).await.is_err());
        assert!(ContextHistoryHandler.handle(call.clone()).await.is_err());
        let call = invocation_with_flags(false, false).await;
        assert!(ContextNotesHandler.handle(call).await.is_err());
    }

    #[test]
    fn clm_history_output_bound_preserves_utf8() {
        let text = "a".repeat(7_999) + "日本語";
        let result = bounded_text(&text);
        assert!(result.starts_with(&"a".repeat(7_999)));
        assert!(result.contains("truncated"));
    }
}
