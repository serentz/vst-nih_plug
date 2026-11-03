# Harmogen

Instrument MIDI VST3 en Rust ([nih-plug](https://github.com/robbert-vdh/nih-plug)) pour Ableton Live :
il apprend une progression d'accords (4, 8 ou 16 mesures) et génère une mélodie, une ligne de basse
ou des accords rythmés accordés sur cette progression, avec des variations à chaque passage.

## État : phase 0 — prototype de faisabilité

Le plugin `Harmogen Spike` :
- reçoit le MIDI de sa piste et le renvoie transposé (paramètre `Transpose`, +12 par défaut) ;
- écrit l'état du transport (tempo, position, mesure, signature, boucle) dans `%TEMP%\harmogen_spike.log`.

## Compilation

```bash
cargo xtask bundle harmogen --release
```

Le bundle est créé dans `target/bundled/harmogen.vst3` ; copier ce dossier dans
`C:\Program Files\Common Files\VST3\`.

## Test dans Ableton Live

1. Piste 1 : charger `Harmogen Spike` et un clip d'accords.
2. Piste 2 : un synthé, *MIDI From* = `1-Piste 1` / `Harmogen Spike`, *Monitor* = `In`.
3. Lancer la lecture : la piste 2 doit jouer les accords transposés.
