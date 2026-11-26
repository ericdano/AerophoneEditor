//! The editor's pages. Each takes the editor model and the view state.

use std::collections::HashSet;

use eframe::egui::{self, vec2, Align, Layout, RichText, Ui};

use ae30_core::params::{data, Param, Table};

use super::theme::{self, palette};
use super::widgets::{self, Env};
use super::{FxSel, Tab, View};
use crate::editor::{Editor, Target};

const CARD_W: f32 = 370.0;

/// A titled card of fixed width.
fn card(ui: &mut Ui, title: &str, width: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(width - 26.0);
            ui.label(theme::section(ui, title));
            ui.add_space(4.0);
            add(ui);
        });
    });
}

/// Cards laid out left to right, wrapping.
fn flow(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.with_layout(
        Layout::left_to_right(Align::Min).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
            add(ui);
        },
    );
}

fn params_at(table: &'static Table, offs: &[u32]) -> Vec<&'static Param> {
    offs.iter().filter_map(|&o| table.at(o)).collect()
}

/// A card with the parameters at `offs`, noting them in `used`.
fn group(
    ui: &mut Ui,
    ed: &mut Editor,
    t: Target,
    block: &str,
    title: &str,
    offs: &[u32],
    used: &mut HashSet<u32>,
) {
    let Some(table) = ed.image(t).block(block).map(|b| b.table) else {
        return;
    };
    // Each setting appears in only one card.
    let offs: Vec<u32> = offs.iter().copied().filter(|o| !used.contains(o)).collect();
    let ps = params_at(table, &offs);
    if ps.is_empty() {
        return;
    }
    used.extend(offs);
    card(ui, title, CARD_W, |ui| widgets::grid(ui, ed, t, block, ps));
}

/// Everything in `block` not yet shown.
fn leftovers(
    ui: &mut Ui,
    ed: &mut Editor,
    t: Target,
    block: &str,
    title: &str,
    used: &HashSet<u32>,
) {
    let Some(table) = ed.image(t).block(block).map(|b| b.table) else {
        return;
    };
    let rest: Vec<&'static Param> = table
        .fixed()
        .filter(|p| !used.contains(&p.off) && !p.name.starts_with("NAME") && p.name != "Name")
        .collect();
    if !rest.is_empty() {
        card(ui, title, CARD_W, |ui| {
            widgets::grid(ui, ed, t, block, rest)
        });
    }
}

fn not_loaded(ui: &mut Ui, ed: &Editor) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        if ed.is_online() {
            ui.spinner();
            ui.label("Reading from the AE-30…");
        } else {
            ui.label(RichText::new("Not connected").weak());
        }
    });
}

// ---- Scene -------------------------------------------------------------------

pub fn scene(ui: &mut Ui, view: &mut View, ed: &mut Editor) {
    if !ed.scene.is_loaded() {
        return not_loaded(ui, ed);
    }
    let t = Target::Scene;
    let common = data().table("Perf_common");
    ui.label(theme::section(ui, "Parts"));
    ui.add_space(2.0);
    egui::ScrollArea::horizontal()
        .id_salt("mixer")
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for part in 1..=5u8 {
                    strip(ui, view, ed, part);
                }
            });
        });
    ui.add_space(12.0);
    let mut used = HashSet::new();
    flow(ui, |ui| {
        group(
            ui,
            ed,
            t,
            "Perf_common",
            "Scene",
            &[24, 16, 20, 21, 25],
            &mut used,
        );
        let pitch: Vec<u32> = common
            .fixed()
            .filter(|p| {
                !used.contains(&p.off) && (p.label().contains("Bend") || p.label().contains("Bite"))
            })
            .map(|p| p.off)
            .collect();
        group(ui, ed, t, "Perf_common", "Bend & Bite", &pitch, &mut used);
        let ctrl: Vec<u32> = common
            .fixed()
            .filter(|p| {
                !used.contains(&p.off)
                    && (p.label().contains("Control Source") || p.label().contains("Motion"))
            })
            .map(|p| p.off)
            .collect();
        group(ui, ed, t, "Perf_common", "Control", &ctrl, &mut used);
        let harm: Vec<u32> = common
            .fixed()
            .filter(|p| {
                !used.contains(&p.off)
                    && (p.label().contains("Harmony") || p.label().contains("Intelligent"))
            })
            .map(|p| p.off)
            .collect();
        group(ui, ed, t, "Perf_common", "Harmony", &harm, &mut used);
        leftovers(ui, ed, t, "Perf_common", "More", &used);
    });
}

/// One mixer channel strip.
fn strip(ui: &mut Ui, view: &mut View, ed: &mut Editor, part: u8) {
    let pal = palette(ui);
    let t = Target::Scene;
    let block = format!("Perf_part_{part}");
    let table = data().table("Perf_part");
    let on = ed.value(t, &block, table.at(1).unwrap()) == Some(1);
    let frame = theme::card(ui).fill(if on { pal.surface } else { pal.bg });
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(170.0);
            ui.spacing_mut().combo_width = 170.0;
            ui.horizontal(|ui| {
                ui.label(theme::title(format!("Part {part}"), 14.0).color(if on {
                    pal.text
                } else {
                    pal.weak
                }));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    widgets::control(ui, ed, t, &block, table.at(1).unwrap());
                });
            });
            let tone = ed.tone_label(part);
            let cat = ed
                .part_tone(part)
                .and_then(|(m, l, p)| data().tone_category(m, l, p))
                .unwrap_or("");
            let b =
                egui::Button::new(RichText::new(&tone).family(theme::semibold()).color(if on {
                    pal.text
                } else {
                    pal.weak
                }))
                .min_size(vec2(170.0, 28.0))
                .truncate();
            if ui.add(b).on_hover_text("Choose a tone").clicked() {
                view.tone_picker = Some(part);
            }
            ui.label(RichText::new(cat).size(10.5).color(pal.weak));
            widgets::control(ui, ed, t, &block, table.at(0).unwrap());
            ui.add_space(4.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    widgets::fader(ui, ed, t, &block, table.at(5).unwrap(), 150.0);
                    let lv = ed.value(t, &block, table.at(5).unwrap()).unwrap_or(0);
                    ui.label(RichText::new(lv.to_string()).size(11.5));
                });
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    egui::Grid::new(("knobs", part))
                        .spacing(vec2(2.0, 4.0))
                        .show(ui, |ui| {
                            widgets::knob(ui, ed, t, &block, table.at(6).unwrap(), "Pan");
                            widgets::knob(ui, ed, t, &block, table.at(24).unwrap(), "Octave");
                            ui.end_row();
                            widgets::knob(ui, ed, t, &block, table.at(7).unwrap(), "Reverb");
                            widgets::knob(ui, ed, t, &block, table.at(8).unwrap(), "Chorus");
                            ui.end_row();
                            widgets::knob(ui, ed, t, &block, table.at(9).unwrap(), "Delay");
                            ui.end_row();
                        });
                });
            });
            ui.horizontal(|ui| {
                if ui
                    .small_button("Part ›")
                    .on_hover_text("All part settings")
                    .clicked()
                {
                    view.part = part;
                    view.tab = Tab::Part;
                }
                let editable = ed.tone_editable(part);
                if ui
                    .add_enabled(editable, egui::Button::new("Tone ›").small())
                    .on_hover_text("Edit this tone's oscillators, filter, envelopes")
                    .on_disabled_hover_text(if part == 5 {
                        "Part 5's tone can't be edited here"
                    } else {
                        "Only ZEN-Core tones can be edited (not SuperNATURAL or drum kits)"
                    })
                    .clicked()
                {
                    view.tone_part = part;
                    view.tab = Tab::Tone;
                }
            });
        })
    });
}

// ---- Part --------------------------------------------------------------------

pub fn part(ui: &mut Ui, view: &mut View, ed: &mut Editor) {
    if !ed.scene.is_loaded() {
        return not_loaded(ui, ed);
    }
    ui.horizontal(|ui| {
        widgets::segmented(
            ui,
            &mut view.part,
            &[
                (1, "Part 1"),
                (2, "Part 2"),
                (3, "Part 3"),
                (4, "Part 4"),
                (5, "Part 5"),
            ],
        );
        ui.add_space(8.0);
        let tone = ed.tone_label(view.part);
        if ui.button(format!("Tone: {tone}")).clicked() {
            view.tone_picker = Some(view.part);
        }
    });
    ui.add_space(10.0);
    let t = Target::Scene;
    let block = format!("Perf_part_{}", view.part);
    let mut used: HashSet<u32> = [2, 3, 4].into();
    flow(ui, |ui| {
        group(ui, ed, t, &block, "Part", &[1, 0, 5, 6, 10, 11], &mut used);
        group(ui, ed, t, &block, "Effect Sends", &[7, 8, 9], &mut used);
        group(ui, ed, t, &block, "Tuning", &[22, 23, 24], &mut used);
        group(
            ui,
            ed,
            t,
            &block,
            "Sound (offsets to the tone)",
            &[27, 28, 29, 30, 31, 32, 33, 34],
            &mut used,
        );
        group(
            ui,
            ed,
            t,
            &block,
            "Playing",
            &[36, 37, 38, 40, 42, 58, 59],
            &mut used,
        );
        group(
            ui,
            ed,
            t,
            &block,
            "Key & Velocity Ranges",
            &(14..=21).collect::<Vec<_>>(),
            &mut used,
        );
        group(
            ui,
            ed,
            t,
            &block,
            "X-Fade",
            &(89..=93).collect::<Vec<_>>(),
            &mut used,
        );
        let eq = format!("Perf_eq_{}", view.part);
        let mut eq_used = HashSet::new();
        group(
            ui,
            ed,
            t,
            &eq,
            "Part EQ",
            &(0..=8).collect::<Vec<_>>(),
            &mut eq_used,
        );
        group(
            ui,
            ed,
            t,
            &block,
            "Scale Tune",
            &(44..=57).collect::<Vec<_>>(),
            &mut used,
        );
        group(
            ui,
            ed,
            t,
            &block,
            "Receive",
            &(72..=88).collect::<Vec<_>>(),
            &mut used,
        );
        leftovers(ui, ed, t, &block, "More", &used);
    });
}

// ---- Effects -----------------------------------------------------------------

fn fx_block(sel: FxSel) -> (String, &'static str, u32) {
    // (block, type list, offset of the type parameter)
    match sel {
        FxSel::Mfx(n) => (format!("Perf_mfx_{n}"), "mfx", 0),
        FxSel::Ifx => ("Perf_ifx".into(), "mfx", 0),
        FxSel::Chorus => ("Perf_cho".into(), "cho", 1),
        FxSel::Delay => ("Perf_dly".into(), "dly", 1),
        FxSel::Reverb => ("Perf_rev".into(), "rev", 1),
    }
}

fn fx_types(kind: &str) -> &'static [ae30_core::params::FxType] {
    let fx = &data().fx;
    match kind {
        "mfx" => &fx.mfx,
        "cho" => &fx.cho,
        "dly" => &fx.dly,
        _ => &fx.rev,
    }
}

fn fx_name(kind: &str, id: i32) -> String {
    fx_types(kind)
        .iter()
        .find(|f| i32::from(f.id) == id)
        .map_or_else(|| format!("Type {id}"), |f| f.name.clone())
}

pub fn effects(ui: &mut Ui, view: &mut View, ed: &mut Editor) {
    if !ed.scene.is_loaded() {
        return not_loaded(ui, ed);
    }
    let pal = palette(ui);
    let t = Target::Scene;
    let part_t = data().table("Perf_part");
    // Routing overview.
    theme::card(ui).show(ui, |ui| {
        ui.label(theme::section(ui, "Signal flow"));
        egui::Grid::new("routing")
            .spacing(vec2(18.0, 6.0))
            .show(ui, |ui| {
                for h in [
                    "Part", "Tone", "Part MFX", "Then", "Reverb", "Chorus", "Delay",
                ] {
                    ui.label(RichText::new(h).size(11.0).color(pal.weak));
                }
                ui.end_row();
                for n in 1..=5u8 {
                    let block = format!("Perf_part_{n}");
                    let on = ed.value(t, &block, part_t.at(1).unwrap()) == Some(1);
                    let txt = |s: String| {
                        if on {
                            RichText::new(s)
                        } else {
                            RichText::new(s).color(pal.weak)
                        }
                    };
                    ui.label(txt(format!("{n}")));
                    ui.label(txt(ed.tone_label(n)));
                    let mfx = format!("Perf_mfx_{n}");
                    let mtab = data().table("Perf_mfx");
                    let ty = ed.value(t, &mfx, mtab.at(0).unwrap()).unwrap_or(0);
                    let sw = ed.value(t, &mfx, mtab.at(1).unwrap()) == Some(1);
                    let follow = ed.value(t, &block, part_t.at(10).unwrap()) == Some(1);
                    let label = if follow {
                        "Tone's own".to_string()
                    } else if sw {
                        fx_name("mfx", ty)
                    } else {
                        "Off".into()
                    };
                    if ui.link(txt(label)).clicked() {
                        view.fx = FxSel::Mfx(n);
                    }
                    let out = ed.value(t, &block, part_t.at(11).unwrap()) == Some(1);
                    if ui
                        .link(txt(if out {
                            "→ IFX".into()
                        } else {
                            "→ Dry".into()
                        }))
                        .clicked()
                        && out
                    {
                        view.fx = FxSel::Ifx;
                    }
                    for off in [7, 8, 9] {
                        let v = ed.value(t, &block, part_t.at(off).unwrap()).unwrap_or(0);
                        ui.label(txt(v.to_string()));
                    }
                    ui.end_row();
                }
            });
    });
    ui.add_space(12.0);
    ui.horizontal_top(|ui| {
        // Effect list.
        theme::card(ui).show(ui, |ui| ui.vertical(|ui| {
            ui.set_width(150.0);
            ui.label(theme::section(ui, "Effects"));
            for n in 1..=5u8 {
                ui.selectable_value(&mut view.fx, FxSel::Mfx(n), format!("Part {n} MFX"));
            }
            ui.separator();
            ui.selectable_value(&mut view.fx, FxSel::Ifx, "Insert (IFX)");
            ui.selectable_value(&mut view.fx, FxSel::Chorus, "Chorus");
            ui.selectable_value(&mut view.fx, FxSel::Delay, "Delay");
            ui.selectable_value(&mut view.fx, FxSel::Reverb, "Reverb");
        }));
        let (block, kind, type_off) = fx_block(view.fx);
        let Some(table) = ed.scene.block(&block).map(|b| b.table) else {
            return;
        };
        let type_p = table.at(type_off).unwrap();
        let ty = ed.value(t, &block, type_p).unwrap_or(0);
        theme::card(ui).show(ui, |ui| ui.vertical(|ui| {
            ui.set_width(460.0);
            ui.horizontal(|ui| {
                ui.label(theme::title(fx_name(kind, ty), 17.0));
                ui.add_space(10.0);
                let mut sel = ty;
                egui::ComboBox::from_id_salt(("fxtype", &block))
                    .selected_text("Change type")
                    .height(400.0)
                    .show_ui(ui, |ui| {
                        for f in fx_types(kind) {
                            ui.selectable_value(&mut sel, i32::from(f.id), &f.name);
                        }
                    });
                if sel != ty {
                    ed.set(t, &block, type_p, sel);
                }
            });
            if let FxSel::Mfx(n) = view.fx {
                let follow = ed.value(t, &format!("Perf_part_{n}"), part_t.at(10).unwrap()) == Some(1);
                if follow {
                    ui.label(
                        RichText::new("This part uses its tone's own effect (Follow Tone MFX is on), so these settings don't apply.")
                            .color(pal.amber),
                    );
                }
            }
            ui.add_space(6.0);
            let fixed: Vec<&'static Param> = table
                .fixed()
                .filter(|p| p.off != type_off && !p.label().contains("Ctrl"))
                .collect();
            widgets::grid(ui, ed, t, &block, fixed);
            ui.add_space(8.0);
            ui.label(theme::section(ui, "Parameters"));
            let typed: Vec<&'static Param> = table.for_fx(ty as u16);
            widgets::grid(ui, ed, t, &block, typed);
            let ctrl: Vec<&'static Param> =
                table.fixed().filter(|p| p.label().contains("Ctrl")).collect();
            if !ctrl.is_empty() {
                ui.add_space(6.0);
                egui::CollapsingHeader::new("Control sources")
                    .id_salt(("fxctrl", &block))
                    .show(ui, |ui| widgets::grid(ui, ed, t, &block, ctrl));
            }
        }));
    });
}

// ---- Controllers ---------------------------------------------------------------

pub const CONTROLLERS: [(&str, &str); 18] = [
    ("breath_1", "Breath"),
    ("bite_1", "Bite Down"),
    ("bite_2", "Bite Up"),
    ("sw_1", "S1 Button"),
    ("sw_2", "S2 Button"),
    ("key_1", "Side Key X"),
    ("key_2", "Side Key C1"),
    ("key_3", "Side Key C2"),
    ("key_4", "Side Key C3"),
    ("key_5", "Side Key C4"),
    ("key_6", "Side Key C5"),
    ("key_7", "Side Key Tc"),
    ("key_8", "Side Key Ta"),
    ("wheel_1", "Wheel Down"),
    ("wheel_2", "Wheel Up"),
    ("pad_1", "Thumb Pad"),
    ("motion_1", "Motion"),
    ("ctrl", "MIDI Control Mode"),
];

pub fn controllers(ui: &mut Ui, view: &mut View, ed: &mut Editor) {
    if !ed.scene.is_loaded() {
        return not_loaded(ui, ed);
    }
    let pal = palette(ui);
    let t = Target::Scene;
    ui.horizontal(|ui| {
        widgets::segmented(
            ui,
            &mut view.ctl_midi,
            &[(false, "Sound (internal)"), (true, "MIDI Out")],
        );
        ui.label(
            RichText::new(if view.ctl_midi {
                "What each controller sends to external MIDI gear (MIDI Control mode)."
            } else {
                "What each controller does to the AE-30's own sound."
            })
            .color(pal.weak),
        );
    });
    ui.add_space(10.0);
    ui.horizontal_top(|ui| {
        theme::card(ui).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                for (i, (key, name)) in CONTROLLERS.iter().enumerate() {
                    if *key == "ctrl" && !view.ctl_midi {
                        continue;
                    }
                    let used = slots_in_use(ed, &assign_block(key, view.ctl_midi));
                    let text = if used > 0 {
                        RichText::new(*name)
                    } else {
                        RichText::new(*name).color(pal.weak)
                    };
                    ui.selectable_value(&mut view.ctl, i, text);
                }
            })
        });
        let (key, name) = CONTROLLERS[view.ctl.min(CONTROLLERS.len() - 1)];
        ui.vertical(|ui| {
            ui.label(theme::title(name, 17.0));
            ui.add_space(4.0);
            if key == "ctrl" {
                flow(ui, |ui| {
                    for n in 1..=2 {
                        let block = format!("Perf_midi_ctrl_{n}");
                        let used = HashSet::new();
                        leftovers(ui, ed, t, &block, &format!("MIDI Control {n}"), &used);
                    }
                });
                return;
            }
            let block = assign_block(key, view.ctl_midi);
            let Some(table) = ed.scene.block(&block).map(|b| b.table) else {
                return;
            };
            let slots = (table.size / 16) as usize;
            let live =
                (key == "breath_1" && ed.breath.1.elapsed().as_secs() < 2).then_some(ed.breath.0);
            flow(ui, |ui| {
                for k in 0..slots {
                    let o = (k * 16) as u32;
                    let ps = params_at(table, &[o, o + 2, o + 3, o + 4, o + 5, o + 6, o + 7]);
                    if ps.len() < 7 {
                        continue;
                    }
                    let vals: Vec<i32> = ps
                        .iter()
                        .map(|p| ed.value(t, &block, p).unwrap_or(0))
                        .collect();
                    let off = vals[0] == 0;
                    card(ui, &format!("Assign {}", k + 1), CARD_W, |ui| {
                        if off {
                            ui.label(RichText::new("Not assigned").color(pal.weak));
                        } else {
                            widgets::curve_graph(
                                ui, vals[1], vals[2], vals[3], vals[4], vals[6], live,
                            );
                        }
                        widgets::grid(ui, ed, t, &block, ps);
                    });
                }
            });
        });
    });
}

fn assign_block(key: &str, midi: bool) -> String {
    format!("Perf_{}_{key}", if midi { "midi" } else { "as" })
}

fn slots_in_use(ed: &Editor, block: &str) -> usize {
    let Some(b) = ed.scene.block(block) else {
        return 0;
    };
    (0..b.data.len() / 16)
        .filter(|k| b.raw((k * 16) as u32).unwrap_or(0) != 0)
        .count()
}

// ---- Tone ------------------------------------------------------------------------

pub fn tone(ui: &mut Ui, view: &mut View, ed: &mut Editor) {
    let pal = palette(ui);
    if !ed.scene.is_loaded() {
        return not_loaded(ui, ed);
    }
    ui.horizontal(|ui| {
        let opts: Vec<(u8, String)> = (1..=4).map(|p| (p, format!("Part {p}"))).collect();
        let opts: Vec<(u8, &str)> = opts.iter().map(|(p, s)| (*p, s.as_str())).collect();
        widgets::segmented(ui, &mut view.tone_part, &opts);
    });
    let part = view.tone_part;
    ui.add_space(8.0);
    if !ed.tone_editable(part) {
        theme::card(ui).show(ui, |ui| {
            ui.label(theme::title(ed.tone_label(part), 17.0));
            ui.label(
                "This part plays a SuperNATURAL Acoustic tone or a drum kit. Roland doesn't publish \
                 their insides, so only ZEN-Core tones (Preset A–C and User tones) can be edited.",
            );
            if ui.button("Choose a ZEN-Core tone…").clicked() {
                view.tone_picker = Some(part);
            }
        });
        return;
    }
    let t = Target::Tone(part);
    if !ed.image(t).is_loaded() {
        return not_loaded(ui, ed);
    }
    // Header: name, save.
    theme::card(ui).show(ui, |ui| {
        ui.horizontal(|ui| {
            let mut name = ed.image(t).scene_name().unwrap_or_default();
            let r = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .char_limit(16)
                    .font(egui::FontId::new(18.0, theme::semibold()))
                    .desired_width(220.0),
            );
            if r.changed() {
                ed.rename_tone(part, &name);
            }
            ui.label(RichText::new(ed.tone_label(part)).color(pal.weak));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(theme::primary(ui, "Save Tone…"))
                    .on_hover_text("Store this tone in a User Tone slot so scenes can use it")
                    .clicked()
                {
                    if ed.user_tones.is_none() {
                        ed.load_user_tones();
                    }
                    view.save_tone = Some(super::SaveTone {
                        part,
                        slot: 0,
                        name: ed.image(t).scene_name().unwrap_or_default(),
                    });
                }
                if ui.button("Change tone…").clicked() {
                    view.tone_picker = Some(part);
                }
            });
        });
    });
    ui.add_space(10.0);

    // Partial selector with on/off.
    let pmt = data().table("PCMEX_pmt");
    ui.horizontal(|ui| {
        ui.label(theme::section(ui, "Partials"));
        for k in 1..=4u8 {
            let sw = pmt
                .params
                .iter()
                .filter(|p| p.name == "Partial Switch")
                .nth(usize::from(k - 1));
            ui.vertical(|ui| {
                let selected = view.partial == k;
                if ui
                    .selectable_label(selected, format!("Partial {k}"))
                    .clicked()
                {
                    view.partial = k;
                }
                if let Some(sw) = sw {
                    widgets::control(ui, ed, t, "PCMEX_pmt", sw);
                }
            });
        }
    });
    ui.add_space(10.0);
    let k = view.partial;
    let ptl = format!("PCMEX_ptl_{k}");
    let mut used = HashSet::new();
    flow(ui, |ui| {
        // Oscillator.
        let osc_type = ed
            .image(t)
            .block(&format!("PCMEX_synPtl_{k}"))
            .and_then(|b| b.get(0))
            .unwrap_or(0);
        card(ui, "Oscillator", CARD_W, |ui| {
            let syn = format!("PCMEX_synPtl_{k}");
            let st = data().table("PCMEX_synPtl");
            widgets::grid(ui, ed, t, &syn, params_at(st, &[0, 1, 6, 7, 8, 9, 17, 22]));
            if osc_type == 0 || osc_type == 2 {
                if let Some(w) = wave_name(ed, t, &ptl) {
                    ui.label(RichText::new(format!("Wave: {w}")).color(pal.accent));
                }
                let pt = data().table("PCMEX_ptl");
                widgets::grid(
                    ui,
                    ed,
                    t,
                    &ptl,
                    params_at(pt, &[27, 28, 32, 36, 40, 41, 42, 43, 127]),
                );
            }
        });
        used.extend([27, 28, 32, 36, 40, 41, 42, 43, 127]);
        // Pitch.
        card(ui, "Pitch", CARD_W, |ui| {
            let pt = data().table("PCMEX_ptl");
            widgets::grid(ui, ed, t, &ptl, params_at(pt, &[1, 2, 3, 44]));
            ui.add_space(4.0);
            let pe = format!("PCMEX_ptlPenv_{k}");
            widgets::envelope(ui, ed, t, &pe, Env::Pitch);
            egui::CollapsingHeader::new("Pitch envelope")
                .id_salt(("penv", k))
                .show(ui, |ui| {
                    let tb = data().table("PCMEX_ptlPenv");
                    widgets::grid(ui, ed, t, &pe, tb.fixed().collect::<Vec<_>>());
                });
        });
        used.extend([1, 2, 3, 44]);
        // Filter.
        card(ui, "Filter", CARD_W, |ui| {
            let pt = data().table("PCMEX_ptl");
            let offs: Vec<u32> = (49..=70).collect();
            widgets::grid(ui, ed, t, &ptl, params_at(pt, &offs));
            let syn = format!("PCMEX_synPtl_{k}");
            let st = data().table("PCMEX_synPtl");
            widgets::grid(ui, ed, t, &syn, params_at(st, &[10, 14, 15, 18, 21]));
            ui.add_space(4.0);
            let fe = format!("PCMEX_ptlFenv_{k}");
            widgets::envelope(ui, ed, t, &fe, Env::Filter);
            egui::CollapsingHeader::new("Filter envelope")
                .id_salt(("fenv", k))
                .show(ui, |ui| {
                    let tb = data().table("PCMEX_ptlFenv");
                    widgets::grid(ui, ed, t, &fe, tb.fixed().collect::<Vec<_>>());
                });
        });
        used.extend(49..=70);
        // Amp.
        card(ui, "Amp", CARD_W, |ui| {
            let pt = data().table("PCMEX_ptl");
            widgets::grid(
                ui,
                ed,
                t,
                &ptl,
                params_at(pt, &[0, 7, 8, 10, 11, 71, 72, 123, 124]),
            );
            ui.add_space(4.0);
            let ae = format!("PCMEX_ptlAenv_{k}");
            widgets::envelope(ui, ed, t, &ae, Env::Amp);
            egui::CollapsingHeader::new("Amp envelope")
                .id_salt(("aenv", k))
                .show(ui, |ui| {
                    let tb = data().table("PCMEX_ptlAenv");
                    widgets::grid(ui, ed, t, &ae, tb.fixed().collect::<Vec<_>>());
                });
        });
        used.extend([0, 7, 8, 10, 11, 71, 72, 123, 124]);
        // LFOs.
        let lfo = format!("PCMEX_ptlLfo_{k}");
        let lt = data().table("PCMEX_ptlLfo");
        for (n, base) in [(1, 0u32), (2, 79)] {
            card(ui, &format!("LFO {n}"), CARD_W, |ui| {
                let offs: Vec<u32> = [0, 1, 2, 4, 8, 10, 11, 15, 17, 18, 22, 23, 25, 27, 29, 30, 3]
                    .iter()
                    .map(|o| o + base)
                    .collect();
                widgets::grid(ui, ed, t, &lfo, params_at(lt, &offs));
                egui::CollapsingHeader::new("Step sequence")
                    .id_salt(("steps", k, n))
                    .show(ui, |ui| {
                        let steps: Vec<u32> = (0..16).map(|i| base + 31 + 2 * i).collect();
                        widgets::grid(ui, ed, t, &lfo, params_at(lt, &steps));
                    });
            });
        }
        // Matrix control.
        card(ui, "Matrix Control", CARD_W, |ui| {
            let pt = data().table("PCMEX_ptl");
            for m in 0..4u32 {
                let b = 86 + 9 * m;
                egui::CollapsingHeader::new(format!("Matrix {}", m + 1))
                    .id_salt(("mtx", k, m))
                    .default_open(m == 0)
                    .show(ui, |ui| {
                        widgets::grid(
                            ui,
                            ed,
                            t,
                            &ptl,
                            params_at(pt, &(b..b + 9).collect::<Vec<_>>()),
                        );
                    });
            }
        });
        used.extend(86..=121);
        // Partial EQ and ranges.
        let eq = format!("PCMEX_ptlEq_{k}");
        let eq_used = HashSet::new();
        leftovers(ui, ed, t, &eq, "Partial EQ", &eq_used);
        let base = 2 + 9 * u32::from(k - 1);
        card(ui, "Key & Velocity Range", CARD_W, |ui| {
            widgets::grid(
                ui,
                ed,
                t,
                "PCMEX_pmt",
                params_at(pmt, &(base + 1..base + 9).collect::<Vec<_>>()),
            );
        });
        leftovers(ui, ed, t, &ptl, "More (partial)", &used);
    });
    ui.add_space(12.0);
    ui.label(theme::section(ui, "Whole tone"));
    ui.add_space(4.0);
    flow(ui, |ui| {
        let used = HashSet::new();
        leftovers(ui, ed, t, "PCMEX_common", "Common", &used);
        let u2 = HashSet::new();
        leftovers(ui, ed, t, "PCMEX_synCom", "Unison & Drift", &u2);
        let u3 = HashSet::new();
        leftovers(
            ui,
            ed,
            t,
            "PCMEX_synPmt",
            "Structure (Ring / Cross Mod)",
            &u3,
        );
        let mut u4 = HashSet::new();
        leftovers(ui, ed, t, "PCMEX_pmt", "Partial Mix", &{
            u4.extend((0..4u32).flat_map(|i| 2 + 9 * i..11 + 9 * i));
            u4
        });
        card(ui, "Tone Effect (MFX)", CARD_W, |ui| {
            let mt = data().table("PCMEX_mfx");
            let ty = ed
                .image(t)
                .block("PCMEX_mfx")
                .and_then(|b| b.get(0))
                .unwrap_or(0);
            ui.label(theme::title(fx_name("mfx", ty), 14.0));
            let mut sel = ty;
            egui::ComboBox::from_id_salt(("tonefx", part))
                .selected_text("Change type")
                .height(400.0)
                .show_ui(ui, |ui| {
                    for f in fx_types("mfx") {
                        ui.selectable_value(&mut sel, i32::from(f.id), &f.name);
                    }
                });
            if sel != ty {
                ed.set(t, "PCMEX_mfx", mt.at(0).unwrap(), sel);
            }
            let ps: Vec<&'static Param> = mt
                .fixed()
                .filter(|p| p.off != 0 && !p.label().contains("Ctrl"))
                .chain(mt.for_fx(ty as u16))
                .collect();
            widgets::grid(ui, ed, t, "PCMEX_mfx", ps);
        });
    });
}

/// The name of the PCM wave a partial plays, from Roland's wave list.
fn wave_name(ed: &Editor, t: Target, block: &str) -> Option<String> {
    let b = ed.image(t).block(block)?;
    let gtype = b.get(27)?;
    let gid = b.get(28)? as u32;
    let num = b.get(32)?;
    if num == 0 {
        return Some("Off".into());
    }
    if gtype != 0 {
        return Some(format!("#{num}"));
    }
    let d = data();
    let bank = d
        .wave_groups
        .iter()
        .find(|(_, &g)| g == gid)
        .map(|(k, _)| k.as_str())?;
    d.waves
        .get(bank)?
        .get(num as usize - 1)
        .map(|n| format!("{n} ({bank}{num})"))
}

// ---- System ------------------------------------------------------------------------

pub fn system(ui: &mut Ui, ed: &mut Editor) {
    let pal = palette(ui);
    if !ed.system.block("Sys_common").is_some_and(|b| b.is_loaded()) {
        return not_loaded(ui, ed);
    }
    let t = Target::System;
    theme::card(ui).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    "System settings change at once but are only kept after a restart once stored.",
                )
                .color(pal.weak),
            );
            if ui
                .add(theme::primary(ui, "Store System Settings"))
                .clicked()
            {
                ed.session.write_system();
                ed.tell("System settings stored");
            }
        });
    });
    ui.add_space(10.0);
    let table = data().table("Sys_common");
    let pick = |f: &dyn Fn(&str) -> bool| -> Vec<u32> {
        table
            .fixed()
            .filter(|p| f(p.label()))
            .map(|p| p.off)
            .collect()
    };
    let mut used = HashSet::new();
    flow(ui, |ui| {
        let general = pick(&|l| {
            [
                "Display",
                "Auto Off",
                "Language",
                "Edit Confirm",
                "Shortcut",
                "Category Knob",
            ]
            .iter()
            .any(|k| l.contains(k))
        });
        group(ui, ed, t, "Sys_common", "General", &general, &mut used);
        let tuning =
            pick(&|l| l.contains("Tune") || l.contains("Transpose") || l.contains("Key Shift"));
        group(ui, ed, t, "Sys_common", "Tuning", &tuning, &mut used);
        let out = pick(&|l| l.contains("Output") || l.contains("Speaker") || l.contains("MIDI"));
        group(ui, ed, t, "Sys_common", "Output & MIDI", &out, &mut used);
        let play = pick(&|l| {
            [
                "Breath",
                "Bite",
                "Fingering",
                "Key Delay",
                "Octave Key",
                "Thumb",
                "Hold",
                "Bend",
            ]
            .iter()
            .any(|k| l.contains(k))
        });
        group(ui, ed, t, "Sys_common", "Playing", &play, &mut used);
        let keys = pick(&|l| l.starts_with("Key Function"));
        group(ui, ed, t, "Sys_common", "Key Functions", &keys, &mut used);
        let src = pick(&|l| l.contains("Source") || l.contains("Harmony") || l.contains("Motion"));
        group(
            ui,
            ed,
            t,
            "Sys_common",
            "Sources, Harmony & Motion",
            &src,
            &mut used,
        );
        leftovers(ui, ed, t, "Sys_common", "More", &used);
    });
}
