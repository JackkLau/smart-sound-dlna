//! Optional read-only end-to-end discovery check on an actual LAN.
use xiaomi_dlna_desktop::dlna;

#[tokio::test]
#[ignore = "requires a live DLNA receiver and DLNA_TEST_RENDERER"]
async fn verify_visible_renderer_on_lan() {
    let name = std::env::var("DLNA_TEST_RENDERER")
        .expect("set DLNA_TEST_RENDERER to the expected device name");
    assert!(!name.is_empty());
    let report = dlna::discover().await.unwrap();
    let names: Vec<_> = report.devices.iter().map(|d| d.name.as_str()).collect();
    println!("Discovered {names:?}");
    assert!(
        names.iter().any(|n| n.contains(&name)),
        "missing {name}, discovery warnings: {:?}",
        report.warnings
    );
}
