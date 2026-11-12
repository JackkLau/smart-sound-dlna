//! Real, packaged tool test. Explicitly run after npm run prepare:video.
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use xiaomi_dlna_desktop::video::probe_file;

#[tokio::test]
#[ignore = "requires the hash-verified host FFprobe; run npm run prepare:video first"]
async fn packaged_ffprobe_reads_a_real_matroska_file_without_exposing_paths() {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => panic!("This test requires a supported video-preview platform."),
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let tool = root.join("binaries").join(format!(
        "ffprobe-{target}{}",
        if cfg!(windows) { ".exe" } else { "" }
    ));
    assert!(tool.is_file(), "run npm run prepare:video before this test");
    let fixture = root.join("tests/fixtures/generated-video.mkv");
    let source = probe_file(fixture.clone(), tool, CancellationToken::new())
        .await
        .expect("real FFprobe should analyze the generated MKV");
    assert_eq!(source.info.video_codec.as_deref(), Some("png"));
    assert_eq!(source.info.width, Some(16));
    assert_eq!(source.info.height, Some(16));
    assert_eq!(source.info.duration_ms, Some(2000));
    assert_eq!(source.info.format, "mkv");
    assert_eq!(
        source.info.size,
        std::fs::metadata(&fixture).unwrap().len().to_string()
    );
    let public = serde_json::to_string(&source.info).unwrap();
    assert!(!public.contains(&root.to_string_lossy().to_string()));
    assert!(!public.contains("filename"));
    assert!(source.open().is_ok());
}
