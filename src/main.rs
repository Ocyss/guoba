#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod client;
mod core;
mod core_control;
mod core_manager;
mod lsp_bridge;
mod scheduler;
mod subscription;
mod yaml_overlay;
use crate::client::InjectHandler;
use crate::client::UnixSocketHandler;
use crate::core::SharedCore;
use rust_embed::RustEmbed;
use salvo::serve_static::StaticDir;
use salvo::serve_static::static_embed;
use salvo::writing::Redirect;
use salvo::{
    Listener,
    prelude::{Router, Server, TcpListener},
};
use salvo::{Response, handler};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
};

const APP_ICON: &[u8] = include_bytes!("../assets/icon.ico");
enum UserEvent {
    TrayIconEvent(tray_icon::TrayIconEvent),
    MenuEvent(tray_icon::menu::MenuEvent),
}

use std::net::{IpAddr, UdpSocket};
use tao::event_loop::ControlFlow;
use tao::{event::Event, event_loop::EventLoopBuilder};

#[handler]
async fn render_ui(res: &mut Response) {
    res.render(Redirect::found("/ui"))
}

#[derive(RustEmbed)]
#[folder = "dist"]
struct DistAssets;

async fn init_server(core: SharedCore) -> Result<(), anyhow::Error> {
    api::init(core.clone());

    let panel_path = core.get_panel();
    let (host, port, sock, remote_url) = {
        let core = core.read().unwrap();
        (
            core.config.host.clone(),
            core.config.port,
            core.path.mihomo_socker.clone(),
            core.config.remote_url.clone(),
        )
    };
    let acceptor = TcpListener::new(format!("{}:{}", host, port))
        .try_bind()
        .await?;

    let router = Router::new()
        .push(api::router())
        .push(
            Router::with_path("/ui/{**}").get(InjectHandler::new(
                StaticDir::new(vec![panel_path])
                    .include_dot_files(false)
                    .auto_list(true)
                    .defaults("index.html")
                    .fallback("index.html"),
            )),
        )
        .push(Router::with_path("/guoba/{*path}").get(static_embed::<DistAssets>()))
        .push(Router::with_path("/").get(render_ui))
        .push(Router::with_path("/{**}").goal(UnixSocketHandler::new(sock)));

    log_server_urls(&host, port, &remote_url);
    println!("{router:?}");

    if let Err(err) = core_manager::ensure_bootstrap_core(&core).await {
        log::warn!("初始化可用内核失败，继续启动: {err:#}");
    }

    match core_manager::auto_update_if_enabled(&core).await {
        Ok(Some(result)) => {
            log::info!(
                "自动更新完成: tag={}, asset={}",
                result.tag,
                result.asset_name
            );
        }
        Ok(None) => {}
        Err(err) => {
            log::warn!("自动更新失败，继续启动: {err:#}");
        }
    }

    if let Err(err) = core_control::restart(&core.clone()) {
        log::warn!("内核启动失败，但 Web 服务继续运行: {err:#}");
    }

    // 启动定时任务调度器
    let mut sched = scheduler::Scheduler::new();
    sched.start(core).await;

    Server::new(acceptor)
        .serve(salvo::Service::new(router).hoop(salvo::logging::Logger::new()))
        .await;

    Ok(())
}

#[tokio::main]
async fn main() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(env_filter)
        .init();

    log::info!("Server start");

    let core = core::Core::new();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let tray_menu = Menu::with_items(&[
        &MenuItem::with_id("panel", "面板", true, None),
        &MenuItem::with_id("config", "配置", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("exit", "退出", true, None),
    ])
    .expect("菜单初始化失败");

    let _tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_menu_on_left_click(false)
        .with_tooltip("system-tray - tray icon library!")
        .with_icon(load_tray_icon(APP_ICON))
        .build()
        .expect("托盘初始化失败");

    let proxy = event_loop.create_proxy();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::TrayIconEvent(event));
    }));

    let proxy = event_loop.create_proxy();
    tray_icon::menu::MenuEvent::set_event_handler(Some(
        move |event: tray_icon::menu::MenuEvent| {
            let _ = proxy.send_event(UserEvent::MenuEvent(event));
        },
    ));

    let on_open_config = || {
        log::info!("Open config...");
    };

    let on_open_panel = {
        let core = core.clone();
        move || {
            log::info!("Open panel...");
            let remote_url = &core.read().unwrap().config.remote_url;
            if let Err(e) = webbrowser::open(remote_url) {
                log::error!("Failed to open panel: {}", e);
            }
        }
    };

    let mut server_started = false;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(_) => {
                if !server_started {
                    server_started = true;
                    tokio::spawn(init_server(core.clone()));
                }
            }
            Event::UserEvent(event) => match event {
                UserEvent::TrayIconEvent(e) => match e {
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } => on_open_panel(),
                    // TrayIconEvent::DoubleClick {
                    //     button: MouseButton::Left,
                    //     ..
                    // } => on_open_config(),
                    _ => (),
                },
                UserEvent::MenuEvent(e) => match e.id.as_ref() {
                    "panel" => on_open_panel(),
                    "config" => on_open_config(),
                    "exit" => {
                        log::info!("Exiting application...");
                        std::process::exit(0);
                    }
                    _ => (),
                },
            },
            _ => (),
        }
    });
}

#[must_use]
fn load_tray_icon(image_data: &[u8]) -> Icon {
    let image = image::load_from_memory(image_data).unwrap();
    let image_buffer = image.to_rgba8();
    let pixels = image_buffer.into_flat_samples().samples;

    Icon::from_rgba(pixels, image.width(), image.height()).unwrap()
}

fn log_server_urls(host: &str, port: u16, remote_url: &str) {
    let mut urls = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];

    if host != "127.0.0.1" && host != "0.0.0.0" && host != "localhost" {
        urls.push(format!("http://{host}:{port}"));
    }

    if let Some(local_ip) = detect_primary_local_ip() {
        urls.push(format!("http://{local_ip}:{port}"));
    }

    if !remote_url.trim().is_empty() {
        urls.push(remote_url.to_string());
    }

    urls.sort();
    urls.dedup();

    println!("----------------------------------------");
    println!("Minihomo Web 服务已启动");
    println!("监听地址: {}:{}", host, port);
    println!("可访问地址:");
    for url in urls {
        println!("  > {}", url);
    }
    println!("----------------------------------------");
}

fn detect_primary_local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}
