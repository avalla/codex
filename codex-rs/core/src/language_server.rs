//! Bounded LSP requests over the existing sandboxed executor transport.
//! Each request opens a fixed document snapshot. No server-requested edits run.

use crate::session::session::Session;
use crate::session::turn_context::{TurnContext, TurnEnvironment};
use anyhow::{Context, bail};
use codex_config::config_toml::LanguageServerConfig;
use codex_exec_server::{
    ExecOutputStream, ExecParams, ExecProcess, ExecProcessEvent, ExecProcessEventReceiver,
    ProcessId,
};
use codex_utils_path_uri::PathUri;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const MAX_MESSAGE: usize = 4 * 1024 * 1024;

struct Rpc {
    process: Arc<dyn ExecProcess>,
    events: ExecProcessEventReceiver,
    buffer: Vec<u8>,
    diagnostics: Option<Value>,
    uri: String,
}

impl Drop for Rpc {
    fn drop(&mut self) {
        let process = self.process.clone();
        tokio::spawn(async move {
            let _ = process.terminate().await;
        });
    }
}

/// Parse one bounded Content-Length frame; retain incomplete frames for the next chunk.
fn take_message(buffer: &mut Vec<u8>) -> anyhow::Result<Option<Value>> {
    let Some(header_end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
        if buffer.len() > 8192 {
            bail!("LSP header too large");
        }
        return Ok(None);
    };
    if header_end > 8192 {
        bail!("LSP header too large");
    }
    let header = std::str::from_utf8(&buffer[..header_end])?;
    let mut length = None;
    for line in header.split("\r\n") {
        let (name, value) = line.split_once(':').context("invalid LSP header")?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                bail!("duplicate Content-Length");
            }
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.context("missing Content-Length")?;
    if length > MAX_MESSAGE {
        bail!("LSP response too large");
    }
    let end = header_end + 4 + length;
    if buffer.len() < end {
        return Ok(None);
    }
    let value = serde_json::from_slice(&buffer[header_end + 4..end])?;
    buffer.drain(..end);
    Ok(Some(value))
}

impl Rpc {
    async fn send(&self, value: Value) -> anyhow::Result<()> {
        let body = serde_json::to_vec(&value)?;
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend(body);
        let response = self.process.write(frame).await?;
        if response.status != codex_exec_server::WriteStatus::Accepted {
            bail!("LSP stdin closed");
        }
        Ok(())
    }

    async fn message(&mut self) -> anyhow::Result<Value> {
        loop {
            if let Some(value) = take_message(&mut self.buffer)? {
                return Ok(value);
            }
            match self.events.recv().await? {
                ExecProcessEvent::Output(chunk) if chunk.stream == ExecOutputStream::Stdout => {
                    if self.buffer.len() + chunk.chunk.0.len() > MAX_MESSAGE + 8192 {
                        bail!("LSP buffer too large");
                    }
                    self.buffer.extend_from_slice(&chunk.chunk.0);
                }
                ExecProcessEvent::Output(_) => (),
                _ => bail!("language server exited before responding"),
            }
        }
    }

    async fn observe(&mut self, value: &Value) -> anyhow::Result<()> {
        if value.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics") {
            let params = &value["params"];
            if params["uri"].as_str() == Some(&self.uri)
                && (params["version"].is_null() || params["version"].as_i64() == Some(1))
            {
                self.diagnostics = Some(params["diagnostics"].clone());
            }
        } else if value.get("method").is_some() && value.get("id").is_some() {
            let method = value["method"].as_str().unwrap_or_default();
            let response = match method {
                "workspace/configuration" => {
                    json!({"jsonrpc":"2.0", "id":value["id"], "result":value["params"]["items"].as_array().map(|items| vec![Value::Null; items.len()]).unwrap_or_default()})
                }
                "window/workDoneProgress/create" => {
                    json!({"jsonrpc":"2.0", "id":value["id"], "result":null})
                }
                "workspace/applyEdit" => {
                    json!({"jsonrpc":"2.0", "id":value["id"], "result":{"applied":false,"failureReason":"Use the approved file-edit tools"}})
                }
                _ => {
                    json!({"jsonrpc":"2.0", "id":value["id"], "error":{"code":-32601,"message":"Unsupported client method"}})
                }
            };
            self.send(response).await?;
        }
        Ok(())
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> anyhow::Result<Value> {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        loop {
            let value = self.message().await?;
            if value.get("method").is_none() && value["id"].as_u64() == Some(id) {
                if let Some(error) = value.get("error") {
                    bail!("LSP request failed: {error}");
                }
                return Ok(value["result"].clone());
            }
            self.observe(&value).await?;
        }
    }
}

pub(crate) async fn query(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    environment: &TurnEnvironment,
    server: &LanguageServerConfig,
    path: &PathUri,
    operation: &str,
    line: u32,
    character: u32,
    new_name: Option<&str>,
) -> anyhow::Result<Value> {
    tokio::time::timeout(
        Duration::from_secs(20),
        query_inner(
            session,
            turn,
            environment,
            server,
            path,
            operation,
            line,
            character,
            new_name,
        ),
    )
    .await
    .context("LSP request timed out after 20 seconds")?
}

async fn query_inner(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    environment: &TurnEnvironment,
    server: &LanguageServerConfig,
    path: &PathUri,
    operation: &str,
    line: u32,
    character: u32,
    new_name: Option<&str>,
) -> anyhow::Result<Value> {
    if server.command.is_empty() || server.command[0].is_empty() {
        bail!("language server command is empty");
    }
    if !path.starts_with(environment.cwd()) {
        bail!("LSP document must be inside the selected workspace");
    }
    let sandbox = environment.sandbox_context(None);
    let fs = environment.environment().get_filesystem();
    let metadata = fs
        .get_metadata(path, Default::default(), Some(&sandbox))
        .await?;
    if !metadata.is_file {
        bail!("LSP document is not a file");
    }
    if metadata.size > (MAX_MESSAGE / 2) as u64 {
        bail!("LSP document exceeds 2 MiB");
    }
    let text = fs
        .read_file_text(path, Default::default(), Some(&sandbox))
        .await?;
    if text.len() > MAX_MESSAGE / 2 {
        bail!("LSP document exceeds 2 MiB");
    }
    let process_id = ProcessId::from(format!("lsp-{}", uuid::Uuid::new_v4()));
    let network = crate::tools::network_approval::execution_network_proxy(
        session,
        turn,
        Some(environment),
        turn.network.clone(),
        &environment.selection.environment_id,
        environment.permission_profile(),
        environment.config().network_policy.as_ref(),
        process_id.as_str(),
        None,
    )
    .await
    .map_err(|error| anyhow::anyhow!("LSP network setup: {error:?}"))?;
    let network_proxy = if let Some(network) = &network {
        let (launch, _) =
            crate::tools::network_approval::remote_network_proxy_launch(environment, network, turn)
                .await
                .map_err(|error| anyhow::anyhow!("LSP network setup: {error:?}"))?;
        launch.proxy.enabled.then_some(launch)
    } else {
        None
    };
    let process = environment
        .environment()
        .get_exec_backend()
        .start(ExecParams {
            process_id,
            metadata: None,
            argv: server.command.clone(),
            cwd: environment.cwd().clone(),
            env_policy: Some(crate::unified_exec::exec_env_policy_from_shell_policy(
                environment.shell_environment_policy(),
            )),
            shell_snapshot: None,
            env: HashMap::new(),
            tty: false,
            pipe_stdin: true,
            arg0: None,
            enforce_managed_network: network_proxy.is_some(),
            sandbox: Some(sandbox.clone()),
            managed_network: None,
            network_proxy,
        })
        .await?
        .process;
    let uri = path.to_string();
    let mut rpc = Rpc {
        events: process.subscribe_events(),
        process,
        buffer: Vec::new(),
        diagnostics: None,
        uri: uri.clone(),
    };
    let initialized = rpc.request(1, "initialize", json!({
        "processId":null,"rootUri":environment.cwd().to_string(),
        "capabilities":{"general":{"positionEncodings":["utf-16"]},"textDocument":{"publishDiagnostics":{"versionSupport":true},"diagnostic":{}},"workspace":{"applyEdit":false}},
    })).await?;
    if initialized["capabilities"]["positionEncoding"]
        .as_str()
        .is_some_and(|encoding| encoding != "utf-16")
    {
        bail!("server selected unsupported position encoding");
    }
    rpc.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}))
        .await?;
    rpc.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":server.language_id,"version":1,"text":text}}})).await?;
    let result = match operation {
        "diagnostics"
            if initialized["capabilities"]
                .get("diagnosticProvider")
                .is_some_and(|v| !v.is_null() && *v != Value::Bool(false)) =>
        {
            rpc.request(
                2,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":uri}}),
            )
            .await?
        }
        "diagnostics" => loop {
            if let Some(diagnostics) = rpc.diagnostics.take() {
                break json!({"kind":"full","items":diagnostics});
            }
            let value = rpc.message().await?;
            rpc.observe(&value).await?;
        },
        "definition" | "references" | "rename" => {
            let mut params =
                json!({"textDocument":{"uri":uri},"position":{"line":line,"character":character}});
            if operation == "references" {
                params["context"] = json!({"includeDeclaration":true});
            }
            if operation == "rename" {
                params["newName"] = json!(
                    new_name
                        .filter(|name| !name.is_empty())
                        .context("rename requires new_name")?
                );
            }
            rpc.request(2, &format!("textDocument/{operation}"), params)
                .await?
        }
        _ => bail!("unsupported language server operation"),
    };
    let current = fs
        .read_file_text(path, Default::default(), Some(&sandbox))
        .await?;
    if current != text {
        bail!("document changed while LSP was running; discard results and retry");
    }
    // Cleanup has a short independent grace period, followed by Drop termination.
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        rpc.request(3, "shutdown", Value::Null).await?;
        rpc.send(json!({"jsonrpc":"2.0","method":"exit"})).await
    })
    .await;
    Ok(
        json!({"uri":uri,"document_version":1,"position_encoding":"utf-16","operation":operation,
        "edits_applied":false,"result":result}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProcess {
        id: ProcessId,
        writes: std::sync::Mutex<Vec<Value>>,
        stopped: std::sync::atomic::AtomicBool,
    }

    impl ExecProcess for FakeProcess {
        fn process_id(&self) -> &ProcessId {
            &self.id
        }
        fn subscribe_wake(&self) -> tokio::sync::watch::Receiver<u64> {
            tokio::sync::watch::channel(0).1
        }
        fn subscribe_events(&self) -> ExecProcessEventReceiver {
            ExecProcessEventReceiver::empty()
        }
        fn read(
            &self,
            _: Option<u64>,
            _: Option<usize>,
            _: Option<u64>,
        ) -> codex_exec_server::ExecProcessFuture<'_, codex_exec_server::ReadResponse> {
            Box::pin(async { unreachable!() })
        }
        fn write(
            &self,
            mut bytes: Vec<u8>,
        ) -> codex_exec_server::ExecProcessFuture<'_, codex_exec_server::WriteResponse> {
            self.writes
                .lock()
                .unwrap()
                .push(take_message(&mut bytes).unwrap().unwrap());
            Box::pin(async {
                Ok(codex_exec_server::WriteResponse {
                    status: codex_exec_server::WriteStatus::Accepted,
                })
            })
        }
        fn signal(
            &self,
            _: codex_exec_server::ProcessSignal,
        ) -> codex_exec_server::ExecProcessFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn terminate(&self) -> codex_exec_server::ExecProcessFuture<'_, ()> {
            self.stopped
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn rpc_rejects_stale_diagnostics_denies_edits_and_terminates_on_drop() {
        let process = Arc::new(FakeProcess {
            id: ProcessId::from("test".to_string()),
            writes: Default::default(),
            stopped: Default::default(),
        });
        let mut rpc = Rpc {
            process: process.clone(),
            events: process.subscribe_events(),
            buffer: Vec::new(),
            diagnostics: None,
            uri: "file:///test.rs".into(),
        };
        for (uri, version) in [("file:///other.rs", 1), ("file:///test.rs", 2)] {
            rpc.observe(
                &json!({"method":"textDocument/publishDiagnostics","params":{
                "uri":uri,"version":version,"diagnostics":[{"message":"stale"}]}}),
            )
            .await
            .unwrap();
            assert!(rpc.diagnostics.is_none());
        }
        rpc.observe(
            &json!({"method":"textDocument/publishDiagnostics","params":{
            "uri":"file:///test.rs","version":1,"diagnostics":[]}}),
        )
        .await
        .unwrap();
        assert_eq!(rpc.diagnostics, Some(json!([])));
        rpc.observe(&json!({"id":99,"method":"workspace/applyEdit","params":{}}))
            .await
            .unwrap();
        assert_eq!(
            process.writes.lock().unwrap()[0]["result"]["applied"],
            false
        );
        let body = serde_json::to_vec(&json!({"id":1,"result":{"capabilities":{}}})).unwrap();
        rpc.buffer = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        rpc.buffer.extend(body);
        assert_eq!(
            rpc.request(1, "initialize", json!({})).await.unwrap(),
            json!({"capabilities":{}})
        );
        drop(rpc);
        tokio::task::yield_now().await;
        assert!(process.stopped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn frame_parser_handles_partial_and_multiple_unicode_messages() {
        let body = serde_json::to_vec(&json!({"result":"🚀"})).unwrap();
        let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        assert!(take_message(&mut bytes).unwrap().is_none());
        bytes.extend(&body);
        let second = bytes.clone();
        bytes.extend(second);
        assert_eq!(take_message(&mut bytes).unwrap().unwrap()["result"], "🚀");
        assert!(take_message(&mut bytes).unwrap().is_some());
        assert!(bytes.is_empty());
    }

    #[test]
    fn frame_parser_rejects_unbounded_or_ambiguous_frames() {
        for header in [
            "Content-Length: 999999999\r\n\r\n",
            "Content-Length: 0\r\nContent-Length: 0\r\n\r\n",
        ] {
            assert!(take_message(&mut header.as_bytes().to_vec()).is_err());
        }
    }
}
