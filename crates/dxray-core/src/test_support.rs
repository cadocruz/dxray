//! Test fixtures for consumers of the launcher inventory, on real adapters.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{Catalogue, Inventory, Launcher, Libraries, Origin, heroic, steam};

static NEXT_DIRECTORY: AtomicU32 = AtomicU32::new(0);

/// A Steam and Heroic tree yielding an entry, a note and a problem: valid and
/// corrupt Steam manifests, and a duplicated Heroic GOG record.
pub struct SteamHeroicFixture {
    _directory: TempDir,
    steam: AtRoot,
    heroic: AtRoot,
}

impl SteamHeroicFixture {
    /// Builds the shared Steam and Heroic fixture from their on-disk formats.
    #[must_use]
    pub fn new() -> Self {
        let directory = TempDir::new("inventory-parity");
        let steam_root = directory.dir("steam");
        directory.dir("steam/steamapps/common/dota 2 beta");
        directory.write(
            "steam/steamapps/appmanifest_570.acf",
            "\"AppState\" { \"appid\" \"570\" \"name\" \"Dota 2\" \"installdir\" \"dota 2 beta\" }",
        );
        directory.write(
            "steam/steamapps/appmanifest_999.acf",
            "\"AppState\" { \"appid\" \"999\" \"name\" \"Broken",
        );

        let heroic_root = directory.dir("heroic");
        let hades = directory.dir("games/Hades");
        directory.write(
            "heroic/store_cache/gog_install_info.json",
            &format!(
                r#"{{"Hades":{{"game":{{"title":"Hades"}},"install":{{"install_path":"{}"}}}}}}"#,
                hades.display()
            ),
        );
        directory.write(
            "heroic/store_cache/gog_library.json",
            r#"{"library":[{"app_name":"Hades","title":"Hades","is_installed":true}]}"#,
        );

        Self {
            _directory: directory,
            steam: AtRoot::new(&steam::STEAM, steam_root),
            heroic: AtRoot::new(&heroic::HEROIC, heroic_root),
        }
    }

    /// The real adapters, restricted only to the fixture's roots.
    #[must_use]
    pub fn launchers(&self) -> [&dyn Launcher; 2] {
        [&self.steam, &self.heroic]
    }

    /// The complete contract emitted by the fixture's real adapter walk.
    #[must_use]
    pub fn inventory(&self) -> Inventory {
        Inventory::collect(&self.launchers())
    }
}

impl Default for SteamHeroicFixture {
    fn default() -> Self {
        Self::new()
    }
}

struct AtRoot {
    adapter: &'static dyn Launcher,
    root: PathBuf,
}

impl AtRoot {
    fn new(adapter: &'static dyn Launcher, root: PathBuf) -> Self {
        Self { adapter, root }
    }
}

impl Launcher for AtRoot {
    fn origin(&self) -> Origin {
        self.adapter.origin()
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        self.roots()
    }

    fn libraries(&self, root: &Path) -> Libraries {
        self.adapter.libraries(root)
    }

    fn games(&self, library: &Path) -> Catalogue {
        self.adapter.games(library)
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let next = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("dxray-{tag}-{}-{next}", std::process::id()));
        fs::create_dir_all(&path).expect("create fixture directory");
        Self(path)
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("fixture file parent"))
            .expect("create fixture parents");
        fs::write(&path, contents).expect("write fixture file");
        path
    }

    fn dir(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(&path).expect("create fixture directory");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
