//! In-process round trip over the constructed server via `tower_lsp`'s
//! `LspService` (no IO): initialize → didOpen a deliberately broken program →
//! assert diagnostics match what `mt_stc::compile` itself reports →
//! cross-file goto-definition → hover → shutdown.

use std::time::Duration;

use futures::StreamExt;
use tower::{Service, ServiceExt};
use tower_lsp::jsonrpc::Request;
use tower_lsp::lsp_types;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Hover, Url};
use tower_lsp::LspService;

use voltrus_lsp::server::Backend;

const MOTOR_ST: &str = "\
FUNCTION_BLOCK Motor
VAR_INPUT
    speed : INT;
END_VAR
END_FUNCTION_BLOCK
";

// Deliberately broken: `boom` is never declared, so mt-stc's codegen reports
// `Unknown variable 'boom'` at the use site (1-based line 6, char col 4).
const BROKEN_MAIN_ST: &str = "\
PROGRAM Main
VAR
    m : Motor;
END_VAR
    boom := 1;
END_PROGRAM
";

fn request(method: &'static str, params: serde_json::Value, id: i64) -> Request {
    Request::build(method).params(params).id(id).finish()
}

fn notification(method: &'static str, params: serde_json::Value) -> Request {
    Request::build(method).params(params).finish()
}

/// Next server→client publishDiagnostics payload for `uri`, with a timeout.
async fn diagnostics_for(socket: &mut tower_lsp::ClientSocket, uri: &Url) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let next = tokio::time::timeout(Duration::from_millis(500), socket.next()).await;
        match next {
            Ok(Some(req)) if req.method() == "textDocument/publishDiagnostics" => {
                let params = req.params().expect("params");
                if params["uri"].as_str() == Some(uri.as_str()) {
                    return params.clone();
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => break,  // socket closed
            Err(_) => continue, // poll timeout; re-check deadline
        }
    }
    panic!("timed out waiting for diagnostics for {uri}");
}

#[tokio::test(flavor = "current_thread")]
async fn round_trip() {
    // Sibling .st file on disk inside the workspace folder.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("motor.st"), MOTOR_ST).expect("write motor.st");
    let motor_uri = Url::from_file_path(dir.path().join("motor.st")).unwrap();
    let main_uri = Url::from_file_path(dir.path().join("main.st")).unwrap();

    let (mut service, mut socket) = LspService::new(Backend::new);

    // initialize with the workspace folder
    let init = request(
        "initialize",
        serde_json::json!({
            "processId": std::process::id(),
            "capabilities": {},
            "workspaceFolders": [{
                "uri": Url::from_file_path(dir.path()).unwrap().to_string(),
                "name": "proj"
            }]
        }),
        1,
    );
    let ok = service
        .ready()
        .await
        .unwrap()
        .call(init)
        .await
        .expect("initialize responds")
        .expect("initialize ok");
    assert!(ok.result().is_some());

    // didOpen the broken program
    let open = notification(
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": main_uri.to_string(),
                "languageId": "iecst",
                "version": 1,
                "text": BROKEN_MAIN_ST
            }
        }),
    );
    service
        .ready()
        .await
        .unwrap()
        .call(open)
        .await
        .expect("notification handled");

    // The server publishes diagnostics for the opened document.
    let published = diagnostics_for(&mut socket, &main_uri).await;
    let diags: Vec<lsp_types::Diagnostic> =
        serde_json::from_value(published["diagnostics"].clone()).expect("diagnostics array");
    assert!(!diags.is_empty(), "expected diagnostics for broken file");
    let err = &diags[0];

    // The mapped range must match mt-stc's own output for this source.
    let out = mt_stc::compile(BROKEN_MAIN_ST);
    assert!(!out.success);
    let expected = &out.diagnostics[0];
    assert_eq!(err.message, expected.message);
    assert_eq!(
        (err.range.start.line, err.range.start.character),
        (expected.line as u32 - 1, expected.col as u32)
    );
    assert_eq!(
        (err.range.end.line, err.range.end.character),
        (expected.end_line as u32 - 1, expected.end_col as u32)
    );
    assert_eq!(err.severity, Some(lsp_types::DiagnosticSeverity::ERROR));

    // goto-definition on `Motor` (main.st line 2, char 11) lands in motor.st.
    let def = request(
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": main_uri.to_string() },
            "position": { "line": 2, "character": 11 }
        }),
        2,
    );
    let resp = service
        .ready()
        .await
        .unwrap()
        .call(def)
        .await
        .expect("definition responds")
        .expect("definition ok");
    let value = resp.result().expect("result").clone();
    let parsed: GotoDefinitionResponse = serde_json::from_value(value).expect("typed response");
    match parsed {
        GotoDefinitionResponse::Scalar(location) => {
            assert_eq!(location.uri, motor_uri);
            assert_eq!(location.range.start.line, 0);
            // Range covers the `Motor` identifier, not the FUNCTION_BLOCK keyword.
            assert!(location.range.start.character >= 15);
        }
        other => panic!("expected scalar location, got {other:?}"),
    }

    // hover over `m` (line 2, char 4) describes the same-file VAR.
    let hover = request(
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": main_uri.to_string() },
            "position": { "line": 2, "character": 4 }
        }),
        3,
    );
    let resp = service
        .ready()
        .await
        .unwrap()
        .call(hover)
        .await
        .expect("hover responds")
        .expect("hover ok");
    let value = resp.result().expect("result").clone();
    let parsed: Hover = serde_json::from_value(value).expect("typed hover");
    let text = match &parsed.contents {
        tower_lsp::lsp_types::HoverContents::Markup(m) => m.value.clone(),
        other => panic!("expected markup hover, got {other:?}"),
    };
    assert!(text.contains("VAR"), "hover mentions kind: {text}");
    assert!(text.contains("Motor"), "hover mentions type: {text}");

    // shutdown
    let shutdown = Request::build("shutdown").id(4).finish();
    let resp = service
        .ready()
        .await
        .unwrap()
        .call(shutdown)
        .await
        .expect("shutdown responds")
        .expect("shutdown ok");
    assert!(resp.is_ok(), "shutdown error: {:?}", resp.error());
}
