//! What Heroic hands Proton for one game: the prefix, the environment and the
//! `SteamAppId` Proton sees. Game settings in `GamesConfig` override the
//! defaults in `config.json`, key by key, as Heroic's own loader does.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{Json, Store};

/// One game's Proton launch, as Heroic would make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// `winePrefix`, which Heroic passes as `STEAM_COMPAT_DATA_PATH`.
    pub prefix: PathBuf,
    /// The game's environment options, then the NVAPI switch Heroic sets over
    /// them.
    pub environment: Vec<(String, String)>,
    /// The `SteamAppId` Proton sees, or why it is not known.
    pub appid: Result<String, String>,
}

/// Why a game has no Proton launch to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoLaunch {
    /// The runner is not Proton, so no Proton policy applies.
    NotProton(String),
    /// A settings file could not be read, or names no runner or prefix.
    Unknown(String),
}

/// Heroic's own default for `autoInstallDxvkNvapi` on Linux.
const NVAPI_BY_DEFAULT: bool = true;

/// The launch of `app_name`, from `store`, under the Heroic configuration
/// `root`.
///
/// # Errors
///
/// [`NoLaunch::NotProton`] for another runner; [`NoLaunch::Unknown`] when a
/// settings file cannot be read or names no runner or prefix.
pub fn launch(root: &Path, store: Store, app_name: &str) -> Result<Launch, NoLaunch> {
    let game = read(&root.join("GamesConfig").join(format!("{app_name}.json")))
        .map_err(NoLaunch::Unknown)?;
    let global = read(&root.join("config.json")).map_err(NoLaunch::Unknown)?;
    let settings = Settings {
        game: game.as_ref().and_then(|file| file.get(app_name)),
        defaults: global.as_ref().and_then(|file| file.get("defaultSettings")),
    };

    let runner = settings
        .get("wineVersion")
        .ok_or_else(|| unknown("Heroic's settings name no runner for this game"))?;
    match runner.get("type").and_then(Json::string) {
        Some("proton") => {}
        Some(other) => return Err(NoLaunch::NotProton(other.to_owned())),
        None => return Err(unknown("the runner in Heroic's settings has no type")),
    }
    let prefix = settings
        .get("winePrefix")
        .and_then(Json::string)
        .filter(|prefix| !prefix.is_empty())
        .ok_or_else(|| unknown("Heroic's settings name no prefix for this game"))?;

    Ok(Launch {
        prefix: expand_home(root, prefix)?,
        environment: environment(&settings),
        appid: if settings.get("disableUMU").and_then(Json::bool) == Some(true) {
            Ok("0".to_owned())
        } else {
            umu_appid(root, store, app_name)
        },
    })
}

/// A game's settings over the global defaults.
struct Settings<'a> {
    game: Option<&'a Json>,
    defaults: Option<&'a Json>,
}

impl<'a> Settings<'a> {
    fn get(&self, key: &str) -> Option<&'a Json> {
        self.game
            .and_then(|game| game.get(key))
            .or_else(|| self.defaults.and_then(|defaults| defaults.get(key)))
    }
}

/// The environment options, quotes stripped as Heroic strips them, then the
/// switch `autoInstallDxvkNvapi` sets. Heroic applies that switch last, so it
/// wins over an option with the same name.
fn environment(settings: &Settings<'_>) -> Vec<(String, String)> {
    let mut environment: Vec<(String, String)> = settings
        .get("enviromentOptions")
        .and_then(Json::array)
        .unwrap_or_default()
        .iter()
        .filter_map(|option| {
            let key = option.get("key").and_then(Json::string)?;
            let value = option.get("value").and_then(Json::string)?;
            Some((key.to_owned(), unquote(value).to_owned()))
        })
        .collect();
    let nvapi = settings
        .get("autoInstallDxvkNvapi")
        .and_then(Json::bool)
        .unwrap_or(NVAPI_BY_DEFAULT);
    if nvapi {
        environment.push(("PROTON_ENABLE_NVAPI".to_owned(), "1".to_owned()));
        environment.push(("DXVK_NVAPI_ALLOW_OTHER_DRIVERS".to_owned(), "1".to_owned()));
    } else {
        environment.push(("PROTON_DISABLE_NVAPI".to_owned(), "1".to_owned()));
    }
    environment
}

/// Heroic's `removeQuoteIfNecessary`: a value wrapped in double quotes loses
/// every leading and trailing one.
fn unquote(value: &str) -> &str {
    if value.len() > 1 && value.starts_with('"') && value.ends_with('"') {
        value.trim_matches('"')
    } else {
        value
    }
}

/// The id umu turns into `SteamAppId`: what follows `umu-` in the game's umu
/// id, which Heroic caches in `store_cache/umu.json`. With no id, umu runs the
/// game as `umu-default`.
fn umu_appid(root: &Path, store: Store, app_name: &str) -> Result<String, String> {
    let path = root.join("store_cache/umu.json");
    let cache = read(&path)?;
    let key = format!("{}_{app_name}", store.runner());
    match cache.as_ref().and_then(|cache| cache.get(&key)) {
        Some(Json::String(id)) => Ok(id
            .strip_prefix("umu-")
            .filter(|rest| {
                !rest.is_empty() && rest.chars().all(|c| c.is_alphanumeric() || c == '_')
            })
            .unwrap_or("0")
            .to_owned()),
        Some(Json::Other) => Ok("default".to_owned()),
        _ => Err(
            "Heroic has not looked this game up in the umu database yet, and the umu id it \
             finds becomes the SteamAppId Proton reads its lists by"
                .to_owned(),
        ),
    }
}

/// `winePrefix` with its first `~` replaced by the home the configuration
/// sits in, as Heroic replaces it.
fn expand_home(root: &Path, prefix: &str) -> Result<PathBuf, NoLaunch> {
    let prefix = if prefix.contains('~') {
        let home = [
            ".config/heroic",
            ".var/app/com.heroicgameslauncher.hgl/config/heroic",
        ]
        .iter()
        .find_map(|suffix| strip_suffix(root, Path::new(suffix)))
        .ok_or_else(|| unknown("the prefix starts at a home this configuration does not name"))?;
        PathBuf::from(prefix.replacen('~', &home.to_string_lossy(), 1))
    } else {
        PathBuf::from(prefix)
    };
    if prefix.is_absolute() {
        Ok(prefix)
    } else {
        Err(unknown("Heroic's settings name a relative prefix"))
    }
}

fn strip_suffix<'a>(path: &'a Path, suffix: &Path) -> Option<&'a Path> {
    let mut path = path;
    for part in suffix.components().rev() {
        if path.file_name()? != part.as_os_str() {
            return None;
        }
        path = path.parent()?;
    }
    Some(path)
}

/// A settings file, `None` when it is not there.
fn read(path: &Path) -> Result<Option<Json>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    Json::parse(&text)
        .map(Some)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn unknown(why: &str) -> NoLaunch {
    NoLaunch::Unknown(why.to_owned())
}

#[cfg(test)]
mod tests;
