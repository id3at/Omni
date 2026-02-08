use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScaleType {
    // Basic
    Chromatic,
    Major,
    Minor,
    
    // Major Modes
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    
    // Harmonic/Melodic
    HarmonicMinor,
    MelodicMinor,
    MajorLocrian,
    SuperLocrian, // Altered
    
    // Pentatonic/Blues
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    MajorBlues,
    
    // Symmetric
    WholeTone,
    DiminishedWholeHalf,
    DiminishedHalfWhole,
    Augmented,
    
    // Exotic/World
    HungarianMinor,
    Enigmatic,
    Persian,
    Hirojoshi,
    Iwato,
    Kumoi,
    InSen,
    Pelog,
    Hirajoshi,
    
    // Bebop
    BebopDominant,
    BebopMajor,
}

impl Default for ScaleType {
    fn default() -> Self {
        Self::Chromatic
    }
}

impl ScaleType {
    pub fn get_intervals(&self) -> &'static [u8] {
        match self {
            ScaleType::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            ScaleType::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleType::Minor => &[0, 2, 3, 5, 7, 8, 10],
            
            ScaleType::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            ScaleType::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            ScaleType::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            ScaleType::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            ScaleType::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            
            ScaleType::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            ScaleType::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            ScaleType::MajorLocrian => &[0, 2, 4, 5, 6, 8, 10],
            ScaleType::SuperLocrian => &[0, 1, 3, 4, 6, 8, 10],
            
            ScaleType::MajorPentatonic => &[0, 2, 4, 7, 9],
            ScaleType::MinorPentatonic => &[0, 3, 5, 7, 10],
            ScaleType::Blues => &[0, 3, 5, 6, 7, 10],
            ScaleType::MajorBlues => &[0, 2, 3, 4, 7, 9],
            
            ScaleType::WholeTone => &[0, 2, 4, 6, 8, 10],
            ScaleType::DiminishedWholeHalf => &[0, 2, 3, 5, 6, 8, 9, 11],
            ScaleType::DiminishedHalfWhole => &[0, 1, 3, 4, 6, 7, 9, 10],
            ScaleType::Augmented => &[0, 3, 4, 7, 8, 11],
            
            ScaleType::HungarianMinor => &[0, 2, 3, 6, 7, 8, 11],
            ScaleType::Enigmatic => &[0, 1, 4, 6, 8, 10, 11],
            ScaleType::Persian => &[0, 1, 4, 5, 6, 8, 11],
            ScaleType::Hirojoshi => &[0, 2, 3, 7, 8],
            ScaleType::Iwato => &[0, 1, 5, 6, 10],
            ScaleType::Kumoi => &[0, 2, 3, 7, 9],
            ScaleType::InSen => &[0, 1, 5, 7, 10],
            ScaleType::Pelog => &[0, 1, 3, 7, 8],
            ScaleType::Hirajoshi => &[0, 2, 3, 7, 8], // Same as Hirojoshi? Often conflated.
            
            ScaleType::BebopDominant => &[0, 2, 4, 5, 7, 9, 10, 11],
            ScaleType::BebopMajor => &[0, 2, 4, 5, 7, 8, 9, 11],
        }
    }

    pub fn iter() -> impl Iterator<Item = ScaleType> {
        [
            ScaleType::Chromatic,
            ScaleType::Major,
            ScaleType::Minor,
            ScaleType::Dorian,
            ScaleType::Phrygian,
            ScaleType::Lydian,
            ScaleType::Mixolydian,
            ScaleType::Locrian,
            ScaleType::HarmonicMinor,
            ScaleType::MelodicMinor,
            ScaleType::MajorLocrian,
            ScaleType::SuperLocrian,
            ScaleType::MajorPentatonic,
            ScaleType::MinorPentatonic,
            ScaleType::Blues,
            ScaleType::MajorBlues,
            ScaleType::WholeTone,
            ScaleType::DiminishedWholeHalf,
            ScaleType::DiminishedHalfWhole,
            ScaleType::Augmented,
            ScaleType::HungarianMinor,
            ScaleType::Enigmatic,
            ScaleType::Persian,
            ScaleType::Hirojoshi,
            ScaleType::Iwato,
            ScaleType::Kumoi,
            ScaleType::InSen,
            ScaleType::Pelog,
            ScaleType::Hirajoshi,
            ScaleType::BebopDominant,
            ScaleType::BebopMajor,
        ].into_iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChordType {
    None,
    Major,
    Minor,
    Diminished,
    Sus2,
    Sus4,
    Maj7,
    Min7,
    Dom7,
    Major9,
    Minor9,
    Dom9,
}

impl Default for ChordType {
    fn default() -> Self {
        Self::None
    }
}

impl ChordType {
    /// Constant array of all chord types for O(1) index lookup (no heap allocation).
    const ALL: [ChordType; 12] = [
        ChordType::None, ChordType::Major, ChordType::Minor, ChordType::Diminished,
        ChordType::Sus2, ChordType::Sus4, ChordType::Maj7, ChordType::Min7,
        ChordType::Dom7, ChordType::Major9, ChordType::Minor9, ChordType::Dom9,
    ];

    /// O(1) lookup by index — RT-safe, zero allocation.
    pub fn from_index(idx: usize) -> Option<ChordType> {
        Self::ALL.get(idx).copied()
    }

    pub fn get_intervals(&self) -> &'static [u8] {
        match self {
            ChordType::None => &[],
            ChordType::Major => &[0, 4, 7],
            ChordType::Minor => &[0, 3, 7],
            ChordType::Diminished => &[0, 3, 6],
            ChordType::Sus2 => &[0, 2, 7],
            ChordType::Sus4 => &[0, 5, 7],
            ChordType::Maj7 => &[0, 4, 7, 11],
            ChordType::Min7 => &[0, 3, 7, 10],
            ChordType::Dom7 => &[0, 4, 7, 10],
            ChordType::Major9 => &[0, 4, 7, 11, 14],
            ChordType::Minor9 => &[0, 3, 7, 10, 14],
            ChordType::Dom9 => &[0, 4, 7, 10, 14],
        }
    }
    
    pub fn iter() -> impl Iterator<Item = ChordType> {
        [
            ChordType::None,
            ChordType::Major,
            ChordType::Minor,
            ChordType::Diminished,
            ChordType::Sus2,
            ChordType::Sus4,
            ChordType::Maj7,
            ChordType::Min7,
            ChordType::Dom7,
            ChordType::Major9,
            ChordType::Minor9,
            ChordType::Dom9,
        ].into_iter()
    }
    
    pub fn name(&self) -> &'static str {
        match self {
            ChordType::None => "None",
            ChordType::Major => "Maj",
            ChordType::Minor => "Min",
            ChordType::Diminished => "Dim",
            ChordType::Sus2 => "Sus2",
            ChordType::Sus4 => "Sus4",
            ChordType::Maj7 => "Maj7",
            ChordType::Min7 => "Min7",
            ChordType::Dom7 => "Dom7",
            ChordType::Major9 => "Maj9",
            ChordType::Minor9 => "Min9",
            ChordType::Dom9 => "Dom9",
        }
    }
}

/// Quantizes a note to the nearest note in the scale relative to a root key.
/// 
/// # Arguments
/// * `note` - The MIDI note number to quantize (0-127)
/// * `root` - The root key (0-127), e.g., 60 for C3
/// * `scale` - The scale type to use
/// 
/// # Returns
/// The quantized MIDI note number.
pub fn quantize(note: u8, root: u8, scale: ScaleType) -> u8 {
    if scale == ScaleType::Chromatic {
        return note;
    }

    let intervals = scale.get_intervals();
    if intervals.is_empty() {
        return note;
    }

    let root_class = root % 12;
    let note_class = note % 12;
    // Calculate note class relative to root (0-11)
    let rel_class = (note_class + 12 - root_class) % 12;

    // Check if it's in scale
    if intervals.contains(&rel_class) {
        return note;
    }

    // Generate all Valid Notes in the vicinity of `note`.
    // Valid Note = k * 12 + (root_class + interval) % 12
    
    let octave = note / 12; 

    // Valid notes in current octave, previous, next.
    let mut best_note = note;
    let mut min_abs_dist = 1000;
    
    for offset in -1..=1 {
        let base_octave = (octave as i32 + offset).max(0) as u8;
        for &iv in intervals {
            let note_val = (base_octave as u32 * 12) + ((root_class + iv) % 12) as u32;
            if note_val > 127 { continue; }
            let note_u8 = note_val as u8;
            
            let dist = (note as i32 - note_u8 as i32).abs();
            if dist < min_abs_dist {
                min_abs_dist = dist;
                best_note = note_u8;
            }
        }
    }
    
    best_note
}
