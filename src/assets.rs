//! Icons and images compiled into the binary.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! embed {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/res/", $path)) as &[u8])),*]
    };
}

static ASSETS: &[(&str, &[u8])] = embed![
    "pizarra128.png",
    "icons/alpha.svg",
    "icons/circle_by_center_and_point.svg",
    "icons/circle_by_three_points.svg",
    "icons/ellipse_by_foci_and_point.svg",
    "icons/eraser.svg",
    "icons/free_grid.svg",
    "icons/grid.svg",
    "icons/line.svg",
    "icons/menu.svg",
    "icons/polygon.svg",
    "icons/rectangle.svg",
    "icons/redo.svg",
    "icons/thickness.svg",
    "icons/undo.svg",
    "icons/zoom_home.svg",
    "icons/zoom_in.svg",
    "icons/zoom_out.svg",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS.iter().find(|(name, _)| *name == path).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ASSETS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
