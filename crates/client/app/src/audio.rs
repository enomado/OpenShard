//! The client-side audio mixer.
//!
//! The shard names effects and music, but never sends audio bytes.  Effects are
//! read from the installation's classic or UOP sound archive; music is found
//! under its `Music` directory. Failure to open an output device or one
//! optional asset leaves the world playable and is reported once, rather than
//! making a headless run or an incomplete client install fail at startup.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use openshard_protocol::server_packet::ServerPacket;
use openshard_protocol::wire::SoundId;
use openshard_protocol::world::{MusicId, Point};

/// Owns the optional platform output and the installed client audio assets.
pub(crate) struct Audio {
    #[cfg(not(target_arch = "wasm32"))]
    native: Option<NativeAudio>,
}

impl Audio {
    /// Build the mixer without making sound a prerequisite for opening a map.
    pub(crate) fn open(client_dir: &Path, effects_volume: f32, music_volume: f32) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            return Self {
                native: NativeAudio::open(client_dir, effects_volume, music_volume),
            };
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (client_dir, effects_volume, music_volume);
            Self {}
        }
    }

    /// Route the two server-directed audio packets to their installed assets.
    pub(crate) fn play_packet(&mut self, packet: &ServerPacket, listener: Point) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(audio) = self.native.as_mut() {
            match packet {
                ServerPacket::PlaySound(sound) => audio.play_sound(sound.sound, sound.at, listener),
                ServerPacket::PlayMusic(music) => audio.play_music(music.track),
                _ => {}
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (packet, listener);
        }
    }

    /// Change the two independent mixer gains without restarting their current
    /// sources. The effect gain is applied as each short source is mixed; the
    /// music player changes its gain immediately.
    pub(crate) fn set_volumes(&mut self, effects: f32, music: f32) {
        let settings = crate::desk::Audio { effects, music }.clamped();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(audio) = self.native.as_mut() {
            audio.effect_volume = settings.effects;
            audio.music.set_volume(settings.music);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = settings;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeAudio {
    output: rodio::MixerDeviceSink,
    effects: openshard_uofiles::sound::SoundArchive,
    music: rodio::Player,
    tracks: HashMap<String, PathBuf>,
    music_names: HashMap<MusicId, String>,
    effect_volume: f32,
    unheard: HashSet<SoundId>,
    missing_tracks: HashSet<MusicId>,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeAudio {
    fn open(client_dir: &Path, effect_volume: f32, music_volume: f32) -> Option<Self> {
        let effects = match openshard_uofiles::sound::SoundArchive::open(client_dir) {
            Ok(effects) => effects,
            Err(error) => {
                eprintln!("audio disabled: opening sound files: {error}");
                return None;
            }
        };
        let mut output = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(output) => output,
            Err(error) => {
                eprintln!("audio disabled: opening default output: {error}");
                return None;
            }
        };
        // Dropping the stream at shutdown is ordinary, not a diagnostic.
        output.log_on_drop(false);
        let music = rodio::Player::connect_new(output.mixer());
        music.set_volume(music_volume);
        Some(Self {
            output,
            effects,
            music,
            tracks: music_tracks(client_dir),
            music_names: music_names(client_dir),
            effect_volume,
            unheard: HashSet::new(),
            missing_tracks: HashSet::new(),
        })
    }

    fn play_sound(&mut self, id: SoundId, at: Point, listener: Point) {
        let sound = match self.effects.sound(id) {
            Ok(Some(sound)) => sound,
            Ok(None) => {
                if self.unheard.insert(id) {
                    eprintln!("audio: sound {:04X} is absent from this install", id.0);
                }
                return;
            }
            Err(error) => {
                if self.unheard.insert(id) {
                    eprintln!("audio: cannot read sound {:04X}: {error}", id.0);
                }
                return;
            }
        };
        use rodio::Source;
        let source = rodio::buffer::SamplesBuffer::new(
            std::num::NonZeroU16::new(sound.channels).unwrap_or(std::num::NonZeroU16::MIN),
            std::num::NonZeroU32::new(sound.sample_rate).unwrap_or(std::num::NonZeroU32::MIN),
            sound.samples,
        )
        .amplify(self.effect_volume);
        // World coordinates are deliberately left in tile units: Rodio's
        // spatial attenuation is then the same distance that decided whether
        // the shard broadcast the packet at all.
        let emitter = point(at);
        let origin = point(listener);
        self.output.mixer().add(rodio::source::Spatial::new(
            source,
            emitter,
            [origin[0] - 0.5, origin[1], origin[2]],
            [origin[0] + 0.5, origin[1], origin[2]],
        ));
    }

    fn play_music(&mut self, track: MusicId) {
        let Some(path) = self
            .music_names
            .get(&track)
            .and_then(|name| self.tracks.get(name))
            .cloned()
            .or_else(|| {
                track_names(track)
                    .iter()
                    .find_map(|name| self.tracks.get(name).cloned())
            })
        else {
            if self.missing_tracks.insert(track) {
                eprintln!("audio: music track {} is absent from this install", track.0);
            }
            return;
        };
        use rodio::Source;
        match std::fs::File::open(&path)
            .and_then(|file| rodio::Decoder::try_from(file).map_err(std::io::Error::other))
        {
            Ok(source) => {
                self.music.clear();
                self.music.append(source.repeat_infinite());
            }
            Err(error) => eprintln!("audio: cannot play {}: {error}", path.display()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn point(point: Point) -> [f32; 3] {
    [f32::from(point.x), f32::from(point.y), f32::from(point.z)]
}

/// Collect native music files once, accepting the capitalisation and file type
/// variants that UO installations have shipped over the years.
#[cfg(not(target_arch = "wasm32"))]
fn music_tracks(client_dir: &Path) -> HashMap<String, PathBuf> {
    let mut tracks = HashMap::new();
    for dir in [
        client_dir.join("Music"),
        client_dir.join("music"),
        client_dir.join("MUSIC"),
    ] {
        collect_music_tracks(&dir, &mut tracks);
    }
    tracks
}

/// The install owns the mapping from a wire music id to a filename.  Read its
/// configuration first so a shard with patched music does not get silently
/// redirected to one of the stock track names.
#[cfg(not(target_arch = "wasm32"))]
fn music_names(client_dir: &Path) -> HashMap<MusicId, String> {
    let config = [
        client_dir.join("Music/Digital/Config.txt"),
        client_dir.join("Music/Config.txt"),
        client_dir.join("music/digital/config.txt"),
        client_dir.join("music/config.txt"),
    ]
    .into_iter()
    .find(|path| path.is_file());
    let Some(config) = config else {
        return HashMap::new();
    };
    let Ok(contents) = std::fs::read_to_string(config) else {
        return HashMap::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let mut fields = line.split([' ', ',', '\t']).filter(|field| !field.is_empty());
            let id = fields.next()?.parse().ok()?;
            let filename = Path::new(fields.next()?)
                .file_stem()?
                .to_str()?
                .to_ascii_lowercase();
            Some((MusicId(id), filename))
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_music_tracks(dir: &Path, tracks: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_music_tracks(&path, tracks);
            continue;
        }
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            continue;
        };
        if !matches!(
            extension.to_ascii_lowercase().as_str(),
            "mp3" | "ogg" | "flac" | "wav"
        ) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            tracks.entry(stem.to_ascii_lowercase()).or_insert(path);
        }
    }
}

/// The classic packet ids whose names are stable across installs.  Numeric
/// forms are also tried, which keeps custom shards free to package their own
/// tracks without teaching the client a new protocol.
#[cfg(not(target_arch = "wasm32"))]
fn track_names(track: MusicId) -> Vec<String> {
    let mut names = vec![track.0.to_string(), format!("{:02}", track.0)];
    // Classic's `Music/Config.txt` uses this list when no local override is
    // present.  It also makes an unmodified installation work without asking
    // the shard to send filenames instead of the protocol's stable ids.
    const CLASSIC: &[&str] = &[
        "oldult01",
        "create1",
        "dragflit",
        "oldult02",
        "oldult03",
        "oldult04",
        "oldult05",
        "oldult06",
        "stones2",
        "britain1",
        "britain2",
        "bucsden",
        "jhelom",
        "lbcastle",
        "linelle",
        "magincia",
        "minoc",
        "ocllo",
        "samlethe",
        "serpents",
        "skarabra",
        "trinsic",
        "vesper",
        "wind",
        "yew",
        "cave01",
        "dungeon9",
        "forest_a",
        "intown01",
        "jungle_a",
        "mountn_a",
        "plains_a",
        "sailing",
        "swamp_a",
        "tavern01",
        "tavern02",
        "tavern03",
        "tavern04",
        "combat1",
        "combat2",
        "combat3",
        "approach",
        "death",
        "victory",
        "btcastle",
        "nujelm",
        "dungeon2",
        "cove",
        "moonglow",
        "zento",
        "tokunodungeon",
        "taiko",
        "dreadhornarea",
        "elfcity",
        "grizzledungeon",
        "melisandeslair",
        "paroxysmuslair",
        "gwennoconversation",
        "goodendgame",
        "goodvsevil",
        "greatearthserpents",
        "humanoids_u9",
        "minocnegative",
        "paws",
        "selimsbar",
        "serpentislecombat_u7",
        "valoriaships",
    ];
    if let Some(name) = CLASSIC.get(usize::from(track.0)) {
        names.push((*name).to_owned());
    }
    names
}
