//! Message type codes for the remote database protocol.

/// A message type code. Requests and responses share the same 16-bit space.
pub type MessageType = u16;

/// Used for control messages with the remote database.
pub mod control_request {
    pub const INTRODUCE: u16 = 0x0000;
    pub const DISCONNECT: u16 = 0x0100;
    pub const RENDER_MENU: u16 = 0x3000;
}

/// Used to setup renders for specific Menus.
pub mod menu_request {
    pub const MENU_ROOT: u16 = 0x1000;
    pub const MENU_GENRE: u16 = 0x1001;
    pub const MENU_ARTIST: u16 = 0x1002;
    pub const MENU_ALBUM: u16 = 0x1003;
    pub const MENU_TRACK: u16 = 0x1004;
    pub const MENU_BPM: u16 = 0x1006;
    pub const MENU_RATING: u16 = 0x1007;
    pub const MENU_YEAR: u16 = 0x1008;
    pub const MENU_LABEL: u16 = 0x100a;
    pub const MENU_COLOR: u16 = 0x100d;
    pub const MENU_TIME: u16 = 0x1010;
    pub const MENU_BITRATE: u16 = 0x1011;
    pub const MENU_HISTORY: u16 = 0x1012;
    pub const MENU_FILENAME: u16 = 0x1013;
    pub const MENU_KEY: u16 = 0x1014;
    pub const MENU_ORIGINAL_ARTIST: u16 = 0x1302;
    pub const MENU_REMIXER: u16 = 0x1602;
    pub const MENU_PLAYLIST: u16 = 0x1105;
    pub const MENU_ARTISTS_OF_GENRE: u16 = 0x1101;
    pub const MENU_ALBUMS_OF_ARTIST: u16 = 0x1102;
    pub const MENU_TRACKS_OF_ALBUM: u16 = 0x1103;
    pub const MENU_TRACKS_OF_RATING: u16 = 0x1107;
    pub const MENU_YEARS_OF_DECADE: u16 = 0x1108;
    pub const MENU_ARTISTS_OF_LABEL: u16 = 0x110a;
    pub const MENU_TRACKS_OF_COLOR: u16 = 0x110d;
    pub const MENU_TRACKS_OF_TIME: u16 = 0x1110;
    pub const MENU_TRACKS_OF_HISTORY: u16 = 0x1112;
    pub const MENU_DISTANCES_OF_KEY: u16 = 0x1114;
    pub const MENU_ALBUMS_OF_ORIGINAL_ARTIST: u16 = 0x1402;
    pub const MENU_ALBUMS_OF_REMIXER: u16 = 0x1702;
    pub const MENU_ALBUMS_OF_GENRE_AND_ARTIST: u16 = 0x1201;
    pub const MENU_TRACKS_OF_ARTIST_AND_ALBUM: u16 = 0x1202;
    pub const MENU_TRACKS_OF_BPM_PERCENT_RANGE: u16 = 0x1206;
    pub const MENU_TRACKS_OF_DECADE_AND_YEAR: u16 = 0x1208;
    pub const MENU_ALBUMS_OF_LABEL_AND_ARTIST: u16 = 0x120a;
    pub const MENU_TRACKS_NEAR_KEY: u16 = 0x1214;
    pub const MENU_TRACKS_OF_ORIGINAL_ARTIST_AND_ALBUM: u16 = 0x1502;
    pub const MENU_TRACKS_OF_REMIXER_AND_ALBUM: u16 = 0x1802;
    pub const MENU_TRACKS_OF_GENRE_ARTIST_AND_ALBUM: u16 = 0x1301;
    pub const MENU_TRACKS_OF_LABEL_ARTIST_AND_ALBUM: u16 = 0x130a;
    pub const MENU_SEARCH: u16 = 0x1300;
    pub const MENU_FOLDER: u16 = 0x2006;
}

/// Request message types used to obtain specific track information.
pub mod data_request {
    pub const GET_METADATA: u16 = 0x2002;
    pub const GET_ARTWORK: u16 = 0x2003;
    pub const GET_WAVEFORM_PREVIEW: u16 = 0x2004;
    pub const GET_TRACK_INFO: u16 = 0x2102;
    pub const GET_GENERIC_METADATA: u16 = 0x2202;
    pub const GET_CUE_AND_LOOPS: u16 = 0x2104;
    pub const GET_BEAT_GRID: u16 = 0x2204;
    pub const GET_WAVEFORM_DETAILED: u16 = 0x2904;
    pub const GET_ADV_CUE_AND_LOOPS: u16 = 0x2b04;
    pub const GET_WAVEFORM_HD: u16 = 0x2c04;
}

/// Response message types for messages sent back by the server.
pub mod response {
    pub const SUCCESS: u16 = 0x4000;
    pub const ERROR: u16 = 0x4003;
    pub const ARTWORK: u16 = 0x4002;
    pub const MENU_ITEM: u16 = 0x4101;
    pub const MENU_HEADER: u16 = 0x4001;
    pub const MENU_FOOTER: u16 = 0x4201;
    pub const BEAT_GRID: u16 = 0x4602;
    pub const CUE_AND_LOOP: u16 = 0x4702;
    pub const WAVEFORM_PREVIEW: u16 = 0x4402;
    pub const WAVEFORM_DETAILED: u16 = 0x4a02;
    pub const ADV_CUE_AND_LOOPS: u16 = 0x4e02;
    pub const WAVEFORM_HD: u16 = 0x4f02;
}

/// True for the response message types.
pub const fn is_response(t: MessageType) -> bool {
    matches!(
        t,
        response::SUCCESS
            | response::ERROR
            | response::ARTWORK
            | response::MENU_ITEM
            | response::MENU_HEADER
            | response::MENU_FOOTER
            | response::BEAT_GRID
            | response::CUE_AND_LOOP
            | response::WAVEFORM_PREVIEW
            | response::WAVEFORM_DETAILED
            | response::ADV_CUE_AND_LOOPS
            | response::WAVEFORM_HD
    )
}

/// Returns a string representation of a message type.
pub fn message_name(t: MessageType) -> String {
    let name = match t {
        control_request::INTRODUCE => "Introduce",
        control_request::DISCONNECT => "Disconnect",
        control_request::RENDER_MENU => "RenderMenu",
        menu_request::MENU_ROOT => "MenuRoot",
        menu_request::MENU_GENRE => "MenuGenre",
        menu_request::MENU_ARTIST => "MenuArtist",
        menu_request::MENU_ALBUM => "MenuAlbum",
        menu_request::MENU_TRACK => "MenuTrack",
        menu_request::MENU_BPM => "MenuBPM",
        menu_request::MENU_RATING => "MenuRating",
        menu_request::MENU_YEAR => "MenuYear",
        menu_request::MENU_LABEL => "MenuLabel",
        menu_request::MENU_COLOR => "MenuColor",
        menu_request::MENU_TIME => "MenuTime",
        menu_request::MENU_BITRATE => "MenuBitrate",
        menu_request::MENU_HISTORY => "MenuHistory",
        menu_request::MENU_FILENAME => "MenuFilename",
        menu_request::MENU_KEY => "MenuKey",
        menu_request::MENU_ORIGINAL_ARTIST => "MenuOriginalArtist",
        menu_request::MENU_REMIXER => "MenuRemixer",
        menu_request::MENU_PLAYLIST => "MenuPlaylist",
        menu_request::MENU_ARTISTS_OF_GENRE => "MenuArtistsOfGenre",
        menu_request::MENU_ALBUMS_OF_ARTIST => "MenuAlbumsOfArtist",
        menu_request::MENU_TRACKS_OF_ALBUM => "MenuTracksOfAlbum",
        menu_request::MENU_TRACKS_OF_RATING => "MenuTracksOfRating",
        menu_request::MENU_YEARS_OF_DECADE => "MenuYearsOfDecade",
        menu_request::MENU_ARTISTS_OF_LABEL => "MenuArtistsOfLabel",
        menu_request::MENU_TRACKS_OF_COLOR => "MenuTracksOfColor",
        menu_request::MENU_TRACKS_OF_TIME => "MenuTracksOfTime",
        menu_request::MENU_TRACKS_OF_HISTORY => "MenuTracksOfHistory",
        menu_request::MENU_DISTANCES_OF_KEY => "MenuDistancesOfKey",
        menu_request::MENU_ALBUMS_OF_ORIGINAL_ARTIST => "MenuAlbumsOfOriginalArtist",
        menu_request::MENU_ALBUMS_OF_REMIXER => "MenuAlbumsOfRemixer",
        menu_request::MENU_ALBUMS_OF_GENRE_AND_ARTIST => "MenuAlbumsOfGenreAndArtist",
        menu_request::MENU_TRACKS_OF_ARTIST_AND_ALBUM => "MenuTracksOfArtistAndAlbum",
        menu_request::MENU_TRACKS_OF_BPM_PERCENT_RANGE => "MenuTracksOfBPMPercentRange",
        menu_request::MENU_TRACKS_OF_DECADE_AND_YEAR => "MenuTracksOfDecadeAndYear",
        menu_request::MENU_ALBUMS_OF_LABEL_AND_ARTIST => "MenuAlbumsOfLabelAndArtist",
        menu_request::MENU_TRACKS_NEAR_KEY => "MenuTracksNearKey",
        menu_request::MENU_TRACKS_OF_ORIGINAL_ARTIST_AND_ALBUM => "MenuTracksOfOriginalArtistAndAlbum",
        menu_request::MENU_TRACKS_OF_REMIXER_AND_ALBUM => "MenuTracksOfRemixerAndAlbum",
        menu_request::MENU_TRACKS_OF_GENRE_ARTIST_AND_ALBUM => "MenuTracksOfGenreArtistAndAlbum",
        menu_request::MENU_TRACKS_OF_LABEL_ARTIST_AND_ALBUM => "MenuTracksOfLabelArtistAndAlbum",
        menu_request::MENU_SEARCH => "MenuSearch",
        menu_request::MENU_FOLDER => "MenuFolder",
        data_request::GET_METADATA => "GetMetadata",
        data_request::GET_ARTWORK => "GetArtwork",
        data_request::GET_WAVEFORM_PREVIEW => "GetWaveformPreview",
        data_request::GET_TRACK_INFO => "GetTrackInfo",
        data_request::GET_GENERIC_METADATA => "GetGenericMetadata",
        data_request::GET_CUE_AND_LOOPS => "GetCueAndLoops",
        data_request::GET_BEAT_GRID => "GetBeatGrid",
        data_request::GET_WAVEFORM_DETAILED => "GetWaveformDetailed",
        data_request::GET_ADV_CUE_AND_LOOPS => "GetAdvCueAndLoops",
        data_request::GET_WAVEFORM_HD => "GetWaveformHD",
        response::SUCCESS => "Success",
        response::ERROR => "Error",
        response::ARTWORK => "Artwork",
        response::MENU_ITEM => "MenuItem",
        response::MENU_HEADER => "MenuHeader",
        response::MENU_FOOTER => "MenuFooter",
        response::BEAT_GRID => "BeatGrid",
        response::CUE_AND_LOOP => "CueAndLoop",
        response::WAVEFORM_PREVIEW => "WaveformPreview",
        response::WAVEFORM_DETAILED => "WaveformDetailed",
        response::ADV_CUE_AND_LOOPS => "AdvCueAndLoops",
        response::WAVEFORM_HD => "WaveformHD",
        _ => return format!("0x{t:04x}"),
    };
    name.to_string()
}
