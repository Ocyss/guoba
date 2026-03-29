use http_body_util::BodyExt as _;
use hyper_util::client::legacy::Client as HyperUtilClient;
use hyper_util::rt::TokioIo;
use hyperlocal_with_windows::{UnixClientExt as _, UnixConnector};
use salvo::async_trait;
use salvo::http::{ReqBody, ResBody, StatusCode};
use salvo::prelude::{Depot, FlowCtrl, Handler, Request, Response};
use salvo::writing::Json;
use salvo::writing::Text;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct UnixSocketHandler {
    client: HyperUtilClient<UnixConnector, ReqBody>,
    socket_path: PathBuf,
    parts: http::request::Parts,
}

impl UnixSocketHandler {
    #[must_use]
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        let request = http::Request::new("");
        let (parts, _) = request.into_parts();
        Self {
            client: HyperUtilClient::unix(),
            socket_path: socket_path.into(),
            parts,
        }
    }
}

#[async_trait]
impl Handler for UnixSocketHandler {
    async fn handle(
        &self,
        req: &mut Request,
        _depot: &mut Depot,
        res: &mut Response,
        _ctrl: &mut FlowCtrl,
    ) {
        let path_and_query = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/")
            .to_string();

        let mut p = self.parts.clone();
        p.method = req.method().clone();
        p.headers = req.headers().clone();
        p.version = req.version();
        p.extensions = req.extensions().clone();

        p.uri =
            hyperlocal_with_windows::Uri::new(&self.socket_path, path_and_query.as_str()).into();

        let mut response = match self
            .client
            .request(hyper::Request::from_parts(p, req.take_body()))
            .await
        {
            Ok(response) => response,
            Err(err) => {
                log::error!("unix socket request failed: {:?}", err);
                if path_and_query.starts_with("/providers/proxies") {
                    res.status_code(StatusCode::OK);
                    res.render(Json(fake_providers_payload(path_and_query.as_str())));
                    return;
                }
                res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
                return;
            }
        };
        res.status_code = Some(response.status());
        res.headers = response.headers().clone();
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            let back_upgrade = hyper::upgrade::on(&mut response);
            let client_upgrade = req.extensions_mut().remove::<hyper::upgrade::OnUpgrade>();
            if let Some(c_upgrade) = client_upgrade {
                tokio::spawn(async move {
                    if let (Ok(b), Ok(c)) = tokio::join!(back_upgrade, c_upgrade) {
                        let _ = tokio::io::copy_bidirectional(
                            &mut TokioIo::new(b),
                            &mut TokioIo::new(c),
                        )
                        .await;
                    }
                });
            }
        } else {
            let (parts, body) = response.into_parts();
            res.body = ResBody::Hyper(body);
            res.extensions = parts.extensions;
        }
    }
}

fn fake_providers_payload(path: &str) -> serde_json::Value {
    if path == "/providers/proxies" {
        serde_json::json!({
            "providers": {}
        })
    } else {
        serde_json::json!({
            "name": "offline",
            "proxies": []
        })
    }
}

pub struct InjectHandler<H> {
    inner: H,
}

impl<H> InjectHandler<H>
where
    H: Handler,
{
    #[must_use]
    pub fn new(handler: H) -> Self {
        Self { inner: handler }
    }
}

#[async_trait]
impl<H> Handler for InjectHandler<H>
where
    H: Handler,
{
    async fn handle(
        &self,
        req: &mut Request,
        depot: &mut Depot,
        res: &mut Response,
        ctrl: &mut FlowCtrl,
    ) {
        self.inner.handle(req, depot, res, ctrl).await;
        let is_html = res
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap_or("").contains("text/html"))
            .unwrap_or(false);

        if res.status_code == Some(StatusCode::OK) && is_html {
            let body = res.take_body();
            if let Ok(collected) = body.collect().await {
                let bytes = collected.to_bytes();
                let mut content = String::from_utf8_lossy(&bytes).into_owned();

                #[cfg(feature = "local_proxy")]
                let script = "<script type=\"module\" src=\"http://127.0.0.1:5173/@fs/F:/toy/minihomo/src-ui/index.tsx\"></script></head>";
                #[cfg(not(feature = "local_proxy"))]
                let script = "<script type=\"module\" src=\"/guoba/__guoba__.js\"></script></head>";

                content = content.replace("</head>", script);
                res.headers_mut().remove(hyper::header::CONTENT_LENGTH);
                res.headers_mut().remove(hyper::header::CONTENT_ENCODING);
                res.render(Text::Html(content));
            }
        }
    }
}
