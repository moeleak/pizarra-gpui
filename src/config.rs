//! Loads the pizarra configuration.
//!
//! Uses the same lookup as the GTK frontend so both share one file:
//! `$PIZARRA_CONFIG` if set, otherwise `<config dir>/pizarra/config.toml`
//! (e.g. `~/.config/pizarra/config.toml` on Linux). A default file is written
//! when none exists so the options are easy to discover.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use directories::ProjectDirs;
use pizarra::config::Config;

/// Everything read from the configuration file.
pub struct Settings {
    /// The engine's options, shared with pizarra-gtk.
    pub config: Config,
    /// Whether pen pressure varies the stroke width when the app starts. Only
    /// this frontend reads it; the engine's parser ignores unknown keys.
    pub pen_pressure: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { config: Config::default(), pen_pressure: true }
    }
}

pub fn read() -> Settings {
    match config_path().and_then(|path| read_or_create(&path)) {
        Ok(settings) => settings,
        Err(err) => {
            log::warn!("using the default configuration: {err:#}");
            Settings::default()
        }
    }
}

fn config_path() -> Result<PathBuf> {
    if let Some(value) = env::var_os("PIZARRA_CONFIG") {
        return Ok(PathBuf::from(value));
    }

    let dirs = ProjectDirs::from("tk", "categulario", "pizarra")
        .context("could not determine the configuration directory for this platform")?;

    Ok(dirs.config_dir().join("config.toml"))
}

fn read_or_create(path: &Path) -> Result<Settings> {
    if path.is_file() {
        let contents = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        return parse(&contents).with_context(|| format!("parsing {}", path.display()));
    }

    let settings = Settings::default();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(path, to_toml(&settings)?).with_context(|| format!("writing {}", path.display()))?;
    log::info!("wrote a default configuration to {}", path.display());

    Ok(settings)
}

fn parse(contents: &str) -> Result<Settings> {
    let config = toml::from_str(contents)?;
    let table: toml::Table = contents.parse()?;
    let pen_pressure = match table.get("pen_pressure") {
        Some(value) => value.as_bool().context("pen_pressure must be true or false")?,
        None => true,
    };
    Ok(Settings { config, pen_pressure })
}

/// Serializes the configuration.
///
/// pizarra 3.0.1 formats colors without zero padding (black becomes `#000FF`),
/// which its own parser then rejects, so the color fields are rewritten here as
/// proper `#RRGGBBAA` strings.
fn to_toml(settings: &Settings) -> Result<String> {
    let config = &settings.config;
    let mut table = toml::Table::try_from(config)?;
    for (key, color) in [("background_color", config.background_color), ("stroke_color", config.stroke_color)] {
        let hex = format!("#{:02X}{:02X}{:02X}{:02X}", color.r, color.g, color.b, color.a);
        table.insert(key.into(), toml::Value::String(hex));
    }
    table.insert("pen_pressure".into(), toml::Value::Boolean(settings.pen_pressure));
    Ok(toml::to_string(&table)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pizarra::point::Unit;

    #[test]
    fn default_config_round_trips_through_toml() {
        let text = to_toml(&Settings::default()).unwrap();
        let settings = parse(&text).unwrap();
        let config = settings.config;
        assert_eq!(config.zoom_factor, Config::default().zoom_factor);
        assert_eq!(config.background_color, Config::default().background_color);
        assert_eq!(config.stroke_color, Config::default().stroke_color);
        assert!(settings.pen_pressure);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let settings = parse("background_color = '#cebada'\nthickness = 5.0\n").unwrap();
        let config = settings.config;
        assert_eq!(config.background_color.css(), "#CEBADA");
        assert_eq!(config.thickness.val(), 5.0);
        assert_eq!(config.scroll_factor, Config::default().scroll_factor);
        assert!(settings.pen_pressure);
    }

    #[test]
    fn pen_pressure_can_be_turned_off() {
        assert!(!parse("pen_pressure = false\n").unwrap().pen_pressure);
        assert!(parse("pen_pressure = 0\n").is_err());
    }

    #[test]
    fn creates_the_file_when_missing() {
        let dir = env::temp_dir().join(format!("pizarra-gpui-test-{}", std::process::id()));
        let path = dir.join("nested").join("config.toml");
        let _ = fs::remove_dir_all(&dir);

        let settings = read_or_create(&path).unwrap();
        assert!(path.is_file());
        assert_eq!(read_or_create(&path).unwrap().config.zoom_factor, settings.config.zoom_factor);

        fs::remove_dir_all(&dir).unwrap();
    }
}
