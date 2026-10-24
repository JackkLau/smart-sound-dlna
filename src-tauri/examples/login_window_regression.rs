//! Native macOS regression for an initial login TLS failure.
//! Run with `--legacy-url` to reproduce Wry's UI-thread panic (exit 101).
//! Default mode exercises the production observer and load timeout (exit 0).
//! This creates only isolated windows and a loopback server; no real accounts.
#[cfg(target_os = "macos")]
#[path = "../src/login_webview.rs"]
mod login_webview;

#[cfg(target_os = "macos")]
fn main() {
    use login_webview::{LoginPage, LOAD_ERROR, LOAD_TIMEOUT};
    use std::{
        io::{Read, Write},
        sync::Arc,
        time::Duration,
    };
    let legacy = std::env::args().any(|arg| arg == "--legacy-url");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            // A non-TLS reply to ClientHello triggers WebKit's -1200 failure.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.read(&mut [0; 1024]);
            let _ = stream.write_all(
                b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            );
        }
    });
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .setup(move |app| {
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = tauri::async_runtime::spawn_blocking(move || {
                    let page = Arc::new(LoginPage::default());
                    let load_page = page.clone();
                    let target: url::Url = format!("https://{address}/").parse().unwrap();
                    let window = tauri::WebviewWindowBuilder::new(
                        &app, "login-regression", tauri::WebviewUrl::External(target.clone()),
                    )
                    .title("登录异常回归（隔离验证）")
                    .incognito(true)
                    .on_navigation(|url| url.scheme() == "https")
                    .on_page_load(move |_, payload| {
                        if payload.event() == tauri::webview::PageLoadEvent::Finished
                            && payload.url().scheme() == "https" {
                            load_page.loaded(false);
                        }
                    })
                    .build().unwrap();
                    let started = std::time::Instant::now();
                    loop {
                        std::thread::sleep(Duration::from_millis(750));
                        let observation = page.observe(&window).unwrap();
                        assert!(observation.cookies.is_empty());
                        assert!(!observation.service_callback);
                        assert!(!page.has_loaded(), "TLS fixture must never commit a page");
                        if legacy && started.elapsed() >= Duration::from_secs(5) {
                            eprintln!("Legacy URL probe after failed initial TLS navigation");
                            let _ = window.url();
                            panic!("expected the legacy URL getter to terminate the UI thread");
                        }
                        if let Some(error) = page.load_error(started.elapsed()) {
                            assert_eq!(error, LOAD_ERROR);
                            assert!(started.elapsed() >= LOAD_TIMEOUT);
                            eprintln!("PASS: TLS failure survives cookie polling and reports a load error");
                            break;
                        }
                        assert!(started.elapsed() < Duration::from_secs(45));
                    }
                    // Retrying the failed native window must also remain safe.
                    window.navigate(target).unwrap();
                    std::thread::sleep(Duration::from_secs(2));
                    assert!(page.observe(&window).unwrap().cookies.is_empty());
                    eprintln!("PASS: retry of the failed window stays responsive");
                    app.exit(0);
                }).await;
                if let Err(error) = result {
                    eprintln!("Regression worker failed: {error}");
                    std::process::exit(1);
                }
            });
            Ok(())
        })
        .build(context).unwrap()
        .run(|_, _| {});
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("This regression requires macOS WKWebView.");
}
