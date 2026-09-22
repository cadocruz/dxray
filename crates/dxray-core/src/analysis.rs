//! From observed library names to a verdict. Pure: no paths, no filesystem.
//!
//! Three ideas shape everything below, and each of them exists because the
//! obvious implementation gets a real binary wrong.
//!
//! **A verdict is a set.** Shipped games link `d3d11.dll` and `d3d12.dll` and
//! pick between them at run time, after asking the driver what it supports.
//! Collapsing that to one answer would be a confident lie, so the renderer
//! result is a set and the order is presentation, not prediction.
//!
//! **`dxgi.dll` is not a renderer.** It is the swapchain and adapter layer
//! shared by Direct3D 10, 11 and 12. On its own it rules OpenGL and Vulkan out
//! and says nothing else, which is why it is modelled as infrastructure and
//! reported as "specific API not determined" rather than as "DirectX".
//!
//! **Where a signal came from is part of the signal.** A load-time import, a
//! delay-loaded import and a file sitting in the directory are three different
//! facts with three different strengths, so provenance travels attached to
//! every finding rather than being flattened away.
//!
//! [`FileVersion`] is the one thing here that describes a file rather than a
//! name, and it is still not a judgement: [`analyse`] leaves every signal at
//! [`FileVersion::Unread`] and never opens anything. The type lives here
//! because it hangs off a [`Signal`]; the reading lives in
//! [`evidence`](crate::evidence), where all the reading lives.

use dxray_pe::VersionInfo;

/// What was observed about one binary, with nothing interpreted yet.
///
/// Built by [`Evidence::from_executable`](crate::evidence), or by hand in a
/// test. Every field is a list of library or file names exactly as they were
/// spelled where they were found: PE import names are famously inconsistent
/// about case (`DXGI.dll`, `dxgi.dll`, `D3D12.dll` all occur in shipped
/// binaries), and matching is done case-insensitively rather than by
/// normalising the input, so the report can still quote what the file said.
#[derive(Debug, Default, Clone)]
pub struct Evidence {
    /// Names from the import table: resolved by the loader before `main` runs.
    pub imports: Vec<String>,
    /// Names from the delay-load table: resolved on the first call, if ever.
    pub delay_imports: Vec<String>,
    /// File names sitting in the same directory as the executable.
    pub neighbours: Vec<String>,
    /// Libraries in that same directory which this image imports, each with the
    /// names *it* imports in turn. One link only.
    ///
    /// This exists because a Unity game's executable is a stub: it imports
    /// `UnityPlayer.dll` and nothing graphical, and the renderer is one link
    /// further on. Without this the verdict on such a binary is "no graphics
    /// API determined", which is false about the game and, worse, disagrees
    /// with the ranking in [`game`](crate::game) — which does follow the link
    /// and scores the binary on what it found there.
    pub linked: Vec<Linked>,
}

/// One library reached by following an image's imports a single step, into the
/// directory the image sits in.
///
/// Only the same directory: a library resolved out of `System32` describes
/// Windows rather than the game, and one three folders away is not a name the
/// loader would resolve this way at all.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Linked {
    /// The library, spelled as the importing image spelled it.
    pub library: String,
    /// Names from that library's own import table.
    pub imports: Vec<String>,
    /// Names from that library's own delay-load table.
    pub delay_imports: Vec<String>,
}

/// Where a finding was observed. Ordered strongest first, which is what makes
/// `sort_by_key` below put load-time evidence above a guess from a directory
/// listing without needing a comparator spelled out at every use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// In the import table. The process cannot start without it.
    Import,
    /// In the delay-load table. Resolved on first use, so the code path may
    /// never be taken — real, but weaker than an import.
    DelayImport,
    /// Reached through a library in the executable's own directory which the
    /// executable imports. Weaker than importing the API directly, because the
    /// intermediary decides whether that code path is ever taken; stronger than
    /// a directory listing, because the executable demonstrably loads the
    /// intermediary.
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
    /// The version of the file this signal names, if one sits beside the
    /// executable and it was looked up.
    ///
    /// [`analyse`] never sets anything but [`FileVersion::Unread`]: it does not
    /// open files. Filling this in is
    /// [`evidence::stamp_versions`](crate::evidence::stamp_versions), which is
    /// the one place in the project that reads a version off disk for a library
    /// the verdict named.
    pub version: FileVersion,
}

impl Signal {
    /// The one rendering of a signal there is: `nvngx_dlss.dll (neighbour,
    /// 310.2.1.0)`.
    ///
    /// Both surfaces call this rather than each formatting a library, a source
    /// and a version their own way. Two spellings of one line is how the
    /// verdict and the ranking came to disagree about a Unity stub, and a
    /// version is exactly the sort of detail one of the two would quietly stop
    /// printing.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.version.label() {
            Some(version) => format!("{} ({}, {version})", self.library, self.source.as_str()),
            None => format!("{} ({})", self.library, self.source.as_str()),
        }
    }
}

/// What reading a version out of the file a signal names produced.
///
/// Five outcomes and not one of them is "absent". The question a person asks
/// before swapping a DLL is *which build is this*, and every way of failing to
/// answer it is a different fact:
///
/// * `0.0.0.0` is a **version**, not a missing one. Twenty-one files on one
///   ordinary Windows install carry a version resource that is all zeroes;
///   [`dxray_pe`] reports them as the number they are, and folding them into
///   "no version" here would undo that on the way out.
/// * A file with no version resource at all is a different answer again, and
///   an ordinary one: plenty of shipped binaries carry none.
/// * A name the loader resolves from `System32` has a version too, and it
///   describes Windows rather than the game. Reporting it would be worse than
///   saying nothing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FileVersion {
    /// Nobody has looked. The state every signal leaves [`analyse`] in, and the
    /// state a verdict that was never stamped stays in — which is why this is
    /// not merged with [`Self::Elsewhere`]: a consumer reading `elsewhere`
    /// should be able to believe that somebody checked.
    #[default]
    Unread,
    /// Looked, and no file of that name sits beside the executable. The normal
    /// answer for `d3d12.dll`: the loader resolves it from `System32`, and that
    /// copy's version is a fact about Windows.
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
    /// claim to make.
    ///
    /// The file version rather than the product version, because it is the one
    /// that identifies a build: NVIDIA ships `nvngx_dlss.dll` with a product
    /// version that repeats across a release line and a file version that does
    /// not. Both survive into the JSON, where they get compared.
    #[must_use]
    pub fn label(self) -> Option<String> {
        match self {
            // The only silence. Nobody looked, so there is nothing to say —
            // and a reader never sees this, because both surfaces stamp.
            Self::Unread => None,
            // Said out loud rather than left blank. Every other state prints
            // something, so a bare line would be indistinguishable from a
            // version somebody forgot to annotate — and this is precisely the
            // state whose whole reason for existing is that it is *not* that.
            // The words are about the directory rather than about Windows:
            // under Proton the copy that loads comes from a prefix, not from a
            // `System32` this tool has ever seen.
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

/// One thing concluded, and every observation that supports it.
///
/// A finding is never created without at least one signal, so the evidence for
/// it is always inspectable and a reader can disagree with the conclusion
/// without having to re-run the tool.
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
        // Signals are sorted strongest-first by `Collector::finish`, so the
        // first one is the strongest. The default is unreachable in practice
        // and chosen to be the weakest answer rather than the most flattering.
        self.signals
            .first()
            .map_or(Source::Neighbour, |signal| signal.source)
    }
}

/// Everything concluded about one binary.
///
/// Four separate lists rather than one list with a category tag, because the
/// four answer four different questions and a caller almost always wants one of
/// them. Mixing them would push a `match` into every consumer.
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
    /// is.
    ///
    /// The `dxgi.dll`-only case gets its own sentence on purpose: it is the
    /// case a naive classifier reports as "DirectX", which is the one claim the
    /// evidence does not support.
    #[must_use]
    pub fn headline(&self) -> String {
        if self.renderers.is_empty() {
            // Built from the infrastructure that was found rather than from a
            // fixed string. `dxcore.dll` is the newer adapter-enumeration layer
            // and fills the same slot as `dxgi.dll`; a hard-coded "DXGI
            // present" would say DXGI about a file containing none, and
            // confidently false is the one answer this crate must not produce.
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
        // A renderer known only from a directory listing is a weaker claim than
        // the bare name would sound: the file is there, and nothing observed
        // says the binary ever loads it. The headline has to carry that.
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
/// # Ordering
///
/// Every list comes back ordered by **evidence strength first** — import, then
/// delay-import, then neighbouring file — and then by a fixed table position.
/// For renderers that position is generation order, newest first; for features
/// it is specificity, so a finding that names a feature outranks one that
/// merely enables a family. Within a family the order is defensible; across
/// vendors (Vulkan against Direct3D 12, say) it is arbitrary and means nothing.
/// It is a presentation order. It is not a prediction of which API the process
/// ends up using, and nothing in this crate can make that prediction.
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

    // One link on, into libraries the image imports that sit beside it. The
    // signal names the INTERMEDIARY rather than the graphics library it reaches:
    // `UnityPlayer.dll` is what this executable's import table actually spelled,
    // and it is the file a reader would go and look at. Which Direct3D the
    // engine then reaches is the finding, not the signal.
    //
    // Renderers and infrastructure only. Features arrive from files in the
    // directory, not from an import table - `nvngx_dlss.dll` is loaded by name
    // at run time and appears in no import table anywhere - so chasing a link
    // for them would find nothing and imply it meant something.
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

    // A directory that is itself part of Windows describes the operating system
    // and not the binary, so its contents are dropped whole rather than being
    // filtered rule by rule. See `looks_like_system_directory`.
    if !looks_like_system_directory(&evidence.neighbours) {
        for name in &evidence.neighbours {
            let lower = name.to_ascii_lowercase();
            // The one file whose presence in a game directory is a renderer
            // signal rather than an injector: see `D3D12_CORE`. The label and
            // rank come from `renderer`, not from a second copy of them here.
            if lower == D3D12_CORE
                && let Some((label, rank)) = renderer(&lower)
            {
                renderers.add(label, rank, name, Source::Neighbour);
            } else if is_shadowable(&lower) {
                local_overrides.add(LOCAL_OVERRIDE, proxy_rank(&lower), name, Source::Neighbour);
            } else if let Some((label, rank)) = feature(&lower) {
                // Features are the case that makes `Source::Neighbour` worth
                // having. `nvngx_dlss.dll` is loaded by name at run time and
                // appears in no import table anywhere; shipping the file is how
                // DLSS arrives, so the directory is the only place it shows up.
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

/// The newer adapter-enumeration layer, filling the same slot as [`DXGI`]: it
/// enumerates devices and draws nothing. Filed here rather than left out
/// because a binary importing only `dxcore.dll` was otherwise told "no graphics
/// API determined" about a file that names an adapter layer, which is the
/// weaker of two true sentences and reads as though nothing was found.
const DXCORE: &str = "dxcore.dll";

/// Shared plumbing, if this is one of its entry points.
///
/// Neither name is a renderer: both rule OpenGL-only and Vulkan-only out and
/// say nothing else. The number is the tie-break position described on
/// [`analyse`], oldest and most common first, so a binary importing both leads
/// with the one a reader is expecting.
fn shared(lower: &str) -> Option<(&'static str, u8)> {
    match lower {
        DXGI => Some(("DXGI", 1)),
        DXCORE => Some(("DXCore", 2)),
        _ => None,
    }
}

/// The Direct3D 12 Agility SDK runtime. Deliberately exempt from
/// [`PROXYABLE`]: it is a Microsoft redistributable that only a Direct3D 12
/// application has any reason to ship, and it is loaded by `d3d12.dll` by name
/// rather than being a name the loader can be tricked into resolving. A copy
/// next to a game is a genuine Direct3D 12 signal, not an injector.
const D3D12_CORE: &str = "d3d12core.dll";

/// A graphics API, if this is one of its entry points.
///
/// The returned number is the tie-break position described on [`analyse`]:
/// newest generation first.
fn renderer(lower: &str) -> Option<(&'static str, u8)> {
    let hit = match lower {
        // `d3d12core.dll` as an *import* is unusual but unambiguous; as a
        // neighbouring file it is handled separately, in `analyse`.
        "d3d12.dll" | D3D12_CORE => ("Direct3D 12", 1),
        "vulkan-1.dll" => ("Vulkan", 2),
        // The `_1` to `_4` variants are the versioned device interfaces. A
        // binary that links one of them links Direct3D 11.
        //
        // `d3d11on12.dll` is the Direct3D 11 API running on a Direct3D 12
        // device. It lands here and not under Direct3D 12 because the code in
        // the binary is Direct3D 11 code; the device underneath it is chosen by
        // the runtime, not by the application.
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

/// The neural-rendering runtime NVIDIA ships as DLSS 5.
///
/// A fourth runtime rather than a replacement: public reporting has it running
/// as a post-process **alongside** `nvngx_dlss.dll`, `nvngx_dlssd.dll` and
/// `nvngx_dlssg.dll`, which is why it ranks above all three in [`feature`] —
/// where this file is shipped the others are shipped too, so it is the narrowest
/// of the four and the one a reader has not already guessed. It carries the same
/// `310.x.y.z` version scheme, so the reader in [`dxray_pe`] needed no change to
/// report it; only this table had to learn the name.
///
/// # The version resource does not say which build this is
///
/// For every other library in [`feature`] the number is the answer: `nvngx_dlss.dll`
/// has been called that in every release and only its resource separates one
/// build from the next. Here the resource separates less than it looks. NVIDIA's
/// build refuses anything below Blackwell, and the community ships patched
/// variants — an RTX 40 build, FP16 builds for the 20 and 30 series, a generic
/// patcher — none of which touch the version resource, so four different
/// binaries all report `310.8.0.0`. Telling them apart means inspecting the CUDA
/// fatbin, which is nowhere near what this crate reads, and no guess is made
/// here in its place: the printed line says a file of this name is there and
/// what its resource claims, and it is not a statement that the build is
/// NVIDIA's. The README says the same thing where a reader of the output will
/// meet it.
///
/// # Nothing about this file has been measured here
///
/// No copy has ever been through this code. The name, the version scheme, the
/// coexistence and the patched variants are public reporting, exactly the
/// standing the other three runtimes had before a real machine turned them up in
/// a shipped game's `bin/`. If the reporting is wrong about the name, this row
/// simply never matches and the output is what it was before — a dead row, not a
/// false one.
const DLSSNR: &str = "nvngx_dlssnr.dll";

/// A vendor feature, if this is one of its libraries.
///
/// Ranked by specificity: a library that names a feature outranks one that only
/// enables the family it belongs to, because "this ships DLSS Frame Generation"
/// is a fact and "this can talk to NVAPI" barely is. The same test orders the
/// DLSS runtimes among themselves: `nvngx_dlss.dll` ships with almost every
/// title that ships any of them and so narrows least, and each runtime that adds
/// a stage beside it narrows more and ranks above it. See [`DLSSNR`] for why
/// neural rendering leads.
fn feature(lower: &str) -> Option<(&'static str, u8)> {
    let hit = match lower {
        // "Neural Rendering" and not "DLSS 5": the file names a feature, the way
        // the three below do, and a release number is a claim about the build
        // that the version resource cannot support. See `DLSSNR`.
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
                // Streamline ships one interposer and a plugin per feature
                // (`sl.dlss.dll`, `sl.reflex.dll`, ...). They are one finding
                // with several signals; the plugin names stay in the signals.
                ("NVIDIA Streamline", 7)
            } else {
                return None;
            }
        }
    };
    Some(hit)
}

/// Leading substrings of the DLL names worth pulling out of the crowd, beyond
/// the ones the tables above already recognise.
///
/// Deliberately broad and deliberately dumb: it decides where a name is
/// printed, nothing else. A false positive costs a line of attention, while the
/// alternative — a curated list — costs a missed renderer every time a vendor
/// ships a new one.
///
/// Twelve of these match nothing the tables above recognise — `amd_ags`,
/// `dstorage`, `dxil`, `gfsdk`, `glide`, `igdext`, `nvofapi`, `openvr`,
/// `openxr` and the rest. They are kept, and kept *here*, because they make no
/// claim: they say a name is worth a reader's eye, never what it means. Only
/// [`is_graphics_related`] consults them, and it consults the rules first, so
/// this tier can widen an answer and can never contradict one. Dropping a name
/// would cost a vendor DLL going unnoticed in an import list of sixty; a rule
/// for one would be a judgement this crate cannot support with evidence.
const GRAPHICS_PREFIXES: &[&str] = &[
    "amd_ags", "amdxc", "d3d", "ddraw", "dstorage", "dxcore", "dxgi", "dxil", "ffx_", "gfsdk",
    "glide", "igdext", "libxess", "nvapi", "nvngx", "nvofapi", "openvr", "openxr", "opengl", "sl.",
    "vulkan", "xess",
];

/// True when `name` is a library worth separating from the api-set stubs in a
/// printed import list.
///
/// The question belongs here and not in a surface, because the answer has to
/// agree with the verdict printed above the list: every name this crate has a
/// rule for *that an import table can trigger* says yes, and the prefix table
/// above only widens that. A surface with a prefix table of its own is how the
/// same `amd_fidelityfx_dx12.dll` came to be reported as FSR and as
/// non-graphical in one block.
///
/// [`PROXYABLE`] is not consulted: it answers a question about a directory
/// listing, and eight of its entries are audio, input, HTTP and versioning
/// libraries that ordinary games import. None of the eight produces a finding,
/// and the shadowable names that do draw are in [`renderer`].
#[must_use]
pub fn is_graphics_related(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    renderer(&lower).is_some()
        || shared(&lower).is_some()
        || feature(&lower).is_some()
        || GRAPHICS_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// System DLL names that a local file can shadow.
///
/// Windows resolves an unqualified import against the executable's directory
/// before the system one, so a file with any of these names next to a game is
/// loaded instead of Microsoft's. That is how `ReShade`, `SpecialK`, `OptiScaler` and
/// every DLL mod loader attach: the file forwards every export to the real
/// system copy and does its own work on the way past.
///
/// This is a reportable fact in its own right and is explicitly *not* a
/// renderer signal. A `d3d11.dll` file in a game directory tells you a proxy is
/// installed; it tells you nothing about whether the game uses Direct3D 11,
/// because a proxy is named after whichever export the injector found
/// convenient.
const PROXYABLE: &[&str] = &[
    "dxgi.dll",
    "d3d12.dll",
    "d3d11.dll",
    "d3d10.dll",
    "d3d9.dll",
    "d3d8.dll",
    "ddraw.dll",
    "opengl32.dll",
    // Every other system renderer name was here and this one was not, which is
    // how a neighbouring Vulkan loader came to produce no verdict at all.
    // `is_shadowable` would catch it now in either case; it is named here
    // because this is the list a reader checks, and because the order of this
    // list is the order the report prints.
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

/// True when a file of this name beside an executable shadows a system DLL.
///
/// The question the neighbours loop has to ask, and it is wider than
/// [`PROXYABLE`]: every graphics entry point in [`renderer`] is a system DLL
/// the loader resolves the same way, so a local `vulkan-1.dll` or
/// `d3d11on12.dll` is the same trick as a local `dxgi.dll`. Asking only about
/// the named list made that loop answer "is it one of these sixteen names" to a
/// question about the loader, and a neighbouring Vulkan loader — a real file,
/// in the directory, found — came back as a verdict byte-identical to the one
/// for an empty directory.
///
/// [`D3D12_CORE`] is excluded by the caller, which reads it as a renderer
/// signal instead; see the constant for why it is not an injector.
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

/// Core operating system DLLs. These ship in `System32` and are never copied
/// into a game directory, so finding several of them side by side identifies
/// the directory rather than describing any binary in it.
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

/// How many of [`OS_CORE`] have to be present before the directory is read as
/// Windows' own. Three, because a mod loader might plausibly ship one of them
/// and no game ships three.
const SYSTEM_DIRECTORY_HITS: usize = 3;

/// True when the neighbours look like `System32` or `SysWOW64`.
///
/// Without this, pointing the tool at a system directory reports every renderer
/// DLL in Windows as an injector planted next to the binary — the directory
/// contains `dxgi.dll`, `d3d11.dll`, `opengl32.dll` and the rest because it is
/// Windows, not because someone installed `ReShade`. The rule that a local system
/// DLL is almost never Microsoft's holds in a game directory and inverts
/// completely here, so the neighbour evidence is discarded rather than
/// interpreted.
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
