//! Raw collection snapshot sent by the local reader; the server owns all interpretation.

use serde::{Deserialize, Serialize};

/// Every row the importer needs from one Anki collection, copied without interpretation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    /// Collection identity and format.
    pub(super) collection: Collection,
    /// Decks by identifier and full name.
    pub(super) decks: Vec<Deck>,
    /// Note types with field names in ordinal order.
    pub(super) notetypes: Vec<Notetype>,
    /// Notes with raw field HTML.
    pub(super) notes: Vec<Note>,
    /// Cards with their Anki scheduling state.
    pub(super) cards: Vec<Card>,
    /// The complete review log.
    pub(super) revlog: Vec<Revlog>,
    /// Content addresses of referenced media files only.
    pub(super) media: Vec<Media>,
}

/// Collection identity that survives sync between devices.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Collection {
    /// Creation time in seconds; identifies the collection across imports.
    pub(super) crt: i64,
    /// Last modification in milliseconds.
    pub(super) modified: i64,
    /// Database schema version.
    pub(super) schema: i64,
    /// Profile folder name.
    pub(super) profile: String,
    /// Anki application version when the reader could determine it.
    pub(super) application: String,
}

/// One deck.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Deck {
    /// Anki deck identifier.
    pub(super) id: i64,
    /// Full deck name.
    pub(super) name: String,
}

/// One note type.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Notetype {
    /// Anki note type identifier.
    pub(super) id: i64,
    /// Display name.
    pub(super) name: String,
    /// Field names in ordinal order.
    pub(super) fields: Vec<String>,
}

/// One note with raw field values.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Note {
    /// Anki note identifier.
    pub(super) id: i64,
    /// Globally unique note identity used by APKG import.
    pub(super) guid: String,
    /// Note type identifier.
    pub(super) notetype: i64,
    /// Last modification in seconds.
    pub(super) modified: i64,
    /// Space-separated tags.
    pub(super) tags: String,
    /// Raw field values in ordinal order.
    pub(super) fields: Vec<String>,
}

/// One card and its Anki scheduling state.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Card {
    /// Anki card identifier.
    pub(super) id: i64,
    /// Owning note.
    pub(super) note: i64,
    /// Current deck.
    pub(super) deck: i64,
    /// Template ordinal.
    pub(super) ordinal: i64,
    /// Card type: 0 new, 1 learning, 2 review, 3 relearning.
    pub(super) kind: i64,
    /// Queue: 0 new, 1 learning, 2 review, 3 day learning, -1 suspended, -2/-3 buried.
    pub(super) queue: i64,
    /// Position, day number, or epoch seconds depending on the queue.
    pub(super) due: i64,
    /// Interval in days, negative seconds while learning.
    pub(super) interval: i64,
    /// SM-2 ease in permille.
    pub(super) factor: i64,
    /// Review count.
    pub(super) reps: i64,
    /// Lapse count.
    pub(super) lapses: i64,
}

/// One review log row.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Revlog {
    /// Review time in epoch milliseconds, unique within the collection.
    pub(super) id: i64,
    /// Reviewed card.
    pub(super) card: i64,
    /// Button: 1–4, or 0 for manual rows.
    pub(super) ease: i64,
    /// New interval: days, or negative seconds.
    pub(super) interval: i64,
    /// Previous interval.
    pub(super) last_interval: i64,
    /// Ease factor; zero for resets and cram rows.
    pub(super) factor: i64,
    /// Answer time in milliseconds.
    pub(super) time: i64,
    /// 0 learn, 1 review, 2 relearn, 3 filtered, 4 manual, 5 rescheduled.
    pub(super) kind: i64,
}

/// One referenced media file by content address.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Media {
    /// File name as referenced by note fields.
    pub(super) name: String,
    /// Lowercase hexadecimal SHA-256 of the bytes.
    pub(super) sha256: String,
    /// Size in bytes.
    pub(super) size: u64,
}
