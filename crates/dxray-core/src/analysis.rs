//! From observed library names to a verdict. Pure: no paths, no filesystem.
//!
//! A verdict is a set: games link both `d3d11.dll` and `d3d12.dll` and pick at
//! run time, so the order is presentation, not prediction. `dxgi.dll` is
//! infrastructure, not a renderer. Where a signal came from travels with it,
//! because an import, a delay-import and a file in the directory are different
//! facts.

use dxray_pe::VersionInfo;

/// What was observed about one binary, with nothing interpreted yet. Names are
/// kept as spelled and matched case-insensitively.
#[derive(Debug, Default, Clone)]
pub struct Evidence {
    /// Names from the import table: resolved by the loader before `main` runs.
    pub imports: Vec<String>,
    /// Names from the delay-load table: resolved on the first call, if ever.
    pub delay_imports: Vec<String>,
    /// File names sitting in the same directory as the executable.
    pub neighbours: Vec<String>,
    /// Libraries in the same directory which this image imports, with what
    /// they import in turn. One link only: enough to see through a Unity stub.
    pub linked: Vec<Linked>,
}

/// One library reached by following an image's imports a single step, into
/// the image's own directory.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Linked {
    /// The library, spelled as the importing image spelled it.
    pub library: String,
    /// Names from that library's own import table.
    pub imports: Vec<String>,
    /// Names from that library's own delay-load table.
    pub delay_imports: Vec<String>,
}

/// Where a finding was observed, ordered strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// In the import table. The process cannot start without it.
    Import,
    /// In the delay-load table. Resolved on first use, so the code path may
    /// never be taken — real, but weaker than an import.
    DelayImport,
    /// Through a library beside the executable that it imports: weaker than a
    /// direct import, stronger than a directory listing.
    Linked,
    /// A file in the executable's directory. It says the file is there; it does
    /// not say the executable ever loads it.
    Neighbour,
}

impl Source {
    /// The stable spelling used in output. Callers render this rather than
    /// `Debug`, which is free to change.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::DelayImport => "delay-import",
            Self::Linked => "linked",
            Self::Neighbour => "neighbour",
        }
    }
}

/// One observation: a library name, how it was observed, and — once the
/// verdict has been stamped — what the file of that name says its version is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    /// The name as it was spelled in the file or directory.
    pub library: String,
    pub source: Source,
    /// The version of the file this signal names. [`analyse`] leaves it unread;
    /// [`evidence::stamp_versions`](crate::evidence::stamp_versions) reads it.
    pub version: FileVersion,
}

impl Signal {
    /// The one rendering of a signal, shared by every surface:
    /// `nvngx_dlss.dll (neighbour, 310.2.1.0)`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.version.label() {
            Some(version) => format!("{} ({}, {version})", self.library, self.source.as_str()),
            None => format!("{} ({})", self.library, self.source.as_str()),
        }
    }
}

/// What reading a version out of the file a signal names produced. `0.0.0.0`
/// is a version, not a missing one, and a copy resolved from `System32`
/// describes Windows, so it is never read.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FileVersion {
    /// Nobody has looked, which is not [`Self::Elsewhere`]: that one means
    /// somebody checked.
    #[default]
    Unread,
    /// Looked, and no file of that name sits beside the executable: the normal
    /// answer for `d3d12.dll`.
    Elsewhere,
    /// The file is there and carries no version resource.
    Unstamped,
    /// The file is there and could not be read as a PE image. A zero-byte
    /// placeholder and a genuine DLL are not the same thing to report.
    Unreadable,
    /// The file is there and its version resource says this.
    Stamped(VersionInfo),
}

impl FileVersion {
    /// The stable spelling used in output, so the JSON and the two human
    /// surfaces cannot end up naming these states differently.
    #[must_use]
    pub fn state(self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Elsewhere => "elsewhere",
            Self::Unstamped => "unstamped",
            Self::Unreadable => "unreadable",
            Self::Stamped(_) => "stamped",
        }
    }

    /// What a human report puts after the source, or `None` when there is no
    /// claim. The file version, which identifies a build; both are in the JSON.
    #[must_use]
    pub fn label(self) -> Option<String> {
        match self {
            // The only silence. Nobody looked, so there is nothing to say —
            // and a reader never sees this, because both surfaces stamp.
            Self::Unread => None,
            // Said out loud, so it cannot pass for a version nobody annotated.
            Self::Elsewhere => Some("no local copy".to_owned()),
            Self::Unstamped => Some("no version".to_owned()),
            Self::Unreadable => Some("version unreadable".to_owned()),
            Self::Stamped(info) => Some(info.file.to_string()),
        }
    }

    /// The two versions, for a consumer that wants to compare rather than
    /// print.
    #[must_use]
    pub fn info(self) -> Option<VersionInfo> {
        if let Self::Stamped(info) = self {
            Some(info)
        } else {
            None
        }
    }
}

/// One thing concluded, and every observation that supports it (at least one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// What was concluded: `"Direct3D 12"`, `"DLSS Frame Generation"`, or for a
    /// local override the name of the overridden system DLL.
    pub name: String,
    /// Every observation behind it, strongest source first.
    pub signals: Vec<Signal>,
    /// Tie-break position within its own list. Not public: it is an artefact of
    /// how the list is ordered, not a fact about the binary.
    rank: u8,
}

impl Finding {
    /// The strongest source behind this finding.
    #[must_use]
    pub fn strength(&self) -> Source {
        // Signals are sorted strongest first; the fallback is the weakest source.
        self.signals
            .first()
            .map_or(Source::Neighbour, |signal| signal.source)
    }
}

/// Everything concluded about one binary, in four lists for four questions.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Graphics APIs the image can reach. May legitimately hold several.
    pub renderers: Vec<Finding>,
    /// Shared plumbing that constrains the answer without giving it: `dxgi.dll`.
    pub infrastructure: Vec<Finding>,
    /// Upscalers, frame generation and vendor SDKs.
    pub features: Vec<Finding>,
    /// System DLL names found as files in the executable's directory. Not a
    /// graphics signal — see [`LOCAL_OVERRIDE`].
    pub local_overrides: Vec<Finding>,
}

impl Verdict {
    /// True when nothing at all was recognised.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.renderers.is_empty()
            && self.infrastructure.is_empty()
            && self.features.is_empty()
            && self.local_overrides.is_empty()
    }

    /// The one-line answer, worded so it cannot be read as more certain than it
    /// is. A `dxgi.dll`-only binary is not reported as "DirectX".
    #[must_use]
    pub fn headline(&self) -> String {
        if self.renderers.is_empty() {
            // Built from what was found: a `dxcore.dll`-only file names no DXGI.
            if self.infrastructure.is_empty() {
                return "no graphics API determined".to_owned();
            }
            let found: Vec<&str> = self
                .infrastructure
                .iter()
                .map(|finding| finding.name.as_str())
                .collect();
            return format!(
                "{} present, specific API not determined",
                found.join(" and ")
            );
        }

        let names: Vec<&str> = self.renderers.iter().map(|f| f.name.as_str()).collect();
        // A renderer known only from the directory is a weaker claim.
        let hedge = if self
            .renderers
            .iter()
            .all(|f| f.strength() == Source::Neighbour)
        {
            " (from files in the directory only)"
        } else {
            ""
        };
        if let [only] = names.as_slice() {
            return format!("{only}{hedge}");
        }
        format!(
            "{} (all linked; selected at run time){hedge}",
            names.join(" or ")
        )
    }
}

/// The name given to a system DLL found as a file next to the executable.
pub const LOCAL_OVERRIDE: &str = "local copy of a system DLL";

/// Reads `evidence` and concludes what it supports.
///
/// Every list is ordered by evidence strength, then by a fixed table position:
/// newest generation first for renderers, most specific first for features.
/// It is a presentation order, not a prediction of the API in use.
#[must_use]
pub fn analyse(evidence: &Evidence) -> Verdict {
    let mut renderers = Collector::default();
    let mut infrastructure = Collector::default();
    let mut features = Collector::default();
    let mut local_overrides = Collector::default();

    for (names, source) in [
        (&evidence.imports, Source::Import),
        (&evidence.delay_imports, Source::DelayImport),
    ] {
        for name in names {
            let lower = name.to_ascii_lowercase();
            if let Some((label, rank)) = renderer(&lower) {
                renderers.add(label, rank, name, source);
            } else if let Some((label, rank)) = shared(&lower) {
                infrastructure.add(label, rank, name, source);
            } else if let Some((label, rank)) = feature(&lower) {
                features.add(label, rank, name, source);
            }
        }
    }

    // One link on. The signal names the intermediary (`UnityPlayer.dll`), the
    // file the import table spelled. Renderers and infrastructure only: a
    // feature DLL is loaded by name and appears in no import table.
    for link in &evidence.linked {
        for name in link.imports.iter().chain(&link.delay_imports) {
            let lower = name.to_ascii_lowercase();
            if let Some((label, rank)) = renderer(&lower) {
                renderers.add(label, rank, &link.library, Source::Linked);
            } else if let Some((label, rank)) = shared(&lower) {
                infrastructure.add(label, rank, &link.library, Source::Linked);
            }
        }
    }

    // A directory that is itself part of Windows says nothing about the binary.
    if !looks_like_system_directory(&evidence.neighbours) {
        for name in &evidence.neighbours {
            let lower = name.to_ascii_lowercase();
            // Not an injector: see `D3D12_CORE`. Label and rank come from `renderer`.
            if lower == D3D12_CORE
                && let Some((label, rank)) = renderer(&lower)
            {
                renderers.add(label, rank, name, Source::Neighbour);
            } else if is_shadowable(&lower) {
                local_overrides.add(LOCAL_OVERRIDE, proxy_rank(&lower), name, Source::Neighbour);
            } else if let Some((label, rank)) = feature(&lower) {
                // Features arrive this way: a DLSS runtime is loaded by name.
                features.add(label, rank, name, Source::Neighbour);
            }
        }
    }

    Verdict {
        renderers: renderers.finish(),
        infrastructure: infrastructure.finish(),
        features: features.finish(),
        local_overrides: local_overrides.finish(),
    }
}

/// Shared infrastructure for Direct3D 10, 11 and 12. Importing it proves the
/// image is not OpenGL-only and not Vulkan-only, and proves nothing else.
const DXGI: &str = "dxgi.dll";

/// The newer adapter-enumeration layer, filling the same slot as [`DXGI`].
const DXCORE: &str = "dxcore.dll";

/// Shared plumbing, if this is one of its entry points: it rules out
/// OpenGL-only and Vulkan-only and says nothing else.
fn shared(lower: &str) -> Option<(&'static str, u8)> {
    match lower {
        DXGI => Some(("DXGI", 1)),
        DXCORE => Some(("DXCore", 2)),
        _ => None,
    }
}

/// The Direct3D 12 Agility SDK runtime. Beside a game it is a Direct3D 12
/// signal, not an injector: `d3d12.dll` loads it by name.
const D3D12_CORE: &str = "d3d12core.dll";

/// A graphics API, if this is one of its entry points, with its tie-break
/// position (newest first).
fn renderer(lower: &str) -> Option<(&'static str, u8)> {
    let hit = match lower {
        // `d3d12core.dll` as an *import* is unusual but unambiguous; as a
        // neighbouring file it is handled separately, in `analyse`.
        "d3d12.dll" | D3D12_CORE => ("Direct3D 12", 1),
        "vulkan-1.dll" => ("Vulkan", 2),
        // The `_1` to `_4` interfaces are Direct3D 11, and so is `d3d11on12.dll`:
        // the code is Direct3D 11 even when the device underneath is 12.
        "d3d11.dll" | "d3d11_1.dll" | "d3d11_2.dll" | "d3d11_3.dll" | "d3d11_4.dll"
        | "d3d11on12.dll" => ("Direct3D 11", 3),
        // `d3d10_1core.dll` is not a typo for `d3d10core.dll`: both ship, and a
        // table with only one of them misses the other.
        "d3d10.dll" | "d3d10_1.dll" | "d3d10core.dll" | "d3d10_1core.dll" => ("Direct3D 10", 4),
        "opengl32.dll" => ("OpenGL", 5),
        "d3d9.dll" => ("Direct3D 9", 6),
        "d3d8.dll" => ("Direct3D 8", 7),
        "ddraw.dll" => ("DirectDraw", 8),
        _ => return None,
    };
    Some(hit)
}

/// The neural-rendering runtime NVIDIA ships as DLSS 5, alongside the other
/// three runtimes, and ranked above them as the narrowest.
///
/// Its version resource does not identify the build: NVIDIA's and the
/// community's variants for RTX 40 and 20/30 all report `310.8.0.0`, and only
/// the CUDA fatbin tells them apart. The name comes from public reporting; no
/// copy has been read by this code yet.
const DLSSNR: &str = "nvngx_dlssnr.dll";

/// A vendor feature, if this is one of its libraries. Ranked by specificity:
/// a library that names a feature outranks one that enables a family.
fn feature(lower: &str) -> Option<(&'static str, u8)> {
    let hit = match lower {
        // Named for the feature, not "DLSS 5": the resource cannot support a
        // release number.
        DLSSNR => ("DLSS Neural Rendering", 1),
        "nvngx_dlssg.dll" => ("DLSS Frame Generation", 2),
        "nvngx_dlssd.dll" => ("DLSS Ray Reconstruction", 3),
        "nvngx_dlss.dll" => ("DLSS Super Resolution", 4),
        "libxess.dll" | "libxess_dx11.dll" => ("XeSS", 6),
        // The driver installs `_nvngx.dll`; the leading underscore is not a
        // typo and a match that misses it misses the system copy entirely.
        "nvngx.dll" | "_nvngx.dll" => ("NGX loader", 8),
        "nvapi64.dll" | "nvapi.dll" => ("NVAPI", 9),
        _ => {
            if lower.starts_with("ffx_fsr") || lower.starts_with("amd_fidelityfx") {
                ("FSR", 5)
            } else if lower.starts_with("sl.")
                && lower.rsplit_once('.').is_some_and(|(_, ext)| ext == "dll")
            {
                // One interposer and a plugin per feature: one finding, several
                // signals.
                ("NVIDIA Streamline", 7)
            } else {
                return None;
            }
        }
    };
    Some(hit)
}

/// Leading substrings of DLL names worth pulling out of the crowd, beyond what
/// the tables recognise. They decide where a name is printed and claim nothing,
/// so a false positive costs a line of attention.
const GRAPHICS_PREFIXES: &[&str] = &[
    "amd_ags", "amdxc", "d3d", "ddraw", "dstorage", "dxcore", "dxgi", "dxil", "ffx_", "gfsdk",
    "glide", "igdext", "libxess", "nvapi", "nvngx", "nvofapi", "openvr", "openxr", "opengl", "sl.",
    "vulkan", "xess",
];

/// True when `name` is a library worth separating from the api-set stubs in a
/// printed import list: every name an import table can make a finding of, and
/// the prefixes above. `PROXYABLE` is not asked; it is about directory
/// listings, and most of it is audio, input and HTTP.
#[must_use]
pub fn is_graphics_related(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    renderer(&lower).is_some()
        || shared(&lower).is_some()
        || feature(&lower).is_some()
        || GRAPHICS_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// System DLL names that a local file can shadow, the way `ReShade`,
/// `SpecialK` and `OptiScaler` attach. A proxy is a reportable fact, not a
/// renderer signal: it is named after whatever export its injector chose.
const PROXYABLE: &[&str] = &[
    "dxgi.dll",
    "d3d12.dll",
    "d3d11.dll",
    "d3d10.dll",
    "d3d9.dll",
    "d3d8.dll",
    "ddraw.dll",
    "opengl32.dll",
    // Named here because this list is what a reader checks, and its order is
    // the order the report prints.
    "vulkan-1.dll",
    "dinput8.dll",
    "dinput.dll",
    "dsound.dll",
    "winmm.dll",
    "version.dll",
    "winhttp.dll",
    "xinput1_3.dll",
    "xinput1_4.dll",
];

/// True when a file of this name beside an executable shadows a system DLL:
/// anything in [`PROXYABLE`] or [`renderer`]. [`D3D12_CORE`] is excluded by the
/// caller.
fn is_shadowable(lower: &str) -> bool {
    PROXYABLE.contains(&lower) || renderer(lower).is_some()
}

/// Position of `lower` in [`PROXYABLE`], used only to order the reported list.
/// A shadowable name that is not in that list sorts last rather than first.
fn proxy_rank(lower: &str) -> u8 {
    PROXYABLE
        .iter()
        .position(|p| *p == lower)
        .map_or(u8::MAX, |at| u8::try_from(at).unwrap_or(u8::MAX))
}

/// Core Windows DLLs, never copied into a game directory.
const OS_CORE: &[&str] = &[
    "advapi32.dll",
    "gdi32.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "ntdll.dll",
    "ole32.dll",
    "shell32.dll",
    "user32.dll",
    "win32u.dll",
];

/// How many of [`OS_CORE`] mark a directory as Windows' own: a mod loader might
/// ship one, no game ships three.
const SYSTEM_DIRECTORY_HITS: usize = 3;

/// True when the neighbours look like `System32` or `SysWOW64`, where every
/// system DLL would otherwise read as an injector.
fn looks_like_system_directory(neighbours: &[String]) -> bool {
    let mut seen = 0usize;
    for name in neighbours {
        let lower = name.to_ascii_lowercase();
        if OS_CORE.contains(&lower.as_str()) {
            seen += 1;
            if seen >= SYSTEM_DIRECTORY_HITS {
                return true;
            }
        }
    }
    false
}

/// Accumulates findings, merging repeat observations into the one they support.
#[derive(Default)]
struct Collector {
    findings: Vec<Finding>,
}

impl Collector {
    fn add(&mut self, name: &str, rank: u8, library: &str, source: Source) {
        let signal = Signal {
            library: library.to_owned(),
            source,
            // Not looked up here and not looked up by anything this function
            // can reach: `analyse` does not open files.
            version: FileVersion::Unread,
        };
        if let Some(finding) = self.findings.iter_mut().find(|f| f.name == name) {
            // The same library can appear twice in one table in a malformed or
            // merged image; recording it twice would inflate the evidence.
            if !finding
                .signals
                .iter()
                .any(|s| s.source == source && s.library.eq_ignore_ascii_case(library))
            {
                finding.signals.push(signal);
            }
            return;
        }
        self.findings.push(Finding {
            name: name.to_owned(),
            signals: vec![signal],
            rank,
        });
    }

    fn finish(mut self) -> Vec<Finding> {
        for finding in &mut self.findings {
            finding.signals.sort_by(|a, b| {
                a.source
                    .cmp(&b.source)
                    .then_with(|| a.library.cmp(&b.library))
            });
        }
        // Stable, so two findings with the same strength and rank keep the
        // order they were observed in rather than swapping between runs.
        self.findings.sort_by_key(|f| (f.strength(), f.rank));
        self.findings
    }
}

#[cfg(test)]
mod tests;
