//! Lecture d'une bibliothèque de MIDI rangée par progression (type « songstarter ») :
//! un dossier par progression, nommé `<n>_<accords>_<degrés>` (ex. `3_Dm-F-C-Am-G_ii-IV-I-vi-V`),
//! contenant des sous-dossiers par famille de fichiers.
//!
//! Le nom du dossier fournit l'harmonie de référence ; seuls les noms d'accords sont
//! utilisés (les degrés romains sont parfois inexacts dans les bibliothèques).

use crate::theory::{PitchClass, NOTE_NAMES};

/// Famille d'accord, telle qu'écrite dans un nom de dossier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Major,
    Minor,
    Diminished,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NamedChord {
    pub root: PitchClass,
    pub family: Family,
}

/// Progression décrite par un nom de dossier.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NamedProgression {
    pub number: u32,
    pub chords: Vec<NamedChord>,
    pub numerals: String,
}

/// Lit un nom d'accord : `Dm`, `F#`, `Bb`, `Bdim`.
pub fn parse_chord_name(name: &str) -> Option<NamedChord> {
    let mut chars = name.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let mut root = NOTE_NAMES.iter().position(|n| n.len() == 1 && n.starts_with(letter))? as i32;
    let mut rest = chars.as_str();
    if let Some(r) = rest.strip_prefix('#') {
        root += 1;
        rest = r;
    } else if let Some(r) = rest.strip_prefix('b') {
        root -= 1;
        rest = r;
    }
    let family = match rest {
        "" => Family::Major,
        "m" => Family::Minor,
        "dim" => Family::Diminished,
        _ => return None,
    };
    Some(NamedChord { root: root.rem_euclid(12) as u8, family })
}

/// Lit un nom de dossier de progression : `3_Dm-F-C-Am-G_ii-IV-I-vi-V`.
pub fn parse_progression_name(name: &str) -> Option<NamedProgression> {
    let mut parts = name.splitn(3, '_');
    let number = parts.next()?.parse().ok()?;
    let chords = parts.next()?.split('-').map(parse_chord_name).collect::<Option<Vec<_>>>()?;
    let numerals = parts.next().unwrap_or("").to_string();
    (!chords.is_empty()).then_some(NamedProgression { number, chords, numerals })
}

/// Famille d'un fichier, déduite de son chemin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    ChordProgression,
    Bass,
    /// Tempo minimal du dossier de tempo (90, 115, 130, 160), 0 s'il est inconnu.
    Melody { bpm_min: u32 },
    FastChords,
    SlowChords,
    Arp,
}

impl FileKind {
    /// Classe un chemin (séparateurs `/` ou `\`). `None` pour un fichier hors catégorie.
    pub fn from_path(path: &str) -> Option<FileKind> {
        let parts: Vec<&str> = path.split(['/', '\\']).collect();
        let has = |dir: &str| parts.iter().any(|p| p.eq_ignore_ascii_case(dir));
        if has("Chord Progression") {
            Some(FileKind::ChordProgression)
        } else if has("Basslines") {
            Some(FileKind::Bass)
        } else if has("Melodies") {
            let bpm_min = parts
                .iter()
                .find_map(|p| p.strip_suffix("bpm").and_then(|r| r.split('-').next()).and_then(|n| n.parse().ok()))
                .unwrap_or(0);
            Some(FileKind::Melody { bpm_min })
        } else if has("Fast Chord Rhythm") {
            Some(FileKind::FastChords)
        } else if has("Slow Chord Rhythm") {
            Some(FileKind::SlowChords)
        } else if has("Arps") {
            Some(FileKind::Arp)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_names() {
        assert_eq!(parse_chord_name("Dm"), Some(NamedChord { root: 2, family: Family::Minor }));
        assert_eq!(parse_chord_name("Bb"), Some(NamedChord { root: 10, family: Family::Major }));
        assert_eq!(parse_chord_name("F#m"), Some(NamedChord { root: 6, family: Family::Minor }));
        assert_eq!(parse_chord_name("Bdim"), Some(NamedChord { root: 11, family: Family::Diminished }));
        assert_eq!(parse_chord_name("Cb"), Some(NamedChord { root: 11, family: Family::Major }));
        assert_eq!(parse_chord_name("Hm"), None);
    }

    #[test]
    fn progression_names() {
        let p = parse_progression_name("3_Dm-F-C-Am-G_ii-IV-I-vi-V").unwrap();
        assert_eq!(p.number, 3);
        assert_eq!(p.chords.len(), 5);
        assert_eq!(p.numerals, "ii-IV-I-vi-V");
        // Degrés avec un « _ » en trop : seuls les accords comptent.
        let p = parse_progression_name("26_Am-C-F-Dm-E_vi-I-IV-ii_III").unwrap();
        assert_eq!(p.chords.last(), Some(&NamedChord { root: 4, family: Family::Major }));
        assert!(parse_progression_name("Basslines").is_none());
    }

    #[test]
    fn file_kinds() {
        assert_eq!(FileKind::from_path(r"C\3_x\Chord Progression\a.mid"), Some(FileKind::ChordProgression));
        assert_eq!(FileKind::from_path("C/3_x/Melodies/130-160bpm/Wet/a.mid"), Some(FileKind::Melody { bpm_min: 130 }));
        assert_eq!(FileKind::from_path("C/3_x/Fast Chord Rhythm/Plucks/a.mid"), Some(FileKind::FastChords));
        assert_eq!(FileKind::from_path("C/3_x/Arps/Dry/a.mid"), Some(FileKind::Arp));
        assert_eq!(FileKind::from_path("C/3_x/readme.txt"), None);
    }
}
