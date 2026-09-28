//! The administrator kill switch `AMWAPOS_IMAGE_SEARCH=off`: discovery is
//! reported as disabled by the administrator, nothing is queued or claimed,
//! the settings cannot turn it back on, and manual pictures keep working.
//! (One test in this binary: it sets a process-wide environment variable.)
mod common;

use std::io::Cursor;

use base64::Engine;
use common::*;
use serde_json::json;

#[test]
fn the_environment_kill_switch_stops_discovery_but_not_manual_pictures() {
    std::env::set_var("AMWAPOS_IMAGE_SEARCH", "off");
    let e = env();
    let o = e.core.product_image_overview(&e.owner_token).unwrap();
    assert_eq!(
        (o["availability"].as_str(), o["environment_disabled"].as_bool(), o["enabled"].as_bool()),
        (Some("disabled_by_administrator"), Some(true), Some(true)),
        "{o}"
    );
    let pid = e.product("Kill switch", "6000000000055", 100, 50, 0);
    let st = e.core.product_image_state(&e.owner_token, &pid).unwrap();
    assert_eq!((st.auto_image_status.as_str(), st.discovery), ("not_attempted", "disabled_by_administrator"));
    // Even previously queued work is never claimed.
    e.core.db.write(|c| Ok(c.execute("UPDATE products SET auto_image_status='pending' WHERE product_id=?1", [&pid])?)).unwrap();
    assert!(e.core.auto_image_claim().unwrap().is_none());
    assert_eq!(e.core.product_image_find(&e.owner_token, &pid).unwrap_err().details.unwrap()["kind"], "auto_images_off");
    assert_eq!(e.core.product_image_backfill(&e.owner_token, None).unwrap_err().details.unwrap()["kind"], "auto_images_off");
    // The setting cannot override the administrator.
    let cfg = serde_json::from_value(json!({ "enabled": true, "region": "bh", "language": "en" })).unwrap();
    assert_eq!(e.core.product_image_configure(&e.owner_token, cfg, None).unwrap()["availability"], "disabled_by_administrator");
    // Manual pictures are unaffected.
    let mut img = image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255]));
    img.put_pixel(50, 50, image::Rgba([0, 0, 0, 255]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let st = e.core.product_image_upload(&e.owner_token, &pid, &base64::engine::general_purpose::STANDARD.encode(&png)).unwrap();
    assert_eq!(st.image_source.as_deref(), Some("manual"));
    for v in ["0", "false", "NO", " Off "] {
        std::env::set_var("AMWAPOS_IMAGE_SEARCH", v);
        assert!(amwapos_core::product_images::search_disabled_by_environment(), "{v}");
    }
    std::env::set_var("AMWAPOS_IMAGE_SEARCH", "on");
    assert!(!amwapos_core::product_images::search_disabled_by_environment());
}
