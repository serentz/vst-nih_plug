//! Interface graphique (egui) : réglages, progression reconnue, passage en cours.

use crate::shared::{Shared, STATE_BUILDING, STATE_CAPTURING, STATE_PLAYING};
use crate::{key_name, Bars, HarmogenParams, Line};
use harmony_core::generate::rhythm::{BASS_PRESETS, CHORD_PRESETS};
use harmony_core::generate::Pattern;
use harmony_core::theory::note_name;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use nih_plug_egui::{create_egui_editor, widgets, EguiState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const WIDTH: u32 = 760;
pub const HEIGHT: u32 = 560;

const ACCENT: Color32 = Color32::from_rgb(0xE0, 0x9A, 0x3A);
const NOTE: Color32 = Color32::from_rgb(0x6F, 0xB8, 0xE8);
const CHORD_A: Color32 = Color32::from_rgb(0x34, 0x3A, 0x48);
const CHORD_B: Color32 = Color32::from_rgb(0x2B, 0x30, 0x3C);
const GRID: Color32 = Color32::from_rgb(0x3A, 0x3F, 0x4A);

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(WIDTH, HEIGHT)
}

pub fn create(params: Arc<HarmogenParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        (),
        |_, _| {},
        move |ctx, setter, _| {
            ctx.request_repaint();
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                header(ui, &shared);
                ui.separator();
                line_and_length(ui, &params, setter, &shared);
                ui.add_space(4.0);
                progression(ui, &params, setter, &shared);
                ui.add_space(4.0);
                piano_roll(ui, &shared);
                ui.add_space(4.0);
                settings(ui, &params, setter);
            });
        },
    )
}

fn header(ui: &mut egui::Ui, shared: &Shared) {
    let state = shared.state.load(Ordering::Relaxed);
    let pos = shared.position.load(Ordering::Relaxed);
    let length = shared.length.load(Ordering::Relaxed).max(1) as i32;
    let status = match state {
        STATE_CAPTURING if pos < 0 => "Capture : commence à la prochaine mesure".to_string(),
        STATE_CAPTURING => format!("Capture de la progression : mesure {} / {}", pos / 16 + 1, length / 16),
        STATE_BUILDING => "Calcul du modèle…".to_string(),
        STATE_PLAYING => format!(
            "Lecture : tour {}  —  mesure {} / {}",
            shared.playing_index.load(Ordering::Relaxed),
            pos / 16 + 1,
            length / 16
        ),
        _ => "En attente : lance la lecture pour capturer la progression".to_string(),
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Harmogen").size(22.0).strong().color(ACCENT));
        ui.add_space(12.0);
        ui.label(egui::RichText::new(status).size(14.0));
    });
}

fn set_param<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

fn line_and_length(ui: &mut egui::Ui, params: &HarmogenParams, setter: &ParamSetter, shared: &Shared) {
    ui.horizontal(|ui| {
        ui.label("Ligne");
        for (line, name) in [(Line::Melody, "Mélodie"), (Line::Bass, "Basse"), (Line::Chords, "Accords")] {
            if ui.selectable_label(params.line.value() == line, name).clicked() {
                set_param(setter, &params.line, line);
            }
        }
        ui.add_space(16.0);
        ui.label("Longueur");
        for (bars, name) in [(Bars::Four, "4"), (Bars::Eight, "8"), (Bars::Sixteen, "16")] {
            if ui.selectable_label(params.bars.value() == bars, name).clicked() {
                set_param(setter, &params.bars, bars);
            }
        }
        ui.label("mesures");
        ui.add_space(16.0);
        if ui.button("⟲ Réapprendre").on_hover_text("Nouvelle capture au prochain début de mesure").clicked() {
            shared.relearn_request.store(true, Ordering::Relaxed);
        }
        if ui.button("Retour au modèle").on_hover_text("Rejoue le modèle au prochain tour").clicked() {
            shared.model_request.store(true, Ordering::Relaxed);
        }
    });
}

fn progression(ui: &mut egui::Ui, params: &HarmogenParams, setter: &ParamSetter, shared: &Shared) {
    let (chords, key, alternatives, length) = match shared.display.lock() {
        Ok(d) => (d.chords.clone(), d.key.clone(), d.alternatives.clone(), d.length),
        Err(_) => return,
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Progression").strong());
        if chords.is_empty() {
            ui.label("pas encore capturée");
        } else {
            ui.label(format!("tonalité {key}"));
            if alternatives.len() > 1 {
                ui.label(egui::RichText::new(format!("(aussi : {})", alternatives[1..].join(", "))).weak());
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let current = params.key.value();
            egui::ComboBox::from_id_salt("key")
                .selected_text(key_name(current))
                .width(130.0)
                .show_ui(ui, |ui| {
                    for v in -1..=23 {
                        if ui.selectable_label(v == current, key_name(v)).clicked() {
                            set_param(setter, &params.key, v);
                        }
                    }
                });
            ui.label("Tonalité");
        });
    });

    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let painter = ui.painter_at(rect);
    if length > 0 {
        let x = |step: u32| rect.left() + rect.width() * step as f32 / length as f32;
        for (i, (start, len, symbol)) in chords.iter().enumerate() {
            let r = Rect::from_min_max(Pos2::new(x(*start), rect.top()), Pos2::new(x(start + len), rect.bottom()));
            painter.rect_filled(r.shrink(1.0), 3.0, if i % 2 == 0 { CHORD_A } else { CHORD_B });
            painter.text(r.center(), Align2::CENTER_CENTER, symbol, FontId::proportional(13.0), Color32::WHITE);
        }
    } else {
        painter.rect_filled(rect, 3.0, CHORD_B);
    }
}

/// Passage en cours de lecture, retrouvé par son étiquette.
fn current_pass(shared: &Shared) -> Option<(Pattern, u32)> {
    let d = shared.display.lock().ok()?;
    let generation = shared.playing_generation.load(Ordering::Relaxed);
    let index = shared.playing_index.load(Ordering::Relaxed);
    if generation != d.generation {
        // Nouveau modèle calculé, pas encore joué : on montre le premier passage à venir.
        return d.passes.front().map(|(i, p)| (p.clone(), *i));
    }
    d.passes.iter().find(|(i, _)| *i == index).map(|(i, p)| (p.clone(), *i))
}

fn piano_roll(ui: &mut egui::Ui, shared: &Shared) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 230.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(0x1E, 0x21, 0x28));
    let Some((pattern, index)) = current_pass(shared) else {
        painter.text(rect.center(), Align2::CENTER_CENTER, "Aucun passage", FontId::proportional(14.0), Color32::GRAY);
        return;
    };
    if pattern.length == 0 || pattern.notes.is_empty() {
        return;
    }
    let lo = pattern.notes.iter().map(|n| n.pitch).min().unwrap().saturating_sub(2);
    let hi = pattern.notes.iter().map(|n| n.pitch).max().unwrap().saturating_add(2);
    let rows = (hi - lo + 1) as f32;
    let roll = Rect::from_min_max(Pos2::new(rect.left() + 34.0, rect.top() + 4.0), rect.right_bottom() - Vec2::new(4.0, 4.0));
    let x = |step: u32| roll.left() + roll.width() * step as f32 / pattern.length as f32;
    let y = |pitch: u8| roll.bottom() - roll.height() * (pitch - lo + 1) as f32 / rows;
    let row_h = roll.height() / rows;

    // Mesures et temps.
    for beat in 0..=pattern.length / 4 {
        let color = if beat % 4 == 0 { GRID } else { Color32::from_rgb(0x2A, 0x2E, 0x37) };
        let xx = x(beat * 4);
        painter.line_segment([Pos2::new(xx, roll.top()), Pos2::new(xx, roll.bottom())], Stroke::new(1.0_f32, color));
    }
    // Noms des C pour se repérer.
    for pitch in lo..=hi {
        if pitch % 12 == 0 {
            painter.text(
                Pos2::new(rect.left() + 4.0, y(pitch) + row_h / 2.0),
                Align2::LEFT_CENTER,
                note_name(pitch),
                FontId::proportional(10.0),
                Color32::GRAY,
            );
        }
    }
    for n in &pattern.notes {
        let alpha = 110 + (n.velocity as u32 * 145 / 127) as u8;
        let r = Rect::from_min_max(Pos2::new(x(n.start), y(n.pitch)), Pos2::new(x(n.start + n.len), y(n.pitch) + row_h));
        painter.rect_filled(r.shrink(0.5), 2.0, Color32::from_rgba_unmultiplied(NOTE.r(), NOTE.g(), NOTE.b(), alpha));
    }
    if shared.state.load(Ordering::Relaxed) == STATE_PLAYING {
        let pos = shared.position.load(Ordering::Relaxed).max(0) as u32;
        let xx = x(pos.min(pattern.length));
        painter.line_segment([Pos2::new(xx, roll.top()), Pos2::new(xx, roll.bottom())], Stroke::new(2.0_f32, ACCENT));
    }
    painter.text(
        rect.right_top() + Vec2::new(-8.0, 6.0),
        Align2::RIGHT_TOP,
        if index == 1 { "modèle".to_string() } else { format!("tour {index}") },
        FontId::proportional(12.0),
        Color32::GRAY,
    );
}

fn rhythm_combo(ui: &mut egui::Ui, id: &str, names: &[&str], param: &IntParam, setter: &ParamSetter) {
    let current = param.value() as usize;
    egui::ComboBox::from_id_salt(id).selected_text(names[current % names.len()]).width(150.0).show_ui(ui, |ui| {
        for (i, name) in names.iter().enumerate() {
            if ui.selectable_label(i == current, *name).clicked() {
                set_param(setter, param, i as i32);
            }
        }
    });
}

fn settings(ui: &mut egui::Ui, params: &HarmogenParams, setter: &ParamSetter) {
    let bass: Vec<&str> = BASS_PRESETS.iter().map(|p| p.name).collect();
    let chords: Vec<&str> = CHORD_PRESETS.iter().map(|p| p.name).collect();
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(egui::RichText::new("Génération").strong());
        egui::Grid::new("gen").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Rythme basse");
            ui.add_enabled_ui(params.line.value() == Line::Bass, |ui| {
                rhythm_combo(ui, "bass", &bass, &params.bass_rhythm, setter)
            });
            ui.end_row();
            ui.label("Rythme accords");
            ui.add_enabled_ui(params.line.value() == Line::Chords, |ui| {
                rhythm_combo(ui, "chords", &chords, &params.chord_rhythm, setter)
            });
            ui.end_row();
            ui.label("Graine");
            ui.add(widgets::ParamSlider::for_param(&params.seed, setter));
            ui.end_row();
            ui.label("Renvoyer l'entrée");
            let mut thru = params.thru.value();
            if ui.checkbox(&mut thru, "").changed() {
                set_param(setter, &params.thru, thru);
            }
            ui.end_row();
        });

        let ui = &mut cols[1];
        ui.label(egui::RichText::new("Variations à chaque tour").strong());
        egui::Grid::new("vary").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for (label, param) in [
                ("Silence", &params.vary_silence),
                ("Hauteur ±1", &params.vary_pitch),
                ("Bloc ±1 (mélodie)", &params.vary_block),
                ("Vélocité", &params.vary_velocity),
                ("Durée", &params.vary_length),
            ] {
                ui.label(label);
                ui.add(widgets::ParamSlider::for_param(param, setter));
                ui.end_row();
            }
            ui.label("Retour au modèle");
            ui.add(widgets::ParamSlider::for_param(&params.reset_every, setter));
            ui.end_row();
        });
    });
}
