use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use rlsp_yaml::server::Backend;
use salvo::prelude::*;
use salvo::websocket::{Message, WebSocketUpgrade};
use tokio::sync::mpsc;
use tower::Service;
use tower_lsp::LspService;
use tower_lsp::jsonrpc::{Request as JsonRpcRequest, Response as JsonRpcResponse};

#[handler]
pub async fn editor_ws(req: &mut Request, res: &mut Response) -> Result<(), StatusError> {
    WebSocketUpgrade::new()
        .upgrade(req, res, |ws| async move {
            if let Err(error) = run_editor_ws(ws).await {
                log::warn!("editor ws bridge error: {error:#}");
            }
        })
        .await
}

async fn run_editor_ws(ws: salvo::websocket::WebSocket) -> Result<()> {
    let (mut ws_sender, mut ws_receiver) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let (mut service, mut socket) = LspService::new(Backend::new);

    let writer_task = tokio::spawn(async move {
        while let Some(payload) = rx.recv().await {
            ws_sender
                .send(Message::text(payload))
                .await
                .map_err(|error| anyhow::anyhow!("websocket send failed: {error}"))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let notification_tx = tx.clone();
    let notification_task = tokio::spawn(async move {
        while let Some(message) = socket.next().await {
            let payload = serde_json::to_string(&message)
                .context("serialize outbound LSP notification failed")?;
            notification_tx
                .send(payload)
                .map_err(|_| anyhow::anyhow!("notification channel closed"))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    while let Some(frame) = ws_receiver.next().await {
        let frame = frame.map_err(|error| anyhow::anyhow!("websocket receive failed: {error}"))?;
        if frame.is_close() {
            break;
        }
        if !frame.is_text() {
            continue;
        }

        let text = frame
            .as_str()
            .map_err(|error| anyhow::anyhow!("invalid websocket text frame: {error}"))?;
        let request = match serde_json::from_str::<JsonRpcRequest>(text) {
            Ok(request) => request,
            Err(error) => {
                log::warn!("invalid LSP request payload: {error}");
                continue;
            }
        };

        if let Some(response) = service
            .call(request)
            .await
            .map_err(|error| anyhow::anyhow!("LSP service call failed: {error}"))?
        {
            send_response(&tx, response)?;
        }
    }

    drop(tx);
    notification_task.abort();

    match writer_task.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(error) if error.is_cancelled() => {}
        Err(error) => return Err(anyhow::anyhow!("writer task join failed: {error}")),
    }

    match notification_task.await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(error),
        Err(error) if error.is_cancelled() => {}
        Err(error) => {
            return Err(anyhow::anyhow!("notification task join failed: {error}"));
        }
    }

    Ok(())
}

fn send_response(tx: &mpsc::UnboundedSender<String>, response: JsonRpcResponse) -> Result<()> {
    let payload = serde_json::to_string(&response).context("serialize LSP response failed")?;
    tx.send(payload)
        .map_err(|_| anyhow::anyhow!("response channel closed"))
}
