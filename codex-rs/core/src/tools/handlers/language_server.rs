use crate::function_tool::FunctionCallError;
use crate::tools::context::{FunctionToolOutput, ToolInvocation, ToolPayload, boxed_tool_output};
use crate::tools::registry::{CoreToolRuntime, ToolExecutor};
use codex_tools::{JsonSchema, ResponsesApiTool, ToolName, ToolSpec};
use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) struct LanguageServerHandler {
    pub(crate) server_names: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    server: String,
    path: String,
    operation: String,
    #[serde(default)]
    line: u32,
    #[serde(default)]
    character: u32,
    new_name: Option<String>,
}

impl ToolExecutor<ToolInvocation> for LanguageServerHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("language_server")
    }
    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }
    fn spec(&self) -> ToolSpec {
        ToolSpec::Function(ResponsesApiTool {
            name: "language_server".to_string(),
            description: "Query a configured language server for diagnostics, definitions, references or a semantic rename preview. Positions are zero-based UTF-16 code units. Rename returns edits without applying them: use the normal approved file-edit tools. Diagnostics are not test results. Results are rejected if the document changes during the query.".to_string(),
            strict: false, defer_loading: None, output_schema: None,
            parameters: JsonSchema::object(BTreeMap::from([
                ("server".to_string(), JsonSchema::string_enum(self.server_names.iter().cloned().map(Into::into).collect(), Some("Configured language server".into()))),
                ("path".to_string(), JsonSchema::string(Some("Workspace-relative document path".into()))),
                ("operation".to_string(), JsonSchema::string_enum(vec!["diagnostics".into(),"definition".into(),"references".into(),"rename".into()], None)),
                ("line".to_string(), JsonSchema::integer(None)),
                ("character".to_string(), JsonSchema::integer(None)),
                ("new_name".to_string(), JsonSchema::string(None)),
            ]), Some(vec!["server".into(),"path".into(),"operation".into()]), Some(false.into())),
        })
    }
    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            let ToolPayload::Function { arguments } = &invocation.payload else {
                return Err(FunctionCallError::RespondToModel(
                    "language_server requires JSON arguments".into(),
                ));
            };
            let args: Args = super::parse_arguments(arguments)?;
            let server = invocation
                .turn
                .config
                .language_servers
                .get(&args.server)
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel("Unknown configured language server".into())
                })?;
            let environment = super::selected_executor(
                &invocation.environments,
                &invocation.step_context.environments,
            )?;
            let path = environment
                .cwd()
                .join(&args.path)
                .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
            let result = crate::language_server::query(
                &invocation.session,
                &invocation.turn,
                environment,
                server,
                &path,
                &args.operation,
                args.line,
                args.character,
                args.new_name.as_deref(),
            )
            .await
            .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
            Ok(boxed_tool_output(FunctionToolOutput::from_text(
                result.to_string(),
                Some(true),
            )))
        })
    }
}
impl CoreToolRuntime for LanguageServerHandler {}
