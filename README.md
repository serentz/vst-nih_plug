# Harmogen

Instrument MIDI VST3 en Rust ([nih-plug](https://github.com/robbert-vdh/nih-plug)) pour Ableton Live :
il apprend une progression d'accords (4, 8 ou 16 mesures) et génère une mélodie, une ligne de basse
ou des accords rythmés accordés sur cette progression, avec des variations à chaque passage.

## Organisation

| Crate | Rôle |
|---|---|
| `harmony-core` | Moteur musical pur (sans dépendance au plugin) : notes, gammes, accords, analyse d'une progression |
| `cli` | Outil `harmogen` pour tester le moteur sur des fichiers MIDI, hors d'Ableton |
| `plugin` | Plugin VST3 (nih-plug) |
| `xtask` | Empaquetage du bundle VST3 |

## État

- **Phase 0 — faisabilité (validée dans Live 12.4.16)** : le plugin reçoit le MIDI de sa piste, envoie
  du MIDI vers une autre piste et lit le transport (tempo, position, mesures, boucle).
- **Phase 1 — analyse harmonique** : découpage de la progression au pas de double croche,
  reconnaissance d'accords (renversements, 7e, sus, couleurs add9/add11), estimation de la tonalité
  avec les tonalités relatives proposées comme alternatives.

## Analyse d'un fichier MIDI

```bash
cargo run -p harmogen-cli --release -- analyze progression.mid --bars 4
```

Options : `--bars N` (longueur de la boucle), `--min-pitch P` / `--max-pitch P` (filtrer les notes,
par exemple `--max-pitch 59` pour ne garder que l'accompagnement d'un fichier qui contient aussi une mélodie).

## Plugin

```bash
cargo xtask bundle harmogen --release
```

Le bundle est créé dans `target/bundled/harmogen.vst3` ; copier ce dossier dans
`C:\Program Files\Common Files\VST3\`.

Routage dans Live :
1. Piste 1 : charger `Harmogen Spike` et un clip d'accords.
2. Piste 2 : un synthé, *MIDI From* = `1-Piste 1`, puis dans le second menu **le nom de l'instance du plugin**
   (pas `Post FX`, qui ne transmet que les notes du clip), *Monitor* = `In`.

Paramètre `Mode` du prototype : `Pulse` (un C4 Live sur chaque temps, pour vérifier le routage) ou `Echo`
(notes reçues renvoyées avec la transposition `Transpose`).

## Tests

```bash
cargo test -p harmony-core
```
