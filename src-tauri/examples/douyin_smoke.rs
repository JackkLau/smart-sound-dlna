//! Opt-in real public-video acceptance, with an isolated library and no accounts.
use std::{sync::Arc, time::Duration};
use xiaomi_dlna_desktop::{douyin, engine::Engine};
#[path = "../src/douyin_webview.rs"]
mod douyin_webview;
fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("public video URL required");
    let directory: std::path::PathBuf = args
        .next()
        .expect("isolated output directory required")
        .into();
    let mut context = xiaomi_dlna_desktop::desktop_context();
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .setup(move |app| {
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = async {
                    let engine = Engine::new(directory.clone()).await?;
                    let id = douyin::resolve_input(&input).await?;
                    let video = match douyin::fetch(&id).await {
                        Ok(video) => video,
                        Err(_) => douyin_webview::read(&app, &id, engine.cancellation()).await?,
                    };
                    println!(
                        "Parsed public video: {} / {} / {}s",
                        video.id, video.title, video.duration
                    );
                    let preview = engine.downloads.register_douyin(video).await?;
                    let job = engine
                        .downloads
                        .start(
                            Arc::downgrade(&engine),
                            &preview.id,
                            1,
                            engine.cancellation(),
                        )
                        .await?;
                    let mut previous = String::new();
                    let result = tokio::time::timeout(Duration::from_secs(240), async {
                        loop {
                            let current = engine
                                .downloads
                                .state()
                                .await
                                .jobs
                                .into_iter()
                                .find(|j| j.id == job.id)
                                .ok_or("missing job")?;
                            if current.phase != previous {
                                println!("Phase: {}", current.phase);
                                previous = current.phase.clone();
                            }
                            match current.phase.as_str() {
                                "completed" => return Ok(current),
                                "failed" | "cancelled" => {
                                    return Err(current
                                        .error
                                        .unwrap_or("download cancelled".into()))
                                }
                                _ => tokio::time::sleep(Duration::from_millis(250)).await,
                            }
                        }
                    })
                    .await
                    .map_err(|_| "download timed out")?;
                    engine.shutdown();
                    let completed = result?;
                    let library = xiaomi_dlna_desktop::library::Library::load(directory)?;
                    let track = library.get(&completed.track_id.unwrap())?;
                    println!(
                        "PASS: restored {} / {} / {} bytes",
                        track.title, track.format, track.size
                    );
                    Ok::<(), String>(())
                }
                .await;
                match result {
                    Ok(()) => app.exit(0),
                    Err(error) => {
                        eprintln!("FAIL: {error}");
                        app.exit(1);
                    }
                }
            });
            Ok(())
        })
        .build(context)
        .unwrap()
        .run(|_, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
}
