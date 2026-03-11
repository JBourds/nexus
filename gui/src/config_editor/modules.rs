use std::path::Path;

use config::parse::NodeProfile;
use egui::Ui;

use crate::state::ModuleState;

/// Small red "X" remove button with tooltip. Returns `true` when clicked.
fn module_remove_button(ui: &mut Ui) -> bool {
    let button = egui::Button::new(
        egui::RichText::new("\u{2716}")
            .color(egui::Color32::from_rgb(220, 60, 60))
            .size(13.0),
    )
    .small()
    .frame(false);
    ui.add(button)
        .on_hover_text("Remove this module from the config")
        .clicked()
}

/// Show the "Modules" section in the config editor sidebar.
pub fn show_modules(ui: &mut Ui, modules: &mut ModuleState, config_dir: Option<&Path>) {
    // Current use list
    if modules.use_list.is_empty() {
        ui.weak("No modules imported");
    } else {
        ui.label("Imported:");
        let mut removed = None;
        for (i, spec) in modules.use_list.iter().enumerate() {
            ui.horizontal(|ui| {
                if module_remove_button(ui) {
                    removed = Some(i);
                }
                ui.label(spec);
            });
        }
        if let Some(i) = removed {
            modules.use_list.remove(i);
            modules.resolve_profiles(config_dir);
        }
    }

    // Add module -- combo box populated from stdlib catalog
    let add_id = egui::Id::new("module_add_combo");
    let mut add_buf: String = ui.data(|d| d.get_temp(add_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label("+");
        egui::ComboBox::from_id_salt("module_add_selector")
            .selected_text(if add_buf.is_empty() {
                "Select module..."
            } else {
                &add_buf
            })
            .show_ui(ui, |ui| {
                for entry in &modules.stdlib_catalog {
                    if modules.use_list.contains(&entry.spec) {
                        continue;
                    }
                    let label = if entry.description.is_empty() {
                        entry.spec.clone()
                    } else {
                        format!("{} -- {}", entry.spec, entry.description)
                    };
                    if ui
                        .selectable_value(&mut add_buf, entry.spec.clone(), label)
                        .clicked()
                    {}
                }
            });
        if ui.button("Add").clicked() && !add_buf.is_empty() {
            if !modules.use_list.contains(&add_buf) {
                modules.use_list.push(add_buf.clone());
                modules.resolve_profiles(config_dir);
            }
            add_buf.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(add_id, add_buf));

    if ui.button("Browse Modules...").clicked() {
        modules.browser_open = true;
    }

    // Show resolved profiles summary
    if !modules.available_profiles.is_empty() {
        ui.add_space(4.0);
        ui.label("Available profiles:");
        let mut profile_names: Vec<_> = modules.available_profiles.keys().cloned().collect();
        profile_names.sort();
        for name in &profile_names {
            if let Some(resolved) = modules.available_profiles.get(name) {
                ui.horizontal(|ui| {
                    ui.label(format!("  {name}"));
                    ui.weak(format!("({})", resolved.source_module));
                });
            }
        }
    }
}

/// Simple fuzzy match: all query characters must appear in the haystack in order,
/// case-insensitive.
fn fuzzy_matches(query: &str, haystack: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let haystack_lower = haystack.to_ascii_lowercase();
    let query_lower = query.to_ascii_lowercase();
    let mut haystack_chars = haystack_lower.chars();
    for qc in query_lower.chars() {
        if !haystack_chars.any(|hc| hc == qc) {
            return false;
        }
    }
    true
}

/// Build a sorted list of unique categories from the stdlib catalog.
fn categories(catalog: &[crate::state::StdlibEntry]) -> Vec<String> {
    let mut cats: Vec<String> = catalog
        .iter()
        .filter_map(|e| e.spec.split('/').next().map(String::from))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    cats.sort();
    cats
}

/// Show the module browser window with vertical layout, tree hierarchy,
/// and fuzzy-find search.
pub fn show_module_browser(
    ctx: &egui::Context,
    modules: &mut ModuleState,
    config_dir: Option<&Path>,
) {
    if !modules.browser_open {
        return;
    }

    let mut open = modules.browser_open;
    egui::Window::new("Module Browser")
        .open(&mut open)
        .default_width(420.0)
        .default_height(500.0)
        .resizable(true)
        .show(ctx, |ui| {
            // Search bar
            ui.horizontal(|ui| {
                ui.label("Search:");
                let resp = ui.text_edit_singleline(&mut modules.browser_search);
                if resp.changed() && !modules.browser_search.is_empty() {
                    // Auto-expand all categories that have matches
                    modules.browser_expanded.clear();
                    for cat in categories(&modules.stdlib_catalog) {
                        let has_match = modules.stdlib_catalog.iter().any(|e| {
                            e.spec.starts_with(&format!("{cat}/"))
                                && fuzzy_matches(&modules.browser_search, &e.spec)
                        });
                        if has_match {
                            modules.browser_expanded.insert(cat);
                        }
                    }
                }
            });

            ui.separator();

            // Module tree + details in a single vertical scroll area
            egui::ScrollArea::vertical()
                .id_salt("browser_scroll")
                .show(ui, |ui| {
                    // Group by category
                    let cats = categories(&modules.stdlib_catalog);

                    for cat in &cats {
                        // Collect matching entries as owned data to avoid
                        // borrowing modules.stdlib_catalog across the mutable
                        // closure below.
                        let cat_entries: Vec<(String, String)> = modules
                            .stdlib_catalog
                            .iter()
                            .filter(|e| {
                                e.spec.starts_with(&format!("{cat}/"))
                                    && fuzzy_matches(&modules.browser_search, &e.spec)
                            })
                            .map(|e| {
                                let short = e.spec.split('/').next_back().unwrap_or(&e.spec).to_string();
                                (e.spec.clone(), short)
                            })
                            .collect();

                        if cat_entries.is_empty() {
                            continue;
                        }

                        // Collect descriptions separately (also owned).
                        let descriptions: Vec<String> = modules
                            .stdlib_catalog
                            .iter()
                            .filter(|e| {
                                e.spec.starts_with(&format!("{cat}/"))
                                    && fuzzy_matches(&modules.browser_search, &e.spec)
                            })
                            .map(|e| e.description.clone())
                            .collect();

                        // Category collapsing header
                        let expanded = modules.browser_expanded.contains(cat);
                        let cat_id = ui.make_persistent_id(format!("browser_cat_{cat}"));
                        let mut collapsing =
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                cat_id,
                                expanded,
                            );
                        if expanded {
                            collapsing.set_open(true);
                        }

                        collapsing
                            .show_header(ui, |ui| {
                                let label = format!("{cat} ({})", cat_entries.len());
                                if ui.strong(label).clicked() {
                                    if modules.browser_expanded.contains(cat) {
                                        modules.browser_expanded.remove(cat);
                                    } else {
                                        modules.browser_expanded.insert(cat.clone());
                                    }
                                }
                            })
                            .body(|ui| {
                                for (i, (spec, short_name)) in cat_entries.iter().enumerate() {
                                    let already_imported = modules.use_list.contains(spec);

                                    let resp = if already_imported {
                                        // Draw with green border to indicate imported
                                        let green = egui::Color32::from_rgb(60, 160, 60);
                                        let (rect, resp) = ui.allocate_exact_size(
                                            egui::vec2(ui.available_width(), 20.0),
                                            egui::Sense::click(),
                                        );
                                        if ui.is_rect_visible(rect) {
                                            if resp.hovered() {
                                                ui.painter().rect_filled(
                                                    rect,
                                                    2.0,
                                                    ui.visuals().widgets.hovered.bg_fill,
                                                );
                                            }
                                            // Green border
                                            ui.painter().rect_stroke(
                                                rect.shrink(1.0),
                                                2.0,
                                                egui::Stroke::new(1.5, green),
                                                egui::StrokeKind::Inside,
                                            );
                                            ui.painter().text(
                                                rect.left_center() + egui::vec2(6.0, 0.0),
                                                egui::Align2::LEFT_CENTER,
                                                short_name,
                                                egui::FontId::default(),
                                                green,
                                            );
                                        }
                                        resp
                                    } else {
                                        ui.selectable_label(false, short_name)
                                    };

                                    if resp.clicked() {
                                        if already_imported {
                                            modules.use_list.retain(|s| s != spec);
                                        } else {
                                            modules.use_list.push(spec.clone());
                                        }
                                        modules.resolve_profiles(config_dir);
                                        modules.browser_selected = Some(spec.clone());
                                    }
                                    let desc = &descriptions[i];
                                    if !desc.is_empty() {
                                        resp.on_hover_text(desc);
                                    }
                                }
                            });
                    }

                    // Details section for selected module
                    if let Some(ref selected) = modules.browser_selected.clone()
                        && let Some(entry) =
                            modules.stdlib_catalog.iter().find(|e| &e.spec == selected)
                        {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);

                            ui.heading(selected);
                            if !entry.description.is_empty() {
                                ui.label(&entry.description);
                            }
                            ui.add_space(4.0);

                            // Provides summary
                            if !entry.provides.links.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.strong("Links:");
                                    ui.label(entry.provides.links.join(", "));
                                });
                            }
                            if !entry.provides.channels.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.strong("Channels:");
                                    ui.label(entry.provides.channels.join(", "));
                                });
                            }
                            if !entry.provides.profiles.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.strong("Profiles:");
                                    ui.label(entry.provides.profiles.join(", "));
                                });
                            }

                            // Profile details
                            if !entry.provides.profiles.is_empty()
                                && let Ok(path) =
                                    config::module::resolve_module_path(selected, None)
                                    && let Ok(module) = config::parse_module_file(&path)
                                        && let Some(profiles) = &module.profiles {
                                            for (pname, profile) in profiles {
                                                ui.add_space(4.0);
                                                ui.strong(format!("Profile: {pname}"));
                                                show_profile_preview(ui, profile);
                                            }
                                        }

                            ui.add_space(4.0);
                            let already = modules.use_list.contains(selected);
                            ui.weak(if already {
                                "Click in the list above to remove"
                            } else {
                                "Click in the list above to import"
                            });
                        }
                });
        });
    modules.browser_open = open;
}

/// Read-only preview of a profile's contents.
pub fn show_profile_preview(ui: &mut Ui, profile: &NodeProfile) {
    ui.indent("profile_preview", |ui| {
        if let Some(ref res) = profile.resources {
            let mut parts = Vec::new();
            if let Some(rate) = res.clock_rate {
                let unit = res
                    .clock_units
                    .as_ref()
                    .map(|u| u.0.as_str())
                    .unwrap_or("Hz");
                parts.push(format!("{rate} {unit}"));
            }
            if let Some(cores) = res.cores {
                parts.push(format!("{cores} cores"));
            }
            if let Some(ram) = res.ram {
                let unit = res.ram_units.as_ref().map(|u| u.0.as_str()).unwrap_or("B");
                parts.push(format!("{ram} {unit} RAM"));
            }
            if !parts.is_empty() {
                ui.label(format!("Resources: {}", parts.join(", ")));
            }
        }
        if let Some(ref states) = profile.power_states {
            let names: Vec<_> = states.keys().collect();
            if !names.is_empty() {
                ui.label(format!("Power states: {}", join_sorted(&names)));
            }
        }
        if let Some(ref sources) = profile.power_sources {
            let names: Vec<_> = sources.keys().collect();
            if !names.is_empty() {
                ui.label(format!("Sources: {}", join_sorted(&names)));
            }
        }
        if let Some(ref sinks) = profile.power_sinks {
            let names: Vec<_> = sinks.keys().collect();
            if !names.is_empty() {
                ui.label(format!("Sinks: {}", join_sorted(&names)));
            }
        }
        if let Some(ref ce) = profile.channel_energy {
            let names: Vec<_> = ce.keys().collect();
            if !names.is_empty() {
                ui.label(format!("Channel energy: {}", join_sorted(&names)));
            }
        }
    });
}

fn join_sorted(items: &[&String]) -> String {
    let mut sorted: Vec<_> = items.iter().map(|s| s.as_str()).collect();
    sorted.sort();
    sorted.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matches_basic() {
        assert!(fuzzy_matches("", "anything"));
        assert!(fuzzy_matches("esp", "boards/esp32_devkit"));
        assert!(fuzzy_matches("esp32", "boards/esp32_devkit"));
        assert!(fuzzy_matches("brd", "boards/esp32_devkit"));
        assert!(fuzzy_matches("sx", "lora/sx1276_915mhz"));
        assert!(fuzzy_matches("915", "lora/sx1276_915mhz"));
    }

    #[test]
    fn fuzzy_matches_case_insensitive() {
        assert!(fuzzy_matches("ESP", "boards/esp32_devkit"));
        assert!(fuzzy_matches("Esp32", "boards/esp32_devkit"));
    }

    #[test]
    fn fuzzy_matches_rejects_non_matching() {
        assert!(!fuzzy_matches("xyz", "boards/esp32_devkit"));
        assert!(!fuzzy_matches("zzz", "lora/sx1276"));
        // Out-of-order characters should not match
        assert!(!fuzzy_matches("tse", "boards/esp32_devkit"));
    }
}
