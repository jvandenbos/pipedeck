//! EQ presets: preset files, the filter-chain graph, and the AutoEq importer
//! (SPEC §7).
//!
//! Everything here is pure data — no PipeWire types — so the whole of SPEC
//! §7.1's module-argument construction, §7.2's preset format and importer, and
//! the preset→control-parameter mapping are unit-tested without a graph. The
//! `unsafe` module load, the `Props` write and the WirePlumber `filters`
//! metadata live in [`crate::pw`], the only module that links libpipewire.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The PipeWire module the EQ is built on (SPEC §7.1).
pub const FILTER_CHAIN_MODULE: &str = "libpipewire-module-filter-chain";

/// Property the daemon stamps on both of a chain's nodes so it can recognise —
/// and hide — its own filter chains (SPEC §7.1).
pub const PROP_PIPEDECK_EQ: &str = "pipedeck.eq";

/// Prefix of the `node.link-group` shared by a chain's capture and playback
/// nodes; the second half of the hiding rule.
pub const LINK_GROUP_PREFIX: &str = "pipedeck-eq-";

/// `metadata.name` of the WirePlumber metadata that carries smart-filter
/// overrides (SPEC §7.1).
pub const METADATA_NAME_FILTERS: &str = "filters";

/// Key written into that metadata to bypass a chain without unloading it.
pub const KEY_FILTER_SMART_DISABLED: &str = "filter.smart.disabled";

/// Subdirectory of the config dir holding preset files (SPEC §7.2).
pub const PRESETS_DIR: &str = "eq";

/// How many peaking bands the fixed graph has.
pub const PEAKING_BANDS: usize = 12;

/// Control-port defaults for the unused low shelf.
const LOWSHELF_DEFAULT: (f32, f32) = (100.0, 0.707);
/// Control-port defaults for an unused peaking band.
const PEAKING_DEFAULT: (f32, f32) = (1000.0, 1.0);
/// Control-port defaults for the unused high shelf.
const HIGHSHELF_DEFAULT: (f32, f32) = (10_000.0, 0.707);

/// Lowest centre frequency a band may ask for, in Hz.
pub const MIN_FREQ: f32 = 10.0;
/// Highest centre frequency a band may ask for, in Hz.
pub const MAX_FREQ: f32 = 24_000.0;
/// Lowest Q a band may ask for.
pub const MIN_Q: f32 = 0.05;
/// Highest Q a band may ask for.
pub const MAX_Q: f32 = 20.0;
/// Most negative gain a band (or the preamp) may ask for, in dB.
pub const MIN_GAIN_DB: f32 = -30.0;
/// Most positive gain a band (or the preamp) may ask for, in dB.
pub const MAX_GAIN_DB: f32 = 30.0;

/// Which biquad a band is realised by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BandKind {
    /// `bq_lowshelf` — at most one per preset.
    Lowshelf,
    /// `bq_peaking` — at most [`PEAKING_BANDS`] per preset.
    Peaking,
    /// `bq_highshelf` — at most one per preset.
    Highshelf,
}

impl BandKind {
    /// The `filter.graph` builtin label this band maps to.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            BandKind::Lowshelf => "bq_lowshelf",
            BandKind::Peaking => "bq_peaking",
            BandKind::Highshelf => "bq_highshelf",
        }
    }

    /// The spelling used in a preset file's `type` field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BandKind::Lowshelf => "lowshelf",
            BandKind::Peaking => "peaking",
            BandKind::Highshelf => "highshelf",
        }
    }

    /// Parse an AutoEq filter-type token (`PK`, `LSC`, `LS`, `HSC`, `HS`).
    #[must_use]
    pub fn from_autoeq(token: &str) -> Option<Self> {
        match token.to_ascii_uppercase().as_str() {
            "PK" | "PEQ" | "PEAKING" => Some(BandKind::Peaking),
            "LS" | "LSC" | "LSQ" | "LOWSHELF" => Some(BandKind::Lowshelf),
            "HS" | "HSC" | "HSQ" | "HIGHSHELF" => Some(BandKind::Highshelf),
            _ => None,
        }
    }
}

/// One parametric band.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    /// Which biquad realises it.
    #[serde(rename = "type")]
    pub kind: BandKind,
    /// Centre (or corner) frequency, Hz.
    pub freq: f32,
    /// Q.
    pub q: f32,
    /// Gain, dB.
    pub gain_db: f32,
}

/// A preset as it appears on disk, minus the id (which is the file stem).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    /// Display name.
    pub name: String,
    /// Preamp, dB. Realised as `pre:Mult = 10^(preamp_db/20)`.
    #[serde(default)]
    pub preamp_db: f32,
    /// The bands, in graph order. Serialised as `[[band]]`, so it must stay the
    /// last field.
    #[serde(default, rename = "band")]
    pub bands: Vec<Band>,
}

/// A loaded preset: a [`PresetFile`] plus the id it is referred to by.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    /// File stem — what `[eq]` in the config and `SetEq` use.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Preamp, dB.
    pub preamp_db: f32,
    /// The bands.
    pub bands: Vec<Band>,
}

impl Preset {
    /// The `(id, name)` pair of the `EqPresets` D-Bus property.
    #[must_use]
    pub fn to_dbus(&self) -> (String, String) {
        (self.id.clone(), self.name.clone())
    }

    /// Re-render as the on-disk shape.
    #[must_use]
    pub fn to_file(&self) -> PresetFile {
        PresetFile {
            name: self.name.clone(),
            preamp_db: self.preamp_db,
            bands: self.bands.clone(),
        }
    }

    /// Serialise to preset TOML.
    ///
    /// # Errors
    /// Only if the preset somehow cannot be represented as TOML.
    pub fn to_toml(&self) -> Result<String, PresetError> {
        toml::to_string_pretty(&self.to_file()).map_err(|e| PresetError::Serialize(e.to_string()))
    }
}

/// Why a preset file was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    /// The file is not valid TOML, or has the wrong shape.
    Parse(String),
    /// The preset could not be rendered back to TOML.
    Serialize(String),
    /// A field is out of the range SPEC §7.2 allows.
    Invalid(String),
    /// The file name could not be turned into an id.
    BadId(String),
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresetError::Parse(e) => write!(f, "not a valid preset: {e}"),
            PresetError::Serialize(e) => write!(f, "could not serialise the preset: {e}"),
            PresetError::Invalid(e) => write!(f, "invalid preset: {e}"),
            PresetError::BadId(e) => write!(f, "unusable preset id: {e}"),
        }
    }
}

impl std::error::Error for PresetError {}

/// Lowercase a label into a `[a-z0-9-]` id: the file stem, and the id used by
/// the config and by D-Bus.
///
/// Returns an empty string when nothing usable is left, which callers must
/// treat as an error.
#[must_use]
pub fn slugify(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut pending_dash = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

/// Validate a parsed preset against SPEC §7.2's caps and ranges.
///
/// # Errors
/// [`PresetError::Invalid`] naming the first offending field.
pub fn validate(id: &str, file: &PresetFile) -> Result<Preset, PresetError> {
    if id.is_empty() {
        return Err(PresetError::BadId("empty id".to_owned()));
    }
    if !file.preamp_db.is_finite() || !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&file.preamp_db) {
        return Err(PresetError::Invalid(format!(
            "preamp_db must be between {MIN_GAIN_DB} and {MAX_GAIN_DB}, got {}",
            file.preamp_db
        )));
    }

    let mut counts: BTreeMap<BandKind, usize> = BTreeMap::new();
    for (n, band) in file.bands.iter().enumerate() {
        let where_ = format!("band {}", n + 1);
        if !band.freq.is_finite() || !(MIN_FREQ..=MAX_FREQ).contains(&band.freq) {
            return Err(PresetError::Invalid(format!(
                "{where_}: freq must be between {MIN_FREQ} and {MAX_FREQ} Hz, got {}",
                band.freq
            )));
        }
        if !band.q.is_finite() || !(MIN_Q..=MAX_Q).contains(&band.q) {
            return Err(PresetError::Invalid(format!(
                "{where_}: q must be between {MIN_Q} and {MAX_Q}, got {}",
                band.q
            )));
        }
        if !band.gain_db.is_finite() || !(MIN_GAIN_DB..=MAX_GAIN_DB).contains(&band.gain_db) {
            return Err(PresetError::Invalid(format!(
                "{where_}: gain_db must be between {MIN_GAIN_DB} and {MAX_GAIN_DB} dB, got {}",
                band.gain_db
            )));
        }
        *counts.entry(band.kind).or_default() += 1;
    }

    for (kind, cap) in [
        (BandKind::Lowshelf, 1),
        (BandKind::Peaking, PEAKING_BANDS),
        (BandKind::Highshelf, 1),
    ] {
        let used = counts.get(&kind).copied().unwrap_or(0);
        if used > cap {
            return Err(PresetError::Invalid(format!(
                "at most {cap} {} band(s) are supported, got {used}",
                kind.as_str()
            )));
        }
    }

    Ok(Preset {
        id: id.to_owned(),
        name: if file.name.trim().is_empty() {
            id.to_owned()
        } else {
            file.name.trim().to_owned()
        },
        preamp_db: file.preamp_db,
        bands: file.bands.clone(),
    })
}

/// Parse preset TOML and validate it.
///
/// # Errors
/// [`PresetError::Parse`] or [`PresetError::Invalid`].
pub fn parse_preset(id: &str, text: &str) -> Result<Preset, PresetError> {
    let file: PresetFile = toml::from_str(text).map_err(|e| PresetError::Parse(e.to_string()))?;
    validate(id, &file)
}

/// The presets directory: `<config dir>/eq` (SPEC §7.2).
///
/// # Errors
/// Whatever [`crate::config::Config::config_dir`] returns.
pub fn presets_dir() -> Result<PathBuf, crate::config::ConfigError> {
    Ok(crate::config::Config::config_dir()?.join(PRESETS_DIR))
}

/// Load every `*.toml` in a presets directory, ordered by id.
///
/// A missing directory yields an empty list. A file that fails to parse or
/// validate is skipped and reported in the second return value rather than
/// failing the whole scan — one bad file must not cost the user their others.
#[must_use]
pub fn load_presets(dir: &Path) -> (Vec<Preset>, Vec<String>) {
    let mut presets: Vec<Preset> = Vec::new();
    let mut problems: Vec<String> = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (presets, problems),
        Err(e) => {
            problems.push(format!("could not read {}: {e}", dir.display()));
            return (presets, problems);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let id = slugify(stem);
        if id.is_empty() {
            problems.push(format!("{}: file name has no usable id", path.display()));
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse_preset(&id, &text) {
                Ok(preset) => presets.push(preset),
                Err(e) => problems.push(format!("{}: {e}", path.display())),
            },
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }

    presets.sort_by(|a, b| a.id.cmp(&b.id));
    presets.dedup_by(|a, b| a.id == b.id);
    (presets, problems)
}

/// Write a preset into a presets directory as `<id>.toml`, creating the
/// directory if needed.
///
/// # Errors
/// I/O failures, or a preset that cannot be serialised.
pub fn write_preset(dir: &Path, preset: &Preset) -> Result<PathBuf, std::io::Error> {
    let text = preset
        .to_toml()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e}")))?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.toml", preset.id));
    std::fs::write(&path, text)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Control parameters (SPEC §7.1 — "preset apply = one Props param write")
// ---------------------------------------------------------------------------

/// The graph node name of the `n`-th peaking band (1-based).
#[must_use]
pub fn peaking_name(n: usize) -> String {
    format!("p{n}")
}

/// Linear preamp multiplier for a dB value: `10^(db/20)`.
#[must_use]
pub fn preamp_mult(preamp_db: f32) -> f32 {
    10f32.powf(preamp_db / 20.0)
}

/// The `(control, value)` pairs that realise a preset on the fixed graph.
///
/// Every control the graph has is always written, so switching presets can
/// never leave a band from the previous one behind: unused bands get `Gain 0`
/// (a 0 dB biquad is flat) with the default `Freq`/`Q` of their slot.
#[must_use]
pub fn preset_to_params(preset: &Preset) -> Vec<(String, f32)> {
    let lowshelf = preset.bands.iter().find(|b| b.kind == BandKind::Lowshelf);
    let highshelf = preset.bands.iter().find(|b| b.kind == BandKind::Highshelf);
    let peaking: Vec<&Band> = preset
        .bands
        .iter()
        .filter(|b| b.kind == BandKind::Peaking)
        .take(PEAKING_BANDS)
        .collect();

    let mut params: Vec<(String, f32)> = Vec::with_capacity(3 + 3 * (PEAKING_BANDS + 2));
    params.push(("pre:Mult".to_owned(), preamp_mult(preset.preamp_db)));

    let mut push = |name: &str, band: Option<&Band>, default: (f32, f32)| {
        let (freq, q, gain) =
            band.map_or((default.0, default.1, 0.0), |b| (b.freq, b.q, b.gain_db));
        params.push((format!("{name}:Freq"), freq));
        params.push((format!("{name}:Q"), q));
        params.push((format!("{name}:Gain"), gain));
    };

    push("ls", lowshelf, LOWSHELF_DEFAULT);
    for n in 1..=PEAKING_BANDS {
        push(
            &peaking_name(n),
            peaking.get(n - 1).copied(),
            PEAKING_DEFAULT,
        );
    }
    push("hs", highshelf, HIGHSHELF_DEFAULT);

    params
}

// ---------------------------------------------------------------------------
// Module arguments (SPEC §7.1)
// ---------------------------------------------------------------------------

/// `node.name` of a chain's main (capture) node — the `Audio/Sink` the daemon
/// tracks and writes controls to.
#[must_use]
pub fn eq_node_name(sink_name: &str) -> String {
    format!("pipedeck.eq.{sink_name}")
}

/// `node.name` of a chain's playback node.
#[must_use]
pub fn eq_playback_node_name(sink_name: &str) -> String {
    format!("pipedeck.eq.{sink_name}.out")
}

/// `node.link-group` (and `filter.smart.name`) shared by both of a chain's
/// nodes.
#[must_use]
pub fn eq_link_group(sink_name: &str) -> String {
    format!("{LINK_GROUP_PREFIX}{sink_name}")
}

/// Whether a node carrying these two property values is one of ours, and so
/// must be hidden from `Devices`/`Streams` (SPEC §7.1).
#[must_use]
pub fn is_eq_node(pipedeck_eq: Option<&str>, link_group: Option<&str>) -> bool {
    pipedeck_eq.is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || link_group.is_some_and(|g| g.starts_with(LINK_GROUP_PREFIX))
}

/// Split a node's `audio.position` property (`"[ FL, FR ]"`) into channel names.
#[must_use]
pub fn parse_positions(raw: &str) -> Vec<String> {
    raw.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Render an `f32` as JSON, always with a decimal point so a value like `100`
/// cannot be read back as an integer control.
fn num(v: f32) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e9 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

/// JSON-escape a string value.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Build the `libpipewire-module-filter-chain` argument string for one target
/// sink, exactly per SPEC §7.1.
///
/// The graph is fixed: a `linear` preamp, one `bq_lowshelf`, twelve
/// `bq_peaking`, one `bq_highshelf`, chained `pre → ls → p1 … p12 → hs`, with a
/// single 1-in/1-out port pair that filter-chain duplicates per channel. Presets
/// never rebuild it; they only write control values (see [`preset_to_params`]).
///
/// `position` is the target's `audio.position`; an empty slice omits the key and
/// lets filter-chain infer a layout.
#[must_use]
pub fn build_filter_chain_args(
    sink_name: &str,
    nick: &str,
    channels: usize,
    position: &[String],
) -> String {
    let node_name = eq_node_name(sink_name);
    let playback_name = eq_playback_node_name(sink_name);
    let link_group = eq_link_group(sink_name);
    let channels = channels.max(1);

    let mut nodes: Vec<String> = Vec::with_capacity(PEAKING_BANDS + 3);
    nodes.push(
        "    { \"type\": \"builtin\", \"label\": \"linear\", \"name\": \"pre\", \"control\": { \"Mult\": 1.0, \"Add\": 0.0 } }"
            .to_owned(),
    );
    nodes.push(biquad_node("bq_lowshelf", "ls", LOWSHELF_DEFAULT));
    for n in 1..=PEAKING_BANDS {
        nodes.push(biquad_node("bq_peaking", &peaking_name(n), PEAKING_DEFAULT));
    }
    nodes.push(biquad_node("bq_highshelf", "hs", HIGHSHELF_DEFAULT));

    let mut chain: Vec<String> = Vec::with_capacity(PEAKING_BANDS + 2);
    chain.push("pre".to_owned());
    chain.push("ls".to_owned());
    for n in 1..=PEAKING_BANDS {
        chain.push(peaking_name(n));
    }
    chain.push("hs".to_owned());

    let links: Vec<String> = chain
        .windows(2)
        .map(|pair| {
            format!(
                "    {{ \"output\": \"{}:Out\", \"input\": \"{}:In\" }}",
                pair[0], pair[1]
            )
        })
        .collect();

    let mut args = String::with_capacity(4096);
    args.push_str("{\n");
    let _ = writeln!(
        args,
        "  \"node.description\": {},",
        quote(&format!("PipeDeck EQ: {nick}"))
    );
    args.push_str("  \"media.name\": \"PipeDeck EQ\",\n");
    args.push_str("  \"filter.graph\": {\n");
    args.push_str("    \"nodes\": [\n");
    args.push_str(&nodes.join(",\n"));
    args.push_str("\n    ],\n");
    args.push_str("    \"links\": [\n");
    args.push_str(&links.join(",\n"));
    args.push_str("\n    ],\n");
    args.push_str("    \"inputs\": [ \"pre:In\" ],\n");
    args.push_str("    \"outputs\": [ \"hs:Out\" ]\n");
    args.push_str("  },\n");
    let _ = writeln!(args, "  \"audio.channels\": {channels},");
    if !position.is_empty() {
        let list: Vec<String> = position.iter().map(|p| quote(p)).collect();
        let _ = writeln!(args, "  \"audio.position\": [ {} ],", list.join(", "));
    }
    args.push_str("  \"capture.props\": {\n");
    let _ = writeln!(args, "    \"node.name\": {},", quote(&node_name));
    args.push_str("    \"media.class\": \"Audio/Sink\",\n");
    let _ = writeln!(args, "    \"node.link-group\": {},", quote(&link_group));
    let _ = writeln!(args, "    {}: true,", quote(PROP_PIPEDECK_EQ));
    args.push_str("    \"filter.smart\": true,\n");
    let _ = writeln!(args, "    \"filter.smart.name\": {},", quote(&link_group));
    let _ = writeln!(
        args,
        "    \"filter.smart.target\": {{ \"node.name\": {} }}",
        quote(sink_name)
    );
    args.push_str("  },\n");
    args.push_str("  \"playback.props\": {\n");
    let _ = writeln!(args, "    \"node.name\": {},", quote(&playback_name));
    args.push_str("    \"node.passive\": true,\n");
    let _ = writeln!(args, "    \"node.link-group\": {},", quote(&link_group));
    let _ = writeln!(args, "    {}: true,", quote(PROP_PIPEDECK_EQ));
    args.push_str("    \"stream.dont-remix\": true,\n");
    let _ = writeln!(args, "    \"target.object\": {}", quote(sink_name));
    args.push_str("  }\n");
    args.push('}');
    args
}

/// One `bq_*` node of the fixed graph, flat at load time.
fn biquad_node(label: &str, name: &str, default: (f32, f32)) -> String {
    format!(
        "    {{ \"type\": \"builtin\", \"label\": \"{label}\", \"name\": \"{name}\", \"control\": {{ \"Freq\": {}, \"Q\": {}, \"Gain\": 0.0 }} }}",
        num(default.0),
        num(default.1)
    )
}

// ---------------------------------------------------------------------------
// AutoEq importer (SPEC §7.2)
// ---------------------------------------------------------------------------

/// The result of parsing an AutoEq `ParametricEQ.txt`.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoEqImport {
    /// `Preamp: <n> dB`, or 0.0 when the file has no preamp line.
    pub preamp_db: f32,
    /// Bands, already capped to the fixed graph.
    pub bands: Vec<Band>,
    /// Non-fatal notes: dropped bands, unparsed lines, clamped values.
    pub warnings: Vec<String>,
}

/// Parse AutoEq's `ParametricEQ.txt` format.
///
/// Handles `Preamp: -6.4 dB` and
/// `Filter 1: ON PK Fc 105 Hz Gain 5.1 dB Q 0.70` (also `LSC`/`LS` →
/// lowshelf, `HSC`/`HS` → highshelf, `Fc 105.0 Hz`), skips `OFF` filters, and
/// drops bands beyond the graph's capacity with a warning.
///
/// # Errors
/// [`PresetError::Parse`] when the text contains no usable filter at all.
pub fn parse_autoeq(text: &str) -> Result<AutoEqImport, PresetError> {
    let mut preamp_db = 0.0_f32;
    let mut bands: Vec<Band> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut saw_filter_line = false;

    for (n, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lower = line.to_ascii_lowercase();

        if lower.starts_with("preamp") {
            match line.split(':').nth(1).and_then(first_number) {
                Some(v) => preamp_db = v,
                None => warnings.push(format!("line {}: could not read the preamp", n + 1)),
            }
            continue;
        }
        if !lower.starts_with("filter") {
            continue;
        }
        saw_filter_line = true;

        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.iter().any(|t| t.eq_ignore_ascii_case("OFF")) {
            continue;
        }
        let Some(on_at) = tokens.iter().position(|t| t.eq_ignore_ascii_case("ON")) else {
            warnings.push(format!("line {}: no ON/OFF marker; skipped", n + 1));
            continue;
        };
        let Some(kind) = tokens.get(on_at + 1).and_then(|t| BandKind::from_autoeq(t)) else {
            warnings.push(format!(
                "line {}: unsupported filter type `{}`; skipped",
                n + 1,
                tokens.get(on_at + 1).copied().unwrap_or("")
            ));
            continue;
        };

        let freq = labelled_number(&tokens, "Fc");
        let gain = labelled_number(&tokens, "Gain");
        let q = labelled_number(&tokens, "Q");
        let (Some(freq), Some(gain), Some(q)) = (freq, gain, q) else {
            warnings.push(format!("line {}: needs Fc, Gain and Q; skipped", n + 1));
            continue;
        };

        let band = Band {
            kind,
            freq: freq.clamp(MIN_FREQ, MAX_FREQ),
            q: q.clamp(MIN_Q, MAX_Q),
            gain_db: gain.clamp(MIN_GAIN_DB, MAX_GAIN_DB),
        };
        if (band.freq - freq).abs() > f32::EPSILON
            || (band.q - q).abs() > f32::EPSILON
            || (band.gain_db - gain).abs() > f32::EPSILON
        {
            warnings.push(format!(
                "line {}: values clamped to the allowed range",
                n + 1
            ));
        }
        bands.push(band);
    }

    if !saw_filter_line {
        return Err(PresetError::Parse(
            "no `Filter N: ...` lines found; is this an AutoEq ParametricEQ.txt?".to_owned(),
        ));
    }

    let preamp_clamped = preamp_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
    if (preamp_clamped - preamp_db).abs() > f32::EPSILON {
        warnings.push(format!(
            "preamp {preamp_db} dB clamped to {preamp_clamped} dB"
        ));
        preamp_db = preamp_clamped;
    }

    // SPEC §7.2: "the importer drops extras with a warning".
    let mut kept: Vec<Band> = Vec::with_capacity(bands.len());
    let (mut lows, mut peaks, mut highs) = (0usize, 0usize, 0usize);
    for band in bands {
        let (used, cap) = match band.kind {
            BandKind::Lowshelf => (&mut lows, 1),
            BandKind::Peaking => (&mut peaks, PEAKING_BANDS),
            BandKind::Highshelf => (&mut highs, 1),
        };
        if *used >= cap {
            warnings.push(format!(
                "more than {cap} {} band(s); the extra {:.0} Hz band was dropped",
                band.kind.as_str(),
                band.freq
            ));
            continue;
        }
        *used += 1;
        kept.push(band);
    }

    Ok(AutoEqImport {
        preamp_db,
        bands: kept,
        warnings,
    })
}

/// Turn a parsed AutoEq file into a preset with the given id and display name.
///
/// # Errors
/// Whatever [`validate`] rejects.
pub fn autoeq_to_preset(
    id: &str,
    name: &str,
    import: &AutoEqImport,
) -> Result<Preset, PresetError> {
    validate(
        id,
        &PresetFile {
            name: name.to_owned(),
            preamp_db: import.preamp_db,
            bands: import.bands.clone(),
        },
    )
}

/// The first number in a fragment, tolerating a trailing unit (`-6.4 dB`).
fn first_number(fragment: &str) -> Option<f32> {
    fragment.split_whitespace().find_map(parse_number)
}

/// The number that follows a label token (`Fc 105 Hz` → 105), also accepting
/// the glued spelling (`Fc105`).
fn labelled_number(tokens: &[&str], label: &str) -> Option<f32> {
    for (i, token) in tokens.iter().enumerate() {
        if token.eq_ignore_ascii_case(label) {
            return tokens.get(i + 1).and_then(|t| parse_number(t));
        }
        if let Some(rest) = strip_prefix_ignore_case(token, label) {
            if let Some(v) = parse_number(rest) {
                return Some(v);
            }
        }
    }
    None
}

fn strip_prefix_ignore_case<'a>(token: &'a str, prefix: &str) -> Option<&'a str> {
    if token.len() > prefix.len() && token[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&token[prefix.len()..])
    } else {
        None
    }
}

/// Parse a number, ignoring a trailing unit glued to it (`105Hz`, `5.1dB`).
fn parse_number(token: &str) -> Option<f32> {
    let end = token
        .char_indices()
        .find(|(i, c)| {
            !(c.is_ascii_digit()
                || *c == '.'
                || ((*c == '-' || *c == '+') && *i == 0)
                || c.eq_ignore_ascii_case(&'e') && *i > 0)
        })
        .map_or(token.len(), |(i, _)| i);
    let head = &token[..end];
    if head.is_empty() || head == "-" || head == "+" || head == "." {
        return None;
    }
    head.parse::<f32>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(bands: Vec<Band>) -> Preset {
        Preset {
            id: "test".to_owned(),
            name: "Test".to_owned(),
            preamp_db: -6.0,
            bands,
        }
    }

    fn band(kind: BandKind, freq: f32, q: f32, gain_db: f32) -> Band {
        Band {
            kind,
            freq,
            q,
            gain_db,
        }
    }

    /// A real ten-line AutoEq export, as `ParametricEQ.txt` ships it.
    const AUTOEQ_SAMPLE: &str = "\
Preamp: -6.4 dB
Filter 1: ON LSC Fc 105 Hz Gain 5.1 dB Q 0.70
Filter 2: ON PK Fc 22 Hz Gain 1.9 dB Q 0.66
Filter 3: ON PK Fc 1030 Hz Gain -2.4 dB Q 1.44
Filter 4: ON PK Fc 2380 Hz Gain 3.6 dB Q 2.71
Filter 5: ON PK Fc 3700 Hz Gain -4.8 dB Q 3.02
Filter 6: ON PK Fc 5900 Hz Gain 2.2 dB Q 4.10
Filter 7: OFF PK Fc 8000 Hz Gain 0.0 dB Q 1.00
Filter 8: ON HSC Fc 10000 Hz Gain -1.2 dB Q 0.70
Filter 9: ON PK Fc 60.0 Hz Gain 0.8 dB Q 1.10
";

    #[test]
    fn slugify_makes_config_safe_ids() {
        assert_eq!(slugify("Sennheiser HD 650"), "sennheiser-hd-650");
        assert_eq!(slugify("HD650_AutoEq"), "hd650-autoeq");
        assert_eq!(slugify("  Bass++  "), "bass");
        assert_eq!(slugify("a"), "a");
        assert_eq!(slugify("!!!"), "");
        assert_eq!(slugify("Ürsula"), "rsula");
    }

    #[test]
    fn preset_toml_round_trips() {
        let text = r#"
name = "Sennheiser HD 650 (AutoEq)"
preamp_db = -6.4

[[band]]
type = "lowshelf"
freq = 105.0
q = 0.7
gain_db = 5.1

[[band]]
type = "peaking"
freq = 1030.0
q = 1.44
gain_db = -2.4

[[band]]
type = "highshelf"
freq = 10000.0
q = 0.7
gain_db = -1.2
"#;
        let parsed = parse_preset("hd650", text).expect("parses");
        assert_eq!(parsed.id, "hd650");
        assert_eq!(parsed.name, "Sennheiser HD 650 (AutoEq)");
        assert!((parsed.preamp_db - -6.4).abs() < 1e-6);
        assert_eq!(parsed.bands.len(), 3);
        assert_eq!(parsed.bands[0].kind, BandKind::Lowshelf);
        assert_eq!(parsed.bands[2].kind, BandKind::Highshelf);

        let again = parse_preset("hd650", &parsed.to_toml().expect("serialises")).expect("reparse");
        assert_eq!(again, parsed);
    }

    #[test]
    fn preset_without_a_name_falls_back_to_the_id() {
        let parsed = parse_preset("flat", "name = \"\"\n").expect("parses");
        assert_eq!(parsed.name, "flat");
        assert!(parsed.bands.is_empty());
        assert!((parsed.preamp_db).abs() < f32::EPSILON);
    }

    #[test]
    fn validation_enforces_the_spec_caps_and_ranges() {
        let ok = PresetFile {
            name: "ok".to_owned(),
            preamp_db: 0.0,
            bands: vec![band(BandKind::Peaking, 1000.0, 1.0, 3.0)],
        };
        assert!(validate("ok", &ok).is_ok());
        assert!(matches!(validate("", &ok), Err(PresetError::BadId(_))));

        let too_many_peaks = PresetFile {
            name: "x".to_owned(),
            preamp_db: 0.0,
            bands: (0..PEAKING_BANDS + 1)
                .map(|_| band(BandKind::Peaking, 1000.0, 1.0, 0.0))
                .collect(),
        };
        assert!(matches!(
            validate("x", &too_many_peaks),
            Err(PresetError::Invalid(_))
        ));

        let two_shelves = PresetFile {
            name: "x".to_owned(),
            preamp_db: 0.0,
            bands: vec![
                band(BandKind::Lowshelf, 100.0, 0.7, 1.0),
                band(BandKind::Lowshelf, 200.0, 0.7, 1.0),
            ],
        };
        assert!(matches!(
            validate("x", &two_shelves),
            Err(PresetError::Invalid(_))
        ));

        for bad in [
            band(BandKind::Peaking, 5.0, 1.0, 0.0),
            band(BandKind::Peaking, 30_000.0, 1.0, 0.0),
            band(BandKind::Peaking, 1000.0, 0.001, 0.0),
            band(BandKind::Peaking, 1000.0, 50.0, 0.0),
            band(BandKind::Peaking, 1000.0, 1.0, 40.0),
            band(BandKind::Peaking, 1000.0, 1.0, -40.0),
            band(BandKind::Peaking, f32::NAN, 1.0, 0.0),
        ] {
            let file = PresetFile {
                name: "x".to_owned(),
                preamp_db: 0.0,
                bands: vec![bad],
            };
            assert!(
                matches!(validate("x", &file), Err(PresetError::Invalid(_))),
                "should have rejected {bad:?}"
            );
        }

        let loud_preamp = PresetFile {
            name: "x".to_owned(),
            preamp_db: 99.0,
            bands: vec![],
        };
        assert!(matches!(
            validate("x", &loud_preamp),
            Err(PresetError::Invalid(_))
        ));
    }

    #[test]
    fn presets_load_from_a_directory_and_bad_files_are_reported_not_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("hd650.toml"),
            "name = \"HD 650\"\npreamp_db = -6.4\n",
        )
        .expect("write");
        std::fs::write(dir.path().join("flat.toml"), "name = \"Flat\"\n").expect("write");
        std::fs::write(dir.path().join("broken.toml"), "name = 5\n").expect("write");
        std::fs::write(dir.path().join("notes.txt"), "ignored").expect("write");

        let (presets, problems) = load_presets(dir.path());
        let ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["flat", "hd650"]);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("broken.toml"));

        // A missing directory is not an error; it just has no presets.
        let (empty, none) = load_presets(&dir.path().join("absent"));
        assert!(empty.is_empty());
        assert!(none.is_empty());
    }

    #[test]
    fn write_preset_round_trips_through_the_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = preset(vec![band(BandKind::Peaking, 1000.0, 1.0, 3.0)]);
        let path = write_preset(&dir.path().join("nested"), &p).expect("write");
        assert!(path.ends_with("test.toml"));
        let (loaded, problems) = load_presets(&dir.path().join("nested"));
        assert!(problems.is_empty());
        assert_eq!(loaded, vec![p]);
    }

    #[test]
    fn preamp_mult_is_the_db_conversion() {
        assert!((preamp_mult(0.0) - 1.0).abs() < 1e-6);
        assert!((preamp_mult(-6.0) - 0.501_187).abs() < 1e-5);
        assert!((preamp_mult(6.0) - 1.995_262).abs() < 1e-5);
    }

    #[test]
    fn preset_to_params_writes_every_control_on_the_graph() {
        let p = preset(vec![
            band(BandKind::Lowshelf, 105.0, 0.7, 5.1),
            band(BandKind::Peaking, 1030.0, 1.44, -2.4),
            band(BandKind::Peaking, 2380.0, 2.71, 3.6),
            band(BandKind::Highshelf, 10_000.0, 0.7, -1.2),
        ]);
        let params = preset_to_params(&p);
        // pre:Mult + 3 controls x (1 lowshelf + 12 peaking + 1 highshelf).
        assert_eq!(params.len(), 1 + 3 * (PEAKING_BANDS + 2));

        let by_name: BTreeMap<&str, f32> = params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert!((by_name["pre:Mult"] - preamp_mult(-6.0)).abs() < 1e-6);
        assert!((by_name["ls:Freq"] - 105.0).abs() < 1e-6);
        assert!((by_name["ls:Gain"] - 5.1).abs() < 1e-6);
        assert!((by_name["p1:Freq"] - 1030.0).abs() < 1e-6);
        assert!((by_name["p2:Gain"] - 3.6).abs() < 1e-6);
        assert!((by_name["hs:Freq"] - 10_000.0).abs() < 1e-6);
        assert!((by_name["hs:Gain"] - -1.2).abs() < 1e-6);

        // Unused peaking slots are flat at their defaults, so switching presets
        // can never leave a band from the previous one behind.
        for n in 3..=PEAKING_BANDS {
            assert!((by_name[format!("p{n}:Gain").as_str()]).abs() < f32::EPSILON);
            assert!((by_name[format!("p{n}:Freq").as_str()] - PEAKING_DEFAULT.0).abs() < 1e-6);
            assert!((by_name[format!("p{n}:Q").as_str()] - PEAKING_DEFAULT.1).abs() < 1e-6);
        }

        // A preset with no shelves still writes flat shelf controls.
        let bare = preset(vec![]);
        let bare_params: BTreeMap<String, f32> = preset_to_params(&bare).into_iter().collect();
        assert!((bare_params["ls:Gain"]).abs() < f32::EPSILON);
        assert!((bare_params["hs:Gain"]).abs() < f32::EPSILON);
        assert!((bare_params["ls:Freq"] - LOWSHELF_DEFAULT.0).abs() < 1e-6);
        assert!((bare_params["hs:Freq"] - HIGHSHELF_DEFAULT.0).abs() < 1e-6);
    }

    #[test]
    fn preset_to_params_ignores_bands_beyond_the_graph() {
        let mut bands = vec![band(BandKind::Peaking, 100.0, 1.0, 1.0); PEAKING_BANDS];
        bands.push(band(BandKind::Peaking, 9999.0, 1.0, 9.0));
        let params: BTreeMap<String, f32> = preset_to_params(&preset(bands)).into_iter().collect();
        assert_eq!(params.len(), 1 + 3 * (PEAKING_BANDS + 2));
        assert!(!params.values().any(|v| (*v - 9999.0).abs() < 1e-6));
    }

    #[test]
    fn node_naming_and_hiding_rules() {
        let sink = "alsa_output.pci-0000_28_00.4.analog-stereo";
        assert_eq!(
            eq_node_name(sink),
            "pipedeck.eq.alsa_output.pci-0000_28_00.4.analog-stereo"
        );
        assert_eq!(
            eq_playback_node_name(sink),
            "pipedeck.eq.alsa_output.pci-0000_28_00.4.analog-stereo.out"
        );
        assert_eq!(
            eq_link_group(sink),
            "pipedeck-eq-alsa_output.pci-0000_28_00.4.analog-stereo"
        );

        assert!(is_eq_node(Some("true"), None));
        assert!(is_eq_node(Some("TRUE"), None));
        assert!(is_eq_node(None, Some("pipedeck-eq-sink-a")));
        assert!(!is_eq_node(None, None));
        assert!(!is_eq_node(Some("false"), Some("other-group")));
    }

    #[test]
    fn positions_parse_from_the_props_spelling() {
        assert_eq!(parse_positions("[ FL, FR ]"), vec!["FL", "FR"]);
        assert_eq!(
            parse_positions("[FL FR FC LFE SL SR]"),
            vec!["FL", "FR", "FC", "LFE", "SL", "SR"]
        );
        assert_eq!(parse_positions("AUX0,AUX1"), vec!["AUX0", "AUX1"]);
        assert!(parse_positions("[]").is_empty());
        assert!(parse_positions("").is_empty());
    }

    /// The module argument string is the one thing that cannot be checked on a
    /// live graph without loading it, so pin its structure hard here.
    #[test]
    fn filter_chain_args_match_spec_7_1() {
        let sink = "alsa_output.pci-0000_28_00.4.analog-stereo";
        let args = build_filter_chain_args(
            sink,
            "ALC892 Analog",
            2,
            &["FL".to_owned(), "FR".to_owned()],
        );

        // Valid JSON (SPA-JSON is a superset, so this is the strict check).
        let parsed: serde_json::Value = serde_json::from_str(&args).expect("valid JSON");

        assert_eq!(parsed["node.description"], "PipeDeck EQ: ALC892 Analog");
        assert_eq!(parsed["media.name"], "PipeDeck EQ");
        assert_eq!(parsed["audio.channels"], 2);
        assert_eq!(parsed["audio.position"][0], "FL");
        assert_eq!(parsed["audio.position"][1], "FR");

        let nodes = parsed["filter.graph"]["nodes"]
            .as_array()
            .expect("nodes array");
        assert_eq!(nodes.len(), PEAKING_BANDS + 3);
        assert_eq!(nodes[0]["label"], "linear");
        assert_eq!(nodes[0]["name"], "pre");
        assert_eq!(nodes[0]["control"]["Mult"], 1.0);
        assert_eq!(nodes[0]["control"]["Add"], 0.0);
        assert_eq!(nodes[1]["label"], "bq_lowshelf");
        assert_eq!(nodes[1]["name"], "ls");
        assert_eq!(nodes[1]["control"]["Gain"], 0.0);
        assert_eq!(nodes[2]["label"], "bq_peaking");
        assert_eq!(nodes[2]["name"], "p1");
        assert_eq!(nodes[PEAKING_BANDS + 1]["name"], "p12");
        assert_eq!(nodes[PEAKING_BANDS + 2]["label"], "bq_highshelf");
        assert_eq!(nodes[PEAKING_BANDS + 2]["name"], "hs");
        for node in nodes {
            assert_eq!(node["type"], "builtin");
        }

        // pre -> ls -> p1 .. p12 -> hs is 13 hops for 14 nodes.
        let links = parsed["filter.graph"]["links"]
            .as_array()
            .expect("links array");
        assert_eq!(links.len(), PEAKING_BANDS + 2 - 1 + 1);
        assert_eq!(links[0]["output"], "pre:Out");
        assert_eq!(links[0]["input"], "ls:In");
        assert_eq!(links[1]["output"], "ls:Out");
        assert_eq!(links[1]["input"], "p1:In");
        assert_eq!(links[links.len() - 1]["output"], "p12:Out");
        assert_eq!(links[links.len() - 1]["input"], "hs:In");

        assert_eq!(parsed["filter.graph"]["inputs"][0], "pre:In");
        assert_eq!(parsed["filter.graph"]["outputs"][0], "hs:Out");

        let capture = &parsed["capture.props"];
        assert_eq!(capture["node.name"], format!("pipedeck.eq.{sink}"));
        assert_eq!(capture["media.class"], "Audio/Sink");
        assert_eq!(capture["node.link-group"], format!("pipedeck-eq-{sink}"));
        assert_eq!(capture[PROP_PIPEDECK_EQ], true);
        assert_eq!(capture["filter.smart"], true);
        assert_eq!(capture["filter.smart.name"], format!("pipedeck-eq-{sink}"));
        assert_eq!(capture["filter.smart.target"]["node.name"], sink);

        let playback = &parsed["playback.props"];
        assert_eq!(playback["node.name"], format!("pipedeck.eq.{sink}.out"));
        assert_eq!(playback["node.passive"], true);
        assert_eq!(playback["node.link-group"], format!("pipedeck-eq-{sink}"));
        assert_eq!(playback[PROP_PIPEDECK_EQ], true);
        assert_eq!(playback["stream.dont-remix"], true);
        assert_eq!(playback["target.object"], sink);
    }

    #[test]
    fn filter_chain_args_omit_an_unknown_position_and_never_ask_for_zero_channels() {
        let args = build_filter_chain_args("sink-a", "Sink A", 0, &[]);
        let parsed: serde_json::Value = serde_json::from_str(&args).expect("valid JSON");
        assert_eq!(parsed["audio.channels"], 1);
        assert!(parsed.get("audio.position").is_none());
    }

    #[test]
    fn filter_chain_args_escape_hostile_names() {
        let args = build_filter_chain_args("we\"ird\\sink", "Ni\"ck", 6, &["FL".to_owned()]);
        let parsed: serde_json::Value = serde_json::from_str(&args).expect("valid JSON");
        assert_eq!(
            parsed["capture.props"]["node.name"],
            "pipedeck.eq.we\"ird\\sink"
        );
        assert_eq!(parsed["playback.props"]["target.object"], "we\"ird\\sink");
        assert_eq!(parsed["node.description"], "PipeDeck EQ: Ni\"ck");
    }

    #[test]
    fn autoeq_sample_imports() {
        let import = parse_autoeq(AUTOEQ_SAMPLE).expect("parses");
        assert!((import.preamp_db - -6.4).abs() < 1e-6);
        // 9 filter lines, one OFF -> 8 bands.
        assert_eq!(import.bands.len(), 8);
        assert!(import.warnings.is_empty(), "{:?}", import.warnings);

        assert_eq!(import.bands[0].kind, BandKind::Lowshelf);
        assert!((import.bands[0].freq - 105.0).abs() < 1e-6);
        assert!((import.bands[0].gain_db - 5.1).abs() < 1e-6);
        assert!((import.bands[0].q - 0.7).abs() < 1e-6);

        assert_eq!(import.bands[1].kind, BandKind::Peaking);
        assert!((import.bands[1].freq - 22.0).abs() < 1e-6);
        assert!((import.bands[2].gain_db - -2.4).abs() < 1e-6);

        // `HSC` maps to the high shelf, and a decimal Fc is fine.
        let hs = import
            .bands
            .iter()
            .find(|b| b.kind == BandKind::Highshelf)
            .expect("a high shelf");
        assert!((hs.freq - 10_000.0).abs() < 1e-6);
        assert!((hs.gain_db - -1.2).abs() < 1e-6);
        assert!(import
            .bands
            .iter()
            .any(|b| (b.freq - 60.0).abs() < 1e-6 && (b.gain_db - 0.8).abs() < 1e-6));

        // The OFF filter never made it in.
        assert!(!import.bands.iter().any(|b| (b.freq - 8000.0).abs() < 1e-6));

        let preset =
            autoeq_to_preset("hd650", "Sennheiser HD 650 (AutoEq)", &import).expect("valid preset");
        assert_eq!(preset.id, "hd650");
        assert_eq!(preset.bands.len(), 8);
        // And it survives a trip through the on-disk format.
        let again = parse_preset("hd650", &preset.to_toml().expect("toml")).expect("reparse");
        assert_eq!(again, preset);
    }

    #[test]
    fn autoeq_tolerates_glued_units_and_alternate_type_spellings() {
        let text = "\
Preamp: -3 dB
Filter 1: ON LS Fc 105.0 Hz Gain 5.1 dB Q 0.70
Filter 2: ON HS Fc 8000Hz Gain -1.5dB Q 0.71
Filter 3: ON PK Fc 1000 Hz Gain 0 dB Q 1
";
        let import = parse_autoeq(text).expect("parses");
        assert!((import.preamp_db - -3.0).abs() < 1e-6);
        assert_eq!(import.bands.len(), 3);
        assert_eq!(import.bands[0].kind, BandKind::Lowshelf);
        assert_eq!(import.bands[1].kind, BandKind::Highshelf);
        assert!((import.bands[1].freq - 8000.0).abs() < 1e-6);
        assert!((import.bands[1].gain_db - -1.5).abs() < 1e-6);
        assert_eq!(import.bands[2].kind, BandKind::Peaking);
    }

    #[test]
    fn autoeq_drops_extras_and_reports_them() {
        let mut text = String::from("Preamp: 0.0 dB\n");
        for n in 1..=PEAKING_BANDS + 2 {
            let _ = writeln!(
                text,
                "Filter {n}: ON PK Fc {} Hz Gain 1.0 dB Q 1.00",
                100 * n
            );
        }
        let _ = writeln!(text, "Filter 90: ON LSC Fc 100 Hz Gain 1.0 dB Q 0.70");
        let _ = writeln!(text, "Filter 91: ON LSC Fc 200 Hz Gain 1.0 dB Q 0.70");

        let import = parse_autoeq(&text).expect("parses");
        assert_eq!(
            import
                .bands
                .iter()
                .filter(|b| b.kind == BandKind::Peaking)
                .count(),
            PEAKING_BANDS
        );
        assert_eq!(
            import
                .bands
                .iter()
                .filter(|b| b.kind == BandKind::Lowshelf)
                .count(),
            1
        );
        assert_eq!(import.warnings.len(), 3);
        // Whatever survived must still validate.
        assert!(autoeq_to_preset("x", "X", &import).is_ok());
    }

    #[test]
    fn autoeq_reports_unusable_input() {
        assert!(matches!(
            parse_autoeq("this is not an autoeq file\n"),
            Err(PresetError::Parse(_))
        ));
        // A file of only OFF filters parses, but produces nothing.
        let off = parse_autoeq("Filter 1: OFF PK Fc 100 Hz Gain 0 dB Q 1\n").expect("parses");
        assert!(off.bands.is_empty());
    }

    #[test]
    fn autoeq_warns_about_junk_lines_and_clamps() {
        let text = "\
Preamp: nonsense dB
Filter 1: ON XX Fc 100 Hz Gain 1 dB Q 1
Filter 2: ON PK Fc 100 Hz Gain 99 dB Q 1
Filter 3: PK Fc 100 Hz Gain 1 dB Q 1
Filter 4: ON PK Fc 100 Hz Q 1
";
        let import = parse_autoeq(text).expect("parses");
        assert_eq!(import.bands.len(), 1);
        assert!((import.bands[0].gain_db - MAX_GAIN_DB).abs() < 1e-6);
        assert_eq!(import.warnings.len(), 5);
    }

    #[test]
    fn number_parsing_helpers() {
        assert_eq!(parse_number("105"), Some(105.0));
        assert_eq!(parse_number("105.5Hz"), Some(105.5));
        assert_eq!(parse_number("-6.4"), Some(-6.4));
        assert_eq!(parse_number("dB"), None);
        assert_eq!(parse_number("-"), None);
        assert_eq!(first_number(" -6.4 dB"), Some(-6.4));
        let tokens = ["Filter", "1:", "ON", "PK", "Fc", "105", "Hz"];
        assert_eq!(labelled_number(&tokens, "Fc"), Some(105.0));
        assert_eq!(labelled_number(&tokens, "Gain"), None);
    }

    #[test]
    fn band_kind_spellings() {
        assert_eq!(BandKind::from_autoeq("pk"), Some(BandKind::Peaking));
        assert_eq!(BandKind::from_autoeq("LSC"), Some(BandKind::Lowshelf));
        assert_eq!(BandKind::from_autoeq("HS"), Some(BandKind::Highshelf));
        assert_eq!(BandKind::from_autoeq("NOTCH"), None);
        assert_eq!(BandKind::Peaking.label(), "bq_peaking");
        assert_eq!(BandKind::Lowshelf.label(), "bq_lowshelf");
        assert_eq!(BandKind::Highshelf.label(), "bq_highshelf");
        assert_eq!(BandKind::Highshelf.as_str(), "highshelf");
    }

    /// The presets `install.sh` ships (packaging's lane) are this module's
    /// input format. Parsing two of them here catches a format drift between
    /// the two lanes at build time instead of on chronos.
    #[test]
    fn the_shipped_presets_parse() {
        let flat = parse_preset("flat", include_str!("../../../presets/flat.toml"))
            .expect("presets/flat.toml parses");
        assert_eq!(flat.name, "Flat");
        assert!(flat.bands.is_empty());

        let loudness = parse_preset("loudness", include_str!("../../../presets/loudness.toml"))
            .expect("presets/loudness.toml parses");
        // The one shipped preset that exercises all three band types.
        assert_eq!(loudness.bands.len(), 3);
        assert_eq!(loudness.bands[0].kind, BandKind::Lowshelf);
        assert_eq!(loudness.bands[1].kind, BandKind::Peaking);
        assert_eq!(loudness.bands[2].kind, BandKind::Highshelf);
        assert_eq!(
            preset_to_params(&loudness).len(),
            1 + 3 * (PEAKING_BANDS + 2)
        );
    }

    #[test]
    fn preset_projects_the_dbus_pair() {
        let p = preset(vec![]);
        assert_eq!(p.to_dbus(), ("test".to_owned(), "Test".to_owned()));
    }
}
