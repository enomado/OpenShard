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
            Self {
                native: NativeAudio::open(client_dir, effects_volume, music_volume),
            }
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

    /// Give the music its next turn, once a frame.
    ///
    /// A looping track has to be started again when it ends, and the mixer owns
    /// no clock to notice that for itself. The check is an atomic load against a
    /// source that is minutes long, so a frame is a generous place for it — and
    /// it is the frame that already owns everything else that advances.
    pub(crate) fn advance(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(audio) = self.native.as_mut() {
            audio.repeat_finished_track();
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
    music_names: HashMap<MusicId, Track>,
    /// The file to start again when the music player runs dry — `None` while
    /// nothing is playing, and while what is playing is a track the install
    /// marks as playing once.
    looping: Option<PathBuf>,
    effect_volume: f32,
    unheard: HashSet<SoundId>,
    missing_tracks: HashSet<MusicId>,
}

/// A track as an installation names it: the file, without its extension, and
/// whether it plays once or until something replaces it.
///
/// The flag is not decoration. Region music loops; a victory sting does not,
/// and a client that repeats one plays it over a player who has walked away.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct Track {
    name: String,
    looping: bool,
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
            looping: None,
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
        let Some((path, looping)) = self.resolve(track) else {
            if self.missing_tracks.insert(track) {
                eprintln!("audio: music track {} is absent from this install", track.0);
            }
            return;
        };
        let Some(source) = decode(&path) else {
            return;
        };
        start_track(&self.music, source);
        // Remembered rather than wrapped in a repeating source: see
        // `repeat_finished_track`, and the trap written above it.
        self.looping = looping.then_some(path);
    }

    /// Which file this shard's track id names here, and whether it repeats.
    ///
    /// Three answers in the order they are trusted: what the installation's own
    /// config says, a file named after the id itself — which is how a pack ships
    /// music of its own without a protocol for it — and finally the classic
    /// table, so an install with no config still plays what every client has
    /// played since 1997.
    fn resolve(&self, track: MusicId) -> Option<(PathBuf, bool)> {
        let named = self
            .music_names
            .get(&track)
            .and_then(|entry| Some((self.tracks.get(&entry.name)?.clone(), entry.looping)));
        named
            .or_else(|| {
                // Nothing states whether a pack's own track repeats. Region
                // music is the overwhelming majority of what a shard sends, and
                // a region left silent after three minutes is the worse of the
                // two mistakes, so it repeats.
                numeric_names(track)
                    .iter()
                    .find_map(|name| self.tracks.get(name).cloned())
                    .map(|path| (path, true))
            })
            .or_else(|| {
                let entry = classic_track(track)?;
                Some((self.tracks.get(&entry.name)?.clone(), entry.looping))
            })
    }

    /// Start a looping track over once it has played to its end.
    ///
    /// The loop is here, and not in `Source::repeat_infinite`, because that
    /// wraps the track in rodio's `Buffered`, and `Buffered` asks a source how
    /// long its current span is *before* pulling a sample from it. A freshly
    /// opened Symphonia decoder answers `Some(0)` — it has not read a packet
    /// yet — which `Buffered` reads as a stream that has already ended. The
    /// repeat is then an infinity of silence: the player reports a queued track,
    /// playing, at full volume, and the device receives zeroes. Priming the
    /// decoder with one sample would dodge it, and would leave the silence one
    /// upstream change away from coming back.
    fn repeat_finished_track(&mut self) {
        if !self.music.empty() {
            return;
        }
        let Some(path) = self.looping.clone() else {
            return;
        };
        let Some(source) = decode(&path) else {
            // A track that has stopped decoding will not start doing so on the
            // next frame, and a message every frame is not a diagnostic.
            self.looping = None;
            return;
        };
        self.music.append(source);
        self.music.play();
    }
}

/// Open and decode a music file, reporting the failure once at the seam that
/// knows the path.
#[cfg(not(target_arch = "wasm32"))]
fn decode(path: &Path) -> Option<rodio::Decoder<std::io::BufReader<std::fs::File>>> {
    match std::fs::File::open(path)
        .and_then(|file| rodio::Decoder::try_from(file).map_err(std::io::Error::other))
    {
        Ok(source) => Some(source),
        Err(error) => {
            eprintln!("audio: cannot play {}: {error}", path.display());
            None
        }
    }
}

/// Replace whatever the music player is playing with `source`.
///
/// The three calls are one operation, and the order is not a style choice.
/// `Player::clear` *pauses* the player as well as emptying it, and `append`
/// lifts only the stopped flag — never the paused one. A track handed over
/// without the closing `play` therefore queues itself behind a pause nothing
/// ever lifts: the very first `0x6D` of a session silences music for the rest
/// of it, with no error anywhere to say so, because every layer below did
/// exactly what it was asked.
#[cfg(not(target_arch = "wasm32"))]
fn start_track(player: &rodio::Player, source: impl rodio::Source + Send + 'static) {
    player.clear();
    player.append(source);
    player.play();
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
fn music_names(client_dir: &Path) -> HashMap<MusicId, Track> {
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
    contents.lines().filter_map(music_line).collect()
}

/// One `Config.txt` line: an id, a filename, and the word `loop` when the track
/// is meant to play until something else replaces it — `9 britainpos,loop`.
///
/// The separators are all three the file has been seen to use, and a line that
/// does not begin with a number is not an entry.
#[cfg(not(target_arch = "wasm32"))]
fn music_line(line: &str) -> Option<(MusicId, Track)> {
    let mut fields = line.split([' ', ',', '\t']).filter(|field| !field.is_empty());
    let id = fields.next()?.parse().ok()?;
    let name = Path::new(fields.next()?)
        .file_stem()?
        .to_str()?
        .to_ascii_lowercase();
    let looping = fields.any(|field| field.eq_ignore_ascii_case("loop"));
    Some((MusicId(id), Track { name, looping }))
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

/// A file named after the id itself, in the two spellings a pack has been seen
/// to use. Tried before the classic table so a shard can ship its own music
/// without the client needing a protocol for it.
#[cfg(not(target_arch = "wasm32"))]
fn numeric_names(track: MusicId) -> [String; 2] {
    [track.0.to_string(), format!("{:02}", track.0)]
}

/// What every install has played since 1997, for one that ships no config.
///
/// Names and loop flags both come from the reference's own fallback table
/// (`ClassicUO.Assets/SoundsLoader.cs`), because the flag is per track and not
/// per kind: `britain1` repeats, `victory` plays once, and guessing either way
/// gets one of them wrong.
#[cfg(not(target_arch = "wasm32"))]
fn classic_track(track: MusicId) -> Option<Track> {
    const CLASSIC: &[(&str, bool)] = &[
        ("oldult01", true),
        ("create1", false),
        ("dragflit", false),
        ("oldult02", true),
        ("oldult03", true),
        ("oldult04", true),
        ("oldult05", true),
        ("oldult06", true),
        ("stones2", true),
        ("britain1", true),
        ("britain2", true),
        ("bucsden", true),
        ("jhelom", false),
        ("lbcastle", false),
        ("linelle", false),
        ("magincia", true),
        ("minoc", true),
        ("ocllo", true),
        ("samlethe", false),
        ("serpents", true),
        ("skarabra", true),
        ("trinsic", true),
        ("vesper", true),
        ("wind", true),
        ("yew", true),
        ("cave01", false),
        ("dungeon9", false),
        ("forest_a", false),
        ("intown01", false),
        ("jungle_a", false),
        ("mountn_a", false),
        ("plains_a", false),
        ("sailing", false),
        ("swamp_a", false),
        ("tavern01", false),
        ("tavern02", false),
        ("tavern03", false),
        ("tavern04", false),
        ("combat1", false),
        ("combat2", false),
        ("combat3", false),
        ("approach", false),
        ("death", false),
        ("victory", false),
        ("btcastle", false),
        ("nujelm", true),
        ("dungeon2", false),
        ("cove", true),
        ("moonglow", true),
        ("zento", true),
        ("tokunodungeon", true),
        ("taiko", true),
        ("dreadhornarea", true),
        ("elfcity", true),
        ("grizzledungeon", true),
        ("melisandeslair", true),
        ("paroxysmuslair", true),
        ("gwennoconversation", true),
        ("goodendgame", true),
        ("goodvsevil", true),
        ("greatearthserpents", true),
        ("humanoids_u9", true),
        ("minocnegative", true),
        ("paws", true),
        ("selimsbar", true),
        ("serpentislecombat_u7", true),
        ("valoriaships", true),
    ];
    let (name, looping) = CLASSIC.get(usize::from(track.0))?;
    Some(Track {
        name: (*name).to_owned(),
        looping: *looping,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::num::{NonZeroU16, NonZeroU32};

    use openshard_protocol::world::MusicId;

    /// A short buffer of silence — enough to be a source, and nothing is ever
    /// asked to play it.
    fn silence() -> rodio::buffer::SamplesBuffer {
        rodio::buffer::SamplesBuffer::new(
            NonZeroU16::new(1).expect("one channel"),
            NonZeroU32::new(22050).expect("the classic rate"),
            vec![0.0; 32],
        )
    }

    /// A source shaped like a freshly opened decoder: it cannot say how long
    /// its current span is until it has decoded something, so it answers
    /// `Some(0)` until the first sample has been pulled.
    ///
    /// That shape is the whole trap. `Source::repeat_infinite` buffers, and
    /// `Buffered` reads `Some(0)` as a stream that has already ended, so the
    /// samples below never reach the queue at all.
    #[derive(Default)]
    struct UnreadDecoder {
        produced: usize,
    }

    /// Long enough to outlast the queue's own silence, short enough to be free.
    const DECODED_SAMPLES: usize = 512;

    impl Iterator for UnreadDecoder {
        type Item = rodio::Sample;

        fn next(&mut self) -> Option<Self::Item> {
            (self.produced < DECODED_SAMPLES).then(|| {
                self.produced += 1;
                0.5
            })
        }
    }

    impl rodio::Source for UnreadDecoder {
        fn current_span_len(&self) -> Option<usize> {
            // Nothing decoded yet, so nothing is known about the span — which
            // is the same `Some(0)` a real decoder answers, and is not the same
            // statement as "this stream has ended", though `Buffered` reads it
            // as one.
            match self.produced {
                0 => Some(0),
                produced => Some(DECODED_SAMPLES - produced),
            }
        }

        fn channels(&self) -> rodio::ChannelCount {
            NonZeroU16::new(1).expect("one channel")
        }

        fn sample_rate(&self) -> rodio::SampleRate {
            NonZeroU32::new(22050).expect("the classic rate")
        }

        fn total_duration(&self) -> Option<std::time::Duration> {
            None
        }
    }

    /// What the player is handed has to arrive at the mixer as sound.
    ///
    /// The silence this catches had every symptom of working: a queued track, a
    /// player that is not paused, a volume of 0.45 and a device stream running.
    /// Only the samples were missing, so only the samples are asserted.
    #[test]
    fn a_track_reaches_the_queue_as_a_signal() {
        let (player, queue) = rodio::Player::new();
        super::start_track(&player, UnreadDecoder::default());
        let peak = queue
            .take(DECODED_SAMPLES * 4)
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak > 0.0, "the queue produced silence: peak {peak}");
    }

    /// The same claim against the installed music, which is the only place the
    /// real decoder can be exercised. Ignored by default: it needs a client.
    #[test]
    #[ignore = "needs an installed client — set OPENSHARD_CLIENT"]
    fn the_installed_track_reaches_the_queue_as_a_signal() {
        let dir = std::env::var("OPENSHARD_CLIENT").expect("OPENSHARD_CLIENT names an install");
        let path = std::path::Path::new(&dir).join("Music/Digital/Britainpos.mp3");
        let source = super::decode(&path).expect("the installed track decodes");
        let (player, queue) = rodio::Player::new();
        super::start_track(&player, source);
        let peak = queue
            .take(200_000)
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak > 0.001, "the queue produced silence: peak {peak}");
    }

    /// The loop flag is per track and decides whether the client starts it
    /// again, so it has to survive the line it is written on.
    #[test]
    fn a_config_line_carries_its_loop_flag() {
        assert_eq!(
            super::music_line("9 britainpos,loop"),
            Some((
                MusicId(9),
                super::Track {
                    name: "britainpos".to_owned(),
                    looping: true
                }
            ))
        );
        assert_eq!(
            super::music_line("10 britain1"),
            Some((
                MusicId(10),
                super::Track {
                    name: "britain1".to_owned(),
                    looping: false
                }
            ))
        );
        assert_eq!(super::music_line(""), None);
        assert_eq!(super::music_line("; a comment"), None);
    }

    /// The regression that made every session silent: `Player::clear` pauses,
    /// so a track appended after it never plays unless the player is resumed.
    ///
    /// `Player::new` builds the queue without touching a device, which is what
    /// lets the trap be caught on a machine with no sound card at all — the
    /// condition the whole mixer was written to keep.
    #[test]
    fn a_started_track_is_not_left_paused() {
        let (player, _queue) = rodio::Player::new();
        super::start_track(&player, silence());
        assert!(
            !player.is_paused(),
            "the music player is paused by `clear` and must be resumed after the track is queued"
        );
    }
}
